//! Root ViewModel: owns shared state, wires Slint callbacks, loads initial data.
//!
//! # Threading
//!
//! Slint components and models are `!Send`, so they may only be touched on the
//! UI thread. Domain work runs on the Tokio runtime and results come back
//! through [`slint::Weak::upgrade_in_event_loop`].
//!
//! To make that possible, all state shared between the two sides is plain data
//! behind `Arc<Mutex<_>>` (identifiers and domain trees, never Slint types).
//! Conversion to UI types happens inside the event-loop closure.
//!
//! See `docs/ja/develop/design/ui/viewmodel-architecture.md`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use flequit_core::InfrastructureRepositoriesTrait;
use flequit_core::facades::{initialization_facades, subtask_facades, task_facades};
use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::subtask::PartialSubTask;
use flequit_model::models::task_projects::task::{PartialTask, Task};
use flequit_model::types::id_types::{ProjectId, SubTaskId, TaskId, TaskListId, UserId};
use flequit_model::types::task_types::TaskStatus as DomainStatus;
use flequit_platform::{Capability, Platform};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};
use tokio::runtime::Handle;

use crate::adapters::datetime::DisplayTimezone;
use crate::adapters::{to_project_item, to_task_item};
use crate::bindings::{
    Actions, AppState, AppWindow, Capabilities as UiCapabilities, DueFilterItem, I18n, Pane,
    ProjectItem, TaskItem,
};
use crate::viewmodels::TaskListUiViewModel;

/// State shared between the UI thread and background tasks.
///
/// Deliberately contains no Slint types so it can cross the thread boundary.
#[derive(Debug, Default)]
struct SharedState {
    /// Last loaded snapshot of the project hierarchy.
    ///
    /// Kept so that changing the selected list can re-derive the visible tasks
    /// without another round trip to storage.
    trees: Vec<ProjectTree>,
    /// Which projects are expanded in the sidebar.
    expanded_projects: HashSet<String>,
    /// Which task rows are expanded.
    task_ui: TaskListUiViewModel,
    /// Currently selected project; empty means "all projects".
    selected_project_id: String,
    /// Currently selected list; empty means "no list selected".
    selected_list_id: String,
    /// Author recorded on every write. Resolved from the current account.
    current_user: Option<UserId>,
}

impl SharedState {
    /// Locates the project a task belongs to, using the cached tree.
    fn project_of_task(&self, task_id: &str) -> Option<ProjectId> {
        self.trees.iter().find_map(|tree| {
            tree.task_lists
                .iter()
                .flat_map(|list| list.tasks.iter())
                .any(|task| task.id.as_str() == task_id)
                .then_some(tree.id)
        })
    }

    /// Locates the project a subtask belongs to.
    fn project_of_subtask(&self, subtask_id: &str) -> Option<ProjectId> {
        self.trees.iter().find_map(|tree| {
            tree.task_lists
                .iter()
                .flat_map(|list| list.tasks.iter())
                .flat_map(|task| task.sub_tasks.iter())
                .any(|sub| sub.id.as_str() == subtask_id)
                .then_some(tree.id)
        })
    }

    /// The project that owns `list_id`.
    fn project_of_list(&self, list_id: &str) -> Option<ProjectId> {
        self.trees.iter().find_map(|tree| {
            tree.task_lists
                .iter()
                .any(|list| list.id.as_str() == list_id)
                .then_some(tree.id)
        })
    }
}

/// The fields an optimistic update may change, captured before the write.
///
/// [`TaskItem`] holds `ModelRc` values and is therefore `!Send`, so it cannot be
/// carried into a background task. Rollback only ever restores these scalars, so
/// the snapshot stores them directly.
#[derive(Debug, Clone)]
struct TaskRowSnapshot {
    id: String,
    title: String,
    notes: String,
    completed: bool,
    status: crate::bindings::TaskStatus,
    overdue: bool,
}

impl TaskRowSnapshot {
    fn capture(item: &TaskItem) -> Self {
        Self {
            id: item.id.to_string(),
            title: item.title.to_string(),
            notes: item.notes.to_string(),
            completed: item.completed,
            status: item.status,
            overdue: item.overdue,
        }
    }

    fn restore(self, item: &mut TaskItem) {
        item.title = SharedString::from(self.title);
        item.notes = SharedString::from(self.notes);
        item.completed = self.completed;
        item.status = self.status;
        item.overdue = self.overdue;
    }
}

/// The due-date filters shown at the top of the sidebar.
///
/// Keys are stable; labels are translated in `.slint`, and the query is the text
/// written into the search box when the button is pressed. Visibility mirrors
/// the defaults described in `design/ui/page/settings/settings.md`.
const DUE_FILTERS: &[(&str, &str, bool)] = &[
    ("overdue", "@overdue", false),
    ("today", "@today", true),
    ("tomorrow", "@tomorrow", true),
    ("three-days", "@3days", false),
    ("this-week", "@week", true),
    ("this-month", "@month", false),
    ("this-quarter", "@quarter", false),
    ("this-year", "@year", false),
    ("this-fiscal-year", "@fiscalyear", false),
];

/// Root ViewModel.
///
/// Generic over the repository bundle so tests can substitute
/// `MockInfrastructureRepositories` without a database.
pub struct AppViewModel<R> {
    repositories: Arc<R>,
    platform: Arc<dyn Platform>,
    runtime: Handle,
    state: Arc<Mutex<SharedState>>,
    timezone: DisplayTimezone,
}

impl<R> AppViewModel<R>
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    /// Builds the ViewModel. Does not touch storage.
    pub fn new(repositories: Arc<R>, platform: Arc<dyn Platform>, runtime: Handle) -> Self {
        Self {
            repositories,
            platform,
            runtime,
            state: Arc::new(Mutex::new(SharedState::default())),
            timezone: DisplayTimezone::System,
        }
    }

    /// Builds a ViewModel that renders every timestamp in UTC.
    ///
    /// Used by tests so assertions do not depend on the machine's timezone.
    pub fn new_for_test(
        repositories: Arc<R>,
        platform: Arc<dyn Platform>,
        runtime: Handle,
    ) -> Self {
        Self {
            timezone: DisplayTimezone::Utc,
            ..Self::new(repositories, platform, runtime)
        }
    }

    /// Publishes platform capabilities so views can hide unavailable features.
    pub fn apply_capabilities(&self, window: &AppWindow) {
        let caps = self.platform.capabilities();
        let ui = window.global::<UiCapabilities>();
        ui.set_system_tray(caps.has(Capability::SystemTray));
        ui.set_global_hotkey(caps.has(Capability::GlobalHotkey));
        ui.set_multi_window(caps.has(Capability::MultiWindow));
        ui.set_os_trash(caps.has(Capability::OsTrash));
        ui.set_arbitrary_file_path(caps.has(Capability::ArbitraryFilePath));
        ui.set_local_notification(caps.has(Capability::LocalNotification));
        ui.set_file_picker(caps.has(Capability::FilePicker));
        ui.set_background_sync(caps.has(Capability::BackgroundSync));
        ui.set_self_update(caps.has(Capability::SelfUpdate));
    }

    /// Publishes the due-date filter buttons.
    ///
    /// These are static UI affordances, not stored data, so they are set once at
    /// startup rather than loaded.
    pub fn apply_due_filters(&self, window: &AppWindow) {
        let items: Vec<DueFilterItem> = DUE_FILTERS
            .iter()
            .map(|(key, query, visible)| DueFilterItem {
                key: SharedString::from(*key),
                query: SharedString::from(*query),
                // Translated in .slint from the key; see I18n.due-filter-label.
                label: SharedString::from(*key),
                count: 0,
                visible: *visible,
            })
            .collect();

        window
            .global::<AppState>()
            .set_due_filters(ModelRc::new(VecModel::from(items)));
    }

    /// Registers every Slint callback.
    ///
    /// Closures capture only `Weak<AppWindow>` and `Arc` state; capturing the
    /// window strongly would keep it alive forever.
    pub fn bind(&self, window: &AppWindow) {
        self.bind_navigation(window);
        self.bind_search(window);
        self.bind_task_mutations(window);
        self.bind_shell(window);
    }

    // -- Navigation ---------------------------------------------------------

    fn bind_navigation(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            actions.on_toggle_project_expansion(move |project_id| {
                let id = project_id.to_string();
                let expanded = {
                    let mut state = state.lock().expect("shared state poisoned");
                    if state.expanded_projects.remove(&id) {
                        false
                    } else {
                        state.expanded_projects.insert(id.clone());
                        true
                    }
                };
                if let Some(window) = weak.upgrade() {
                    update_project_row(&window, &id, |item| item.expanded = expanded);
                }
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_select_project(move |project_id| {
                {
                    let mut state = state.lock().expect("shared state poisoned");
                    state.selected_project_id = project_id.to_string();
                    state.selected_list_id.clear();
                }
                let Some(window) = weak.upgrade() else { return };
                let app_state = window.global::<AppState>();
                app_state.set_selected_project_id(project_id);
                app_state.set_selected_list_id(SharedString::default());
                clear_selection(&window);
                refresh_tasks(&window, &state, timezone);
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_select_task_list(move |project_id, list_id| {
                {
                    let mut state = state.lock().expect("shared state poisoned");
                    state.selected_project_id = project_id.to_string();
                    state.selected_list_id = list_id.to_string();
                }
                let Some(window) = weak.upgrade() else { return };
                let app_state = window.global::<AppState>();
                app_state.set_selected_project_id(project_id);
                app_state.set_selected_list_id(list_id);
                app_state.set_sidebar_open(false);
                clear_selection(&window);
                refresh_tasks(&window, &state, timezone);
            });
        }

        {
            let weak = window.as_weak();
            actions.on_select_task(move |task_id| {
                if let Some(window) = weak.upgrade() {
                    select_task(&window, &task_id);
                }
            });
        }

        // Selecting a subtask also expands its parent row, so the list shows
        // where the detail pane's content came from.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            actions.on_select_subtask(move |subtask_id| {
                let Some(window) = weak.upgrade() else { return };
                let parent = window
                    .global::<AppState>()
                    .get_tasks()
                    .iter()
                    .find(|task| task.subtasks.iter().any(|sub| sub.id == subtask_id))
                    .map(|task| task.id.to_string());

                if let Some(parent_id) = parent {
                    {
                        let mut state = state.lock().expect("shared state poisoned");
                        state.task_ui.expand(&parent_id);
                    }
                    update_task_row(&window, &parent_id, |item| item.expanded = true);
                    select_task(&window, &SharedString::from(parent_id.as_str()));
                }

                window
                    .global::<AppState>()
                    .set_selected_subtask_id(subtask_id);
            });
        }

        {
            let weak = window.as_weak();
            actions.on_go_to_parent_task(move || {
                let Some(window) = weak.upgrade() else { return };
                let app_state = window.global::<AppState>();
                let parent_id = app_state.get_selected_task().id;
                app_state.set_selected_subtask_id(SharedString::default());
                select_task(&window, &parent_id);
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            actions.on_toggle_task_expansion(move |task_id| {
                let id = task_id.to_string();
                let expanded = {
                    let mut state = state.lock().expect("shared state poisoned");
                    state.task_ui.toggle(&id)
                };
                if let Some(window) = weak.upgrade() {
                    update_task_row(&window, &id, |item| item.expanded = expanded);
                }
            });
        }
    }

    // -- Search -------------------------------------------------------------

    fn bind_search(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_search_changed(move |query| {
                let Some(window) = weak.upgrade() else { return };
                window.global::<AppState>().set_search_query(query);
                refresh_tasks(&window, &state, timezone);
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_due_filter_clicked(move |key| {
                let Some(window) = weak.upgrade() else { return };
                let app_state = window.global::<AppState>();
                let query = app_state
                    .get_due_filters()
                    .iter()
                    .find(|f| f.key == key)
                    .map(|f| f.query.clone())
                    .unwrap_or_default();

                // Pressing an active filter clears it, so the button doubles as a toggle.
                let next = if app_state.get_search_query() == query {
                    SharedString::default()
                } else {
                    query
                };
                app_state.set_search_query(next);
                app_state.set_sidebar_open(false);
                refresh_tasks(&window, &state, timezone);
            });
        }
    }

    // -- Task mutations -----------------------------------------------------

    fn bind_task_mutations(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        // Add task: no optimistic row, because the id is assigned here and the
        // ordering comes from storage. The list is reloaded on success.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_add_task(move |list_id, title| {
                let title = title.trim().to_string();
                if title.is_empty() || list_id.is_empty() {
                    return;
                }

                let (project_id, user_id, order_index) = {
                    let state = state.lock().expect("shared state poisoned");
                    let project = state.project_of_list(&list_id);
                    let order = state
                        .trees
                        .iter()
                        .flat_map(|tree| tree.task_lists.iter())
                        .find(|list| list.id.as_str() == list_id.as_str())
                        .map(|list| list.tasks.len() as i32)
                        .unwrap_or(0);
                    (project, state.current_user, order)
                };

                let (Some(project_id), Some(user_id)) = (project_id, user_id) else {
                    report_error(&weak, "task.save-failed");
                    tracing::error!(%list_id, "cannot add a task: unknown project or user");
                    return;
                };

                let Ok(list_id) = TaskListId::try_from_str(list_id.as_str()) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let now = Utc::now();
                let task = Task {
                    id: TaskId::new(),
                    project_id,
                    list_id,
                    title,
                    description: None,
                    status: DomainStatus::NotStarted,
                    priority: 0,
                    plan_start_date: None,
                    plan_end_date: None,
                    do_start_date: None,
                    do_end_date: None,
                    is_range_date: None,
                    recurrence_rule: None,
                    order_index,
                    is_archived: false,
                    assigned_user_ids: Vec::new(),
                    tag_ids: Vec::new(),
                    created_at: now,
                    updated_at: now,
                    deleted: false,
                    updated_by: user_id,
                };

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let result = task_facades::create_task(
                        repositories.as_ref(),
                        &project_id,
                        &task,
                        &user_id,
                    )
                    .await;

                    match result {
                        Ok(_) => {
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(detail) => {
                            tracing::error!(%detail, "failed to create task");
                            report_error(&weak, "task.save-failed");
                        }
                    }
                });
            });
        }

        // Toggle completion: optimistic, with rollback on failure.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            actions.on_toggle_task_completed(move |task_id| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                let completed = !before.completed;
                let snapshot = TaskRowSnapshot::capture(&before);

                update_task_row(&window, &task_id, |item| {
                    item.completed = completed;
                    item.status = if completed {
                        crate::bindings::TaskStatus::Completed
                    } else {
                        crate::bindings::TaskStatus::NotStarted
                    };
                    item.overdue = item.overdue && !completed;
                });
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    status: Some(if completed {
                        DomainStatus::Completed
                    } else {
                        DomainStatus::NotStarted
                    }),
                    ..Default::default()
                };

                spawn_task_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    task_id.to_string(),
                    patch,
                    snapshot,
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            actions.on_update_task_title(move |task_id, title| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                if before.title == title {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);

                update_task_row(&window, &task_id, |item| item.title = title.clone());

                let patch = PartialTask {
                    title: Some(title.to_string()),
                    ..Default::default()
                };
                spawn_task_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    task_id.to_string(),
                    patch,
                    snapshot,
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            actions.on_update_task_notes(move |task_id, notes| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                if before.notes == notes {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);

                update_task_row(&window, &task_id, |item| item.notes = notes.clone());

                let patch = PartialTask {
                    description: Some(Some(notes.to_string())),
                    ..Default::default()
                };
                spawn_task_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    task_id.to_string(),
                    patch,
                    snapshot,
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_toggle_subtask_completed(move |subtask_id| {
                let Some(window) = weak.upgrade() else { return };

                let completed = window
                    .global::<AppState>()
                    .get_tasks()
                    .iter()
                    .flat_map(|task| task.subtasks.iter().collect::<Vec<_>>())
                    .find(|sub| sub.id == subtask_id)
                    .map(|sub| !sub.completed);
                let Some(completed) = completed else { return };

                let (project_id, user_id) = {
                    let state = state.lock().expect("shared state poisoned");
                    (state.project_of_subtask(&subtask_id), state.current_user)
                };
                let (Some(project_id), Some(user_id)) = (project_id, user_id) else {
                    report_error(&weak, "task.save-failed");
                    return;
                };
                let Ok(subtask_id) = SubTaskId::try_from_str(subtask_id.as_str()) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let patch = PartialSubTask {
                    completed: Some(completed),
                    status: Some(if completed {
                        DomainStatus::Completed
                    } else {
                        DomainStatus::NotStarted
                    }),
                    ..Default::default()
                };

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let result = subtask_facades::update_sub_task(
                        repositories.as_ref(),
                        &project_id,
                        &subtask_id,
                        &patch,
                        &user_id,
                    )
                    .await;

                    match result {
                        Ok(_) => reload_projects(&weak, &state, &repositories, timezone).await,
                        Err(detail) => {
                            tracing::error!(%detail, "failed to update subtask");
                            report_error(&weak, "task.save-failed");
                        }
                    }
                });
            });
        }

        // Deletion is not wired yet: `task_facades::delete_task` requires a
        // `TransactionManager<Transaction = sea_orm::DatabaseTransaction>` bound,
        // which would leak the storage engine into the UI crate and break the
        // layering the architecture check enforces. The bound has to move behind
        // `flequit-infrastructure` first.
        {
            let weak = window.as_weak();
            actions.on_delete_task(move |task_id| {
                tracing::warn!(%task_id, "task deletion is not implemented yet");
                report_error(&weak, "task.delete-failed");
            });
        }
    }

    // -- Shell --------------------------------------------------------------

    fn bind_shell(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        {
            let weak = window.as_weak();
            actions.on_dismiss_error(move || {
                if let Some(window) = weak.upgrade() {
                    window
                        .global::<AppState>()
                        .set_error_message(SharedString::default());
                }
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_retry_last_action(move || {
                if let Some(window) = weak.upgrade() {
                    let app_state = window.global::<AppState>();
                    app_state.set_error_message(SharedString::default());
                    app_state.set_loading(true);
                }
                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    reload_projects(&weak, &state, &repositories, timezone).await;
                });
            });
        }

        {
            actions.on_open_settings(move || {
                // The settings screen is not built yet; log rather than silently
                // ignoring the press, so the gap is visible in a bug report.
                tracing::info!("settings screen is not implemented yet");
            });
        }

        {
            let weak = window.as_weak();
            window.global::<I18n>().on_locale_changed(move |locale| {
                // Bundled translations re-evaluate every @tr() automatically;
                // update_all_translations() is not needed.
                if slint::select_bundled_translation(locale.as_str()).is_err() {
                    tracing::warn!(%locale, "no bundled translation for locale");
                    return;
                }
                if let Some(window) = weak.upgrade() {
                    window.global::<I18n>().set_current_locale(locale);
                }
            });
        }
    }

    /// Loads projects and tasks, then publishes them to the UI.
    ///
    /// Returns immediately; the work happens on the Tokio runtime and the
    /// result is applied on the UI thread.
    pub fn load_initial(&self, window: &AppWindow) {
        window.global::<AppState>().set_loading(true);

        let weak = window.as_weak();
        let repositories = Arc::clone(&self.repositories);
        let state = Arc::clone(&self.state);
        let timezone = self.timezone;

        self.runtime.spawn(async move {
            let account = initialization_facades::load_current_account(repositories.as_ref()).await;
            match account {
                Ok(Some(account)) => {
                    state.lock().expect("shared state poisoned").current_user =
                        Some(account.user_id);
                }
                Ok(None) => tracing::warn!("no current account; writes will be rejected"),
                Err(detail) => tracing::error!(%detail, "failed to load the current account"),
            }

            reload_projects(&weak, &state, &repositories, timezone).await;
        });
    }
}

// -- Free functions ---------------------------------------------------------

/// Reloads the project hierarchy and republishes both models.
///
/// Mutations reload rather than patching the cached tree: the tree is the source
/// for task ordering and list membership, and keeping a second copy in sync with
/// storage is the kind of duplication that drifts silently.
async fn reload_projects<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    timezone: DisplayTimezone,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let loaded = initialization_facades::load_all_project_trees(repositories.as_ref()).await;

    let outcome = match loaded {
        Ok(trees) => Ok(trees),
        Err(detail) => {
            tracing::error!(%detail, "failed to load project trees");
            Err("project.load-failed")
        }
    };

    let state = Arc::clone(state);
    let posted = weak.upgrade_in_event_loop(move |window| {
        let app_state = window.global::<AppState>();
        app_state.set_loading(false);

        match outcome {
            Ok(trees) => {
                {
                    let mut guard = state.lock().expect("shared state poisoned");
                    guard.trees = trees;
                    ensure_default_selection(&mut guard);
                }
                publish_selection(&window, &state);
                refresh_projects(&window, &state);
                refresh_tasks(&window, &state, timezone);
            }
            Err(code) => {
                let message = window.global::<I18n>().invoke_error_message(code.into());
                app_state.set_error_message(message);
            }
        }
    });

    if let Err(error) = posted {
        tracing::error!(%error, "could not deliver the load result to the UI thread");
    }
}

/// Selects the first project and list when nothing is selected yet.
///
/// Without this the app opens with an empty task pane and a disabled "add task"
/// field, which reads as a broken screen rather than an empty one.
fn ensure_default_selection(state: &mut SharedState) {
    let still_valid = state
        .trees
        .iter()
        .any(|tree| tree.id.as_str() == state.selected_project_id);
    if !state.selected_project_id.is_empty() && still_valid {
        return;
    }

    let Some(first_project) = state.trees.first() else {
        state.selected_project_id.clear();
        state.selected_list_id.clear();
        return;
    };

    state.selected_project_id = first_project.id.as_str();
    state.expanded_projects.insert(first_project.id.as_str());
    state.selected_list_id = first_project
        .task_lists
        .first()
        .map(|list| list.id.as_str())
        .unwrap_or_default();
}

/// Mirrors the selection held in shared state onto the UI globals.
fn publish_selection(window: &AppWindow, state: &Arc<Mutex<SharedState>>) {
    let state = state.lock().expect("shared state poisoned");
    let app_state = window.global::<AppState>();
    app_state.set_selected_project_id(SharedString::from(state.selected_project_id.as_str()));
    app_state.set_selected_list_id(SharedString::from(state.selected_list_id.as_str()));
}

/// Rebuilds the sidebar project model from the cached tree snapshot.
fn refresh_projects(window: &AppWindow, state: &Arc<Mutex<SharedState>>) {
    let state = state.lock().expect("shared state poisoned");
    let items: Vec<ProjectItem> = state
        .trees
        .iter()
        .filter(|tree| !tree.deleted)
        .map(|tree| {
            let expanded = state.expanded_projects.contains(&tree.id.as_str());
            to_project_item(tree, expanded)
        })
        .collect();

    tracing::debug!(projects = items.len(), "publishing sidebar projects");
    window
        .global::<AppState>()
        .set_projects(ModelRc::new(VecModel::from(items)));
}

/// Rebuilds the task model for the current selection and search text.
///
/// This is a full replacement, which is correct here: the visible set changes
/// wholesale when the selection or query changes. Single-row edits must use
/// [`update_task_row`] instead.
fn refresh_tasks(window: &AppWindow, state: &Arc<Mutex<SharedState>>, timezone: DisplayTimezone) {
    let now = Utc::now();
    let query = window
        .global::<AppState>()
        .get_search_query()
        .to_lowercase();
    let state = state.lock().expect("shared state poisoned");

    let items: Vec<TaskItem> = state
        .trees
        .iter()
        .filter(|tree| {
            state.selected_project_id.is_empty() || tree.id.as_str() == state.selected_project_id
        })
        .flat_map(|tree| tree.task_lists.iter())
        .filter(|list| {
            state.selected_list_id.is_empty() || list.id.as_str() == state.selected_list_id
        })
        .flat_map(|list| list.tasks.iter())
        .filter(|task| !task.deleted && !task.is_archived)
        .filter(|task| matches_query(&task.title, task.description.as_deref(), &query))
        .map(|task| {
            let expanded = state.task_ui.is_expanded(&task.id.as_str());
            to_task_item(task, expanded, &now, timezone)
        })
        .collect();

    tracing::debug!(
        tasks = items.len(),
        project = %state.selected_project_id,
        list = %state.selected_list_id,
        "publishing task list"
    );
    window
        .global::<AppState>()
        .set_tasks(ModelRc::new(VecModel::from(items)));
}

/// Plain substring match over the title and notes.
///
/// The keyword syntax described in `design/ui/page/main/main.md` (`@today`,
/// `#tag`, `@list:`) is not parsed yet; a query starting with `@` or `#` is
/// treated as "no filter" so those buttons do not hide everything.
fn matches_query(title: &str, notes: Option<&str>, query: &str) -> bool {
    if query.is_empty() || query.starts_with('@') || query.starts_with('#') {
        return true;
    }
    title.to_lowercase().contains(query) || notes.is_some_and(|n| n.to_lowercase().contains(query))
}

/// Persists a task patch, rolling the row back if the write fails.
#[allow(clippy::too_many_arguments)]
fn spawn_task_patch<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    task_id: String,
    patch: PartialTask,
    snapshot: TaskRowSnapshot,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let (project_id, user_id) = {
        let state = state.lock().expect("shared state poisoned");
        (state.project_of_task(&task_id), state.current_user)
    };
    let (Some(project_id), Some(user_id)) = (project_id, user_id) else {
        tracing::error!(%task_id, "cannot update a task: unknown project or user");
        rollback_task_row(weak, snapshot);
        report_error(weak, "task.save-failed");
        return;
    };
    let Ok(parsed_id) = TaskId::try_from_str(&task_id) else {
        rollback_task_row(weak, snapshot);
        report_error(weak, "input.malformed-id");
        return;
    };

    let weak = weak.clone();
    let repositories = Arc::clone(repositories);
    runtime.spawn(async move {
        let result = task_facades::update_task(
            repositories.as_ref(),
            &project_id,
            &parsed_id,
            &patch,
            &user_id,
        )
        .await;

        if let Err(detail) = result {
            tracing::error!(%detail, %task_id, "failed to update task");
            rollback_task_row(&weak, snapshot);
            report_error(&weak, "task.save-failed");
        }
    });
}

/// Restores a row to the value captured before the optimistic update.
fn rollback_task_row(weak: &Weak<AppWindow>, snapshot: TaskRowSnapshot) {
    let posted = weak.upgrade_in_event_loop(move |window| {
        let id = SharedString::from(snapshot.id.as_str());
        update_task_row(&window, &id, move |item| snapshot.restore(item));
        sync_selected_task(&window, &id);
    });
    if let Err(error) = posted {
        tracing::error!(%error, "could not roll back the optimistic update");
    }
}

/// Shows an error, resolving the message through the translation catalogue.
fn report_error(weak: &Weak<AppWindow>, code: &'static str) {
    let posted = weak.upgrade_in_event_loop(move |window| {
        let message = window.global::<I18n>().invoke_error_message(code.into());
        let app_state = window.global::<AppState>();
        app_state.set_loading(false);
        app_state.set_error_message(message);
    });
    if let Err(error) = posted {
        tracing::error!(%error, code, "could not deliver an error to the UI thread");
    }
}

/// Reads the current UI row for a task.
fn task_row(window: &AppWindow, task_id: &SharedString) -> Option<TaskItem> {
    window
        .global::<AppState>()
        .get_tasks()
        .iter()
        .find(|item| item.id == task_id)
}

/// Selects a task and mirrors the resolved row into `AppState`.
///
/// The view cannot search the model itself — Slint bindings are declarative
/// expressions — so the lookup happens here and the result is published as a
/// plain property.
fn select_task(window: &AppWindow, task_id: &SharedString) {
    let app_state = window.global::<AppState>();

    match task_row(window, task_id) {
        Some(item) => {
            app_state.set_selected_task_id(task_id.clone());
            app_state.set_selected_task(item);
            app_state.set_has_selected_task(true);
            // Compact shows one pane at a time; move to the detail view.
            app_state.set_active_pane(Pane::Detail);
        }
        None => {
            tracing::warn!(%task_id, "selection target is not in the visible model");
            clear_selection(window);
        }
    }
}

/// Keeps the detail pane in step after a row changed in place.
fn sync_selected_task(window: &AppWindow, task_id: &SharedString) {
    let app_state = window.global::<AppState>();
    if app_state.get_selected_task_id() != task_id {
        return;
    }
    if let Some(item) = task_row(window, task_id) {
        app_state.set_selected_task(item);
    }
}

/// Clears the selection and returns to the list pane.
fn clear_selection(window: &AppWindow) {
    let app_state = window.global::<AppState>();
    app_state.set_selected_task_id(SharedString::default());
    app_state.set_selected_subtask_id(SharedString::default());
    app_state.set_has_selected_task(false);
    app_state.set_active_pane(Pane::List);
}

/// Applies a change to a single task row without rebuilding the model.
///
/// Rebuilding would reset scroll position and discard row identity, so
/// single-value edits always go through here.
fn update_task_row(window: &AppWindow, task_id: &str, edit: impl FnOnce(&mut TaskItem)) {
    let model = window.global::<AppState>().get_tasks();
    let Some(index) = model.iter().position(|item| item.id == task_id) else {
        return;
    };
    let Some(mut item) = model.row_data(index) else {
        return;
    };
    edit(&mut item);
    model.set_row_data(index, item);
}

/// Applies a change to a single sidebar project row.
fn update_project_row(window: &AppWindow, project_id: &str, edit: impl FnOnce(&mut ProjectItem)) {
    let model = window.global::<AppState>().get_projects();
    let Some(index) = model.iter().position(|item| item.id == project_id) else {
        return;
    };
    let Some(mut item) = model.row_data(index) else {
        return;
    };
    edit(&mut item);
    model.set_row_data(index, item);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches_query("anything", None, ""));
    }

    #[test]
    fn a_keyword_query_is_not_applied_as_a_substring() {
        // "@today" must not be matched against titles; until the keyword parser
        // exists it means "no filter" rather than "hide everything".
        assert!(matches_query("Buy milk", None, "@today"));
        assert!(matches_query("Buy milk", None, "#home"));
    }

    #[test]
    fn a_plain_query_matches_the_title_case_insensitively() {
        assert!(matches_query("Buy Milk", None, "milk"));
        assert!(!matches_query("Buy Milk", None, "bread"));
    }

    #[test]
    fn a_plain_query_matches_the_notes() {
        assert!(matches_query("Errand", Some("get bread"), "bread"));
    }
}
