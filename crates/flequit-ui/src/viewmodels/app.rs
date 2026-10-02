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

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Utc};
use flequit_core::InfrastructureRepositoriesTrait;
use flequit_core::facades::{
    initialization_facades, project_facades, recurrence_facades, tag_bookmark_facades, tag_facades,
    task_facades, task_list_facades, user_facades,
};
use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::tag::Tag;
use flequit_model::models::task_projects::task::{PartialTask, Task, TaskTree};
use flequit_model::models::task_projects::task_list::TaskListTree;
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::types::id_types::{
    ProjectId, RecurrenceRuleId, TagId, TaskId, TaskListId, UserId,
};
use flequit_model::types::task_types::TaskStatus as DomainStatus;
use flequit_platform::{
    Capability, LifecycleEvent, LifecycleObserver, NotificationId, NotificationRequest,
    PermissionState, Platform, PlatformError, SystemTheme,
};
use flequit_types::errors::service_error::ServiceError;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};
use tokio::runtime::Handle;

use crate::UiError;
use crate::adapters::color::{PROJECT_COLORS, parse_hex};
use crate::adapters::datetime::{
    DateTimeDisplaySettings, DateTimeParts, DisplayTimezone, format_due, from_display_parts,
    is_overdue, to_display_parts,
};
use crate::adapters::task::from_status;
use crate::adapters::{to_bookmarked_tag_item, to_project_item, to_recurrence_state, to_tag_item};
use crate::bindings::{
    Actions, AppState, AppWindow, Capabilities as UiCapabilities, ColorOption, I18n, Pane,
    ProjectItem, SettingsState as UiSettingsState, TagItem, TaskItem, TaskPriority, TaskSort,
    TaskStatus, Theme,
};
use crate::viewmodels::TaskListUiViewModel;
use crate::viewmodels::ordering;
use crate::viewmodels::project_editor;
use crate::viewmodels::recurrence::{
    self, NextOccurrence, occurrence, preview_limit, rule_from_state,
};
use crate::viewmodels::reload_gate::ReloadGate;
use crate::viewmodels::search::{NameIndex, QueryEdit as SearchEdit, SearchSession, UserEntry};
use crate::viewmodels::settings::{SearchMemory, SettingsStore, SettingsViewModel, UserSettings};

mod deletion;
mod query;
mod sync_diagnostics;
use crate::viewmodels::tag_editor;
use crate::viewmodels::tag_suggestion;
use query::{publish_search, refresh_tasks};

const HELP_URL: &str = "https://github.com/varubogu/flequit_slint";
const TEXT_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug)]
struct PendingEdit<S> {
    revision: u64,
    rollback_snapshot: S,
}

/// Coalesces repeated edits of the same row while retaining the snapshot from
/// before the first keystroke for a correct rollback.
#[derive(Debug)]
struct EditDebouncer<S> {
    pending: Mutex<HashMap<String, PendingEdit<S>>>,
}

impl<S> Default for EditDebouncer<S> {
    fn default() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }
}

impl<S> EditDebouncer<S> {
    fn schedule(&self, key: String, rollback_snapshot: S) -> u64 {
        let mut pending = self.pending.lock().expect("edit debouncer poisoned");
        let entry = pending.entry(key).or_insert(PendingEdit {
            revision: 0,
            rollback_snapshot,
        });
        entry.revision = entry.revision.wrapping_add(1);
        entry.revision
    }

    fn take_if_latest(&self, key: &str, revision: u64) -> Option<S> {
        let mut pending = self.pending.lock().expect("edit debouncer poisoned");
        if pending.get(key)?.revision != revision {
            return None;
        }
        pending.remove(key).map(|edit| edit.rollback_snapshot)
    }
}

struct ThemeMonitor {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ThemeMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::warn!("system theme monitor panicked");
        }
    }
}

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
    /// The search box: its text and what each confirmed `@` token points at.
    ///
    /// The query is the only thing that decides which tasks are listed; the
    /// sidebar holds no selection of its own.
    search: SearchSession,
    /// Every name the search box can resolve. Rebuilt on each reload.
    search_index: NameIndex,
    /// The stored query, restored once the first load has the names it needs.
    pending_search: Option<String>,
    /// Whether the disambiguation list was open at the last publish.
    candidates_shown: bool,
    /// Users an `@` token can name.
    users: Vec<UserEntry>,
    /// Saves the query and quick-add history. Set when the window is bound.
    search_memory: Option<SearchMemory>,
    /// What was last saved, so an unchanged state is not saved again.
    remembered: (String, Vec<String>),
    /// The project the tag manager and "new list" work on, derived from the
    /// query, the selected task or the quick-add destination.
    context_project_id: String,
    /// Where the quick-add field puts new tasks.
    add_target: Option<String>,
    /// A destination picked by hand, and the default it replaced. Kept until
    /// the query changes what the default would be.
    add_target_override: Option<(String, Option<String>)>,
    /// Task lists recently added to, newest first.
    recent_add_targets: Vec<String>,
    /// Narrows the destination picker.
    add_target_filter: String,
    /// A task just added from the quick-add field. Listed even when the query
    /// excludes it, until the query changes, so the new task does not vanish.
    just_added: Option<String>,
    /// Whether archived projects are listed in the sidebar.
    ///
    /// Archiving would otherwise be one-way: a hidden project has no row to
    /// open the editor from.
    show_archived_projects: bool,
    /// How the visible task list is ordered.
    ///
    /// Session state for now: view preferences are not persisted yet
    /// (`docs/ja/develop/design/data/user-preferences.md`).
    task_sort: TaskSort,
    /// Whether completed tasks are left out of the task list.
    ///
    /// Applied on top of the query, so the current filter can be narrowed to
    /// what is still open without rewriting it. Session state, like `task_sort`.
    hide_completed_tasks: bool,
    /// Tag names by id, for every loaded project.
    ///
    /// Tasks carry only tag ids, and both the row labels and `#tag` searches
    /// need names, so they are resolved once per load instead of per row.
    tag_names: HashMap<TagId, String>,
    /// Full tag rows grouped by their owning project.
    tags_by_project: HashMap<ProjectId, Vec<Tag>>,
    /// Sidebar pins belonging to the current user.
    tag_bookmarks: Vec<TagBookmark>,
    /// What is in the detail pane's "add a tag" field, composing text included.
    /// Kept so a reload can narrow the refreshed tags the same way.
    tag_query: String,
    /// Author recorded on every write. Resolved from the current account.
    current_user: Option<UserId>,
    /// Keeps a burst of edits from running one full reload each.
    reload: ReloadGate,
    /// Tasks created by completing a repeating task, keyed by the completed
    /// task. Undoing that completion withdraws the successor again while it is
    /// still untouched. Session state: after a restart the pair is just two
    /// ordinary tasks.
    successors: HashMap<TaskId, Successor>,
    /// A deleted task still in storage while the toast offers to undo it.
    pending_deletion: Option<deletion::PendingDeletion>,
    /// Numbers deletions, so a timer can tell whether its own is still pending.
    deletion_token: u64,
}

/// The occurrence a completed repeating task handed its series on to.
#[derive(Debug, Clone)]
struct Successor {
    /// The task and its subtasks as they were created, carrying the
    /// handed-over rule.
    created: NextOccurrence,
    /// The rule as the completed task had it, restored on withdrawal.
    rule_before: RecurrenceRule,
}

/// What a status change does to a repeating task's series.
enum SeriesStep {
    /// Completing the task creates the next one.
    HandOver(Successor),
    /// Undoing the completion takes the untouched next one back.
    Withdraw(Successor),
}

impl SharedState {
    /// Locates the project a task (at any depth) belongs to, using the cached
    /// tree.
    fn project_of_task(&self, task_id: &str) -> Option<ProjectId> {
        self.task_with_project(task_id).map(|(tree, _)| tree.id)
    }

    /// Locates a task at any depth and the project that owns it, using the
    /// cached tree.
    fn task_with_project(&self, task_id: &str) -> Option<(&ProjectTree, &TaskTree)> {
        let id = TaskId::try_from_str(task_id).ok()?;
        self.trees
            .iter()
            .find_map(|tree| tree.find_task(&id).map(|task| (tree, task)))
    }

    /// The tasks above `task_id`, top-level first. Empty for a top-level task
    /// or one that is not loaded.
    fn ancestors_of(&self, task_id: &str) -> Vec<&TaskTree> {
        let Ok(id) = TaskId::try_from_str(task_id) else {
            return Vec::new();
        };
        self.trees
            .iter()
            .flat_map(ProjectTree::root_tasks)
            .find_map(|root| root.path_to(&id))
            .unwrap_or_default()
    }

    /// Every live top-level task, with the project and the list that own it
    /// (`None` for a task directly under the project).
    ///
    /// The search query decides which of these are listed. Archived projects
    /// take part only while they are shown in the sidebar.
    fn live_tasks(&self) -> impl Iterator<Item = (&ProjectTree, Option<&TaskListTree>, &TaskTree)> {
        self.live_projects()
            .flat_map(|tree| {
                let in_lists = tree
                    .task_lists
                    .iter()
                    .filter(|list| !list.deleted && !list.is_archived)
                    .flat_map(move |list| {
                        list.tasks.iter().map(move |task| (tree, Some(list), task))
                    });
                let own = tree.tasks.iter().map(move |task| (tree, None, task));
                in_lists.chain(own)
            })
            .filter(|(_, _, task)| !task.deleted && !task.is_archived)
    }

    /// Every project shown in the sidebar.
    fn live_projects(&self) -> impl Iterator<Item = &ProjectTree> {
        self.trees
            .iter()
            .filter(|tree| !tree.deleted && (self.show_archived_projects || !tree.is_archived))
    }

    /// Every live task list, with its project.
    fn live_lists(&self) -> impl Iterator<Item = (&ProjectTree, &TaskListTree)> {
        self.live_projects()
            .flat_map(|tree| tree.task_lists.iter().map(move |list| (tree, list)))
            .filter(|(_, list)| !list.deleted && !list.is_archived)
    }

    /// Resolves a task's tag ids to names, dropping ids the cache does not know.
    fn tag_names_of(&self, task: &TaskTree) -> Vec<&str> {
        task.tag_ids
            .iter()
            .filter_map(|id| self.tag_names.get(id).map(String::as_str))
            .collect()
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

    /// The live tag `tag_id` inside `project_id`, if the cache holds one.
    ///
    /// Tag writes are addressed by project, so a tag id arriving from the UI is
    /// checked against the project it claims to belong to before it is used.
    fn tag_in_project(&self, project_id: &ProjectId, tag_id: &TagId) -> Option<&Tag> {
        self.tags_by_project
            .get(project_id)
            .and_then(|tags| tags.iter().find(|tag| tag.id == *tag_id && !tag.deleted))
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
    priority: TaskPriority,
    due_label: String,
    has_due: bool,
    overdue: bool,
    due_parts: DateTimeParts,
    is_range_date: bool,
    start_label: String,
    has_start: bool,
    start_parts: DateTimeParts,
}

impl TaskRowSnapshot {
    fn capture(item: &TaskItem) -> Self {
        Self {
            id: item.id.to_string(),
            title: item.title.to_string(),
            notes: item.notes.to_string(),
            completed: item.completed,
            status: item.status,
            priority: item.priority,
            due_label: item.due_label.to_string(),
            has_due: item.has_due,
            overdue: item.overdue,
            due_parts: DateTimeParts {
                year: item.due_year,
                month: item.due_month,
                day: item.due_day,
                hour: item.due_hour,
                minute: item.due_minute,
            },
            is_range_date: item.is_range_date,
            start_label: item.start_label.to_string(),
            has_start: item.has_start,
            start_parts: DateTimeParts {
                year: item.start_year,
                month: item.start_month,
                day: item.start_day,
                hour: item.start_hour,
                minute: item.start_minute,
            },
        }
    }

    fn restore(self, item: &mut TaskItem) {
        item.title = SharedString::from(self.title);
        item.notes = SharedString::from(self.notes);
        item.completed = self.completed;
        item.status = self.status;
        item.priority = self.priority;
        item.due_label = SharedString::from(self.due_label);
        item.has_due = self.has_due;
        item.overdue = self.overdue;
        item.due_year = self.due_parts.year;
        item.due_month = self.due_parts.month;
        item.due_day = self.due_parts.day;
        item.due_hour = self.due_parts.hour;
        item.due_minute = self.due_parts.minute;
        item.is_range_date = self.is_range_date;
        item.start_label = SharedString::from(self.start_label);
        item.has_start = self.has_start;
        item.start_year = self.start_parts.year;
        item.start_month = self.start_parts.month;
        item.start_day = self.start_parts.day;
        item.start_hour = self.start_parts.hour;
        item.start_minute = self.start_parts.minute;
    }
}

/// Root ViewModel.
///
/// Generic over the repository bundle so tests can substitute
/// `MockInfrastructureRepositories` without a database.
pub struct AppViewModel<R> {
    repositories: Arc<R>,
    platform: Arc<dyn Platform>,
    runtime: Handle,
    state: Arc<Mutex<SharedState>>,
    // Shared so the font-scan thread can hand its result back to the settings
    // ViewModel from the UI thread.
    settings: Arc<SettingsViewModel>,
    timezone: DisplayTimezone,
    theme_monitor: Mutex<Option<ThemeMonitor>>,
}

impl<R> AppViewModel<R>
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    /// Builds the ViewModel. Does not touch storage.
    pub fn new(repositories: Arc<R>, platform: Arc<dyn Platform>, runtime: Handle) -> Self {
        let settings = Arc::new(SettingsViewModel::new_without_persistence(runtime.clone()));
        Self {
            repositories,
            platform,
            runtime,
            state: Arc::new(Mutex::new(SharedState::default())),
            settings,
            timezone: DisplayTimezone::System,
            theme_monitor: Mutex::new(None),
        }
    }

    /// Builds the ViewModel with application-provided persisted settings.
    pub fn new_with_settings(
        repositories: Arc<R>,
        platform: Arc<dyn Platform>,
        runtime: Handle,
        user_settings: UserSettings,
        settings_store: Arc<dyn SettingsStore>,
    ) -> Self {
        let timezone = DisplayTimezone::from_setting(&user_settings.timezone);
        let state = SharedState {
            pending_search: Some(user_settings.search_query.clone()),
            recent_add_targets: user_settings.recent_add_targets.clone(),
            remembered: (
                user_settings.search_query.clone(),
                user_settings.recent_add_targets.clone(),
            ),
            ..SharedState::default()
        };
        let settings = Arc::new(SettingsViewModel::new(
            user_settings,
            settings_store,
            runtime.clone(),
        ));
        Self {
            repositories,
            platform,
            runtime,
            state: Arc::new(Mutex::new(state)),
            settings,
            timezone,
            theme_monitor: Mutex::new(None),
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
            repositories,
            platform,
            settings: Arc::new(SettingsViewModel::new_for_test(runtime.clone())),
            runtime,
            state: Arc::new(Mutex::new(SharedState::default())),
            timezone: DisplayTimezone::Utc,
            theme_monitor: Mutex::new(None),
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
        ui.set_font_enumeration(caps.has(Capability::FontEnumeration));
    }

    /// Publishes the settings-backed due-date filter buttons.
    pub fn apply_due_filters(&self, window: &AppWindow) {
        self.settings.apply(window);
    }

    /// Publishes persisted settings to their Slint globals.
    pub fn apply_settings(&self, window: &AppWindow) {
        self.settings.apply(window);
    }

    /// Publishes the fixed project-colour palette.
    pub fn apply_project_colors(&self, window: &AppWindow) {
        let colors = PROJECT_COLORS
            .iter()
            .filter_map(|value| {
                parse_hex(value).map(|color| ColorOption {
                    value: SharedString::from(*value),
                    swatch: color.into(),
                })
            })
            .collect::<Vec<_>>();

        window
            .global::<AppState>()
            .set_project_colors(ModelRc::new(VecModel::from(colors)));
    }

    /// Registers every Slint callback.
    ///
    /// Closures capture only `Weak<AppWindow>` and `Arc` state; capturing the
    /// window strongly would keep it alive forever.
    pub fn bind(&self, window: &AppWindow) {
        self.state
            .lock()
            .expect("shared state poisoned")
            .search_memory = Some(self.settings.search_memory(window));
        self.bind_system_theme(window);
        self.load_font_options(window);
        self.bind_navigation(window);
        self.bind_search(window);
        self.bind_datetime_settings(window);
        self.bind_task_mutations(window);
        self.bind_task_ordering(window);
        self.bind_recurrence(window);
        self.bind_reminders(window);
        self.bind_project_management(window);
        self.bind_tag_management(window);
        self.settings.bind(window);
        sync_diagnostics::bind(window, &self.repositories, &self.runtime, self.timezone);
        self.bind_shell(window);
    }

    fn bind_datetime_settings(&self, window: &AppWindow) {
        let weak = window.as_weak();
        let state = Arc::clone(&self.state);
        let timezone = self.timezone;
        window
            .global::<Actions>()
            .on_refresh_datetime_display(move || {
                let Some(window) = weak.upgrade() else { return };
                refresh_tasks(&window, &state, timezone);
                let selected_task_id = window.global::<AppState>().get_selected_task_id();
                if !selected_task_id.is_empty() {
                    sync_selected_task(&window, &selected_task_id);
                }
            });
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

        // Any task, at any depth: a subtask opened from its parent's pane or a
        // parent opened from the breadcrumb.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_select_task(move |task_id| {
                if let Some(window) = weak.upgrade() {
                    open_task(&window, &state, &task_id, timezone);
                    refresh_tags(&window, &state);
                }
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            actions.on_move_task_selection(move |delta| {
                let Some(window) = weak.upgrade() else { return };
                if select_task_at_offset(&window, delta) {
                    refresh_tags(&window, &state);
                }
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            actions.on_select_task_boundary(move |first| {
                let Some(window) = weak.upgrade() else { return };
                if select_task_boundary(&window, first) {
                    refresh_tags(&window, &state);
                }
            });
        }

        // Subtasks are rows of their own, so opening or closing a task adds or
        // removes rows rather than changing one.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_toggle_task_expansion(move |task_id| {
                state
                    .lock()
                    .expect("shared state poisoned")
                    .task_ui
                    .toggle(task_id.as_str());
                if let Some(window) = weak.upgrade() {
                    refresh_tasks(&window, &state, timezone);
                }
            });
        }
    }

    // -- Search -------------------------------------------------------------

    fn bind_search(&self, window: &AppWindow) {
        query::bind(window, &self.state, self.timezone);
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
            actions.on_add_task(move |target, title| {
                let title = title.trim().to_string();
                if title.is_empty() || target.is_empty() {
                    return;
                }

                let (destination, user_id) = {
                    let state = state.lock().expect("shared state poisoned");
                    (query::destination(&state, &target), state.current_user)
                };
                let (Some(destination), Some(user_id)) = (destination, user_id) else {
                    report_error(&weak, "task.save-failed");
                    tracing::error!(%target, "cannot add a task: unknown destination or user");
                    return;
                };

                let task = new_task(
                    destination.project_id,
                    destination.list_id,
                    None,
                    title,
                    destination.order_index,
                    user_id,
                );
                let target = target.to_string();

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let result = task_facades::create_task(
                        repositories.as_ref(),
                        &task.project_id,
                        &task,
                        &user_id,
                    )
                    .await;

                    match result {
                        Ok(_) => {
                            query::note_task_added(&state, &target, task.id.as_str());
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, "failed to create task");
                            report_error(&weak, ui_error.code());
                        }
                    }
                });
            });
        }

        // A subtask is a task with a parent: it lives in the parent's project
        // and belongs to no list of its own.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_add_subtask(move |parent_id, title| {
                let title = title.trim().to_string();
                let parent_id = parent_id.to_string();
                if title.is_empty() || parent_id.is_empty() {
                    return;
                }

                let (parent, user_id) = {
                    let state = state.lock().expect("shared state poisoned");
                    let parent = state.task_with_project(&parent_id).map(|(tree, parent)| {
                        let order = next_order_index(
                            parent.sub_tasks.iter().map(|child| child.order_index),
                        );
                        (tree.id, parent.id, order)
                    });
                    (parent, state.current_user)
                };
                let (Some((project_id, parsed_parent_id, order_index)), Some(user_id)) =
                    (parent, user_id)
                else {
                    tracing::error!(%parent_id, "cannot add a subtask: unknown task or user");
                    report_error(&weak, "task.save-failed");
                    return;
                };

                let subtask = new_task(
                    project_id,
                    None,
                    Some(parsed_parent_id),
                    title,
                    order_index,
                    user_id,
                );

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let result = task_facades::create_task(
                        repositories.as_ref(),
                        &project_id,
                        &subtask,
                        &user_id,
                    )
                    .await;

                    match result {
                        Ok(_) => {
                            state
                                .lock()
                                .expect("shared state poisoned")
                                .task_ui
                                .expand(&parent_id);
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, %parent_id, "failed to create subtask");
                            report_error(&weak, ui_error.code());
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
            let platform = Arc::clone(&self.platform);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_toggle_task_completed(move |task_id| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    toggle_subtask_in_pane(
                        &window,
                        &state,
                        &repositories,
                        &runtime,
                        timezone,
                        &task_id,
                    );
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

                let step = if completed {
                    plan_next_occurrence(&window, &state, &task_id, timezone)
                        .map(SeriesStep::HandOver)
                } else {
                    take_untouched_successor(&state, &task_id).map(SeriesStep::Withdraw)
                };
                spawn_completion_patch(
                    &weak,
                    &state,
                    &repositories,
                    &platform,
                    &runtime,
                    timezone,
                    task_id.to_string(),
                    patch,
                    snapshot,
                    step,
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            actions.on_update_task_due(move |task_id, year, month, day, hour, minute| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                let parts = DateTimeParts {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                };
                let timezone = DisplayTimezone::from_setting(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                );
                let Some(due) = from_display_parts(parts, timezone) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                if before.has_due
                    && before.due_year == year
                    && before.due_month == month
                    && before.due_day == day
                    && before.due_hour == hour
                    && before.due_minute == minute
                {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);
                let completed = before.completed;

                let display = DateTimeDisplaySettings::new(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                    window
                        .global::<UiSettingsState>()
                        .get_current_datetime_format()
                        .as_str(),
                );
                update_task_row(&window, &task_id, |item| {
                    item.due_label = SharedString::from(format_due(&due, &display));
                    item.has_due = true;
                    item.overdue = is_overdue(Some(&due), completed, &Utc::now());
                    let parts = to_display_parts(&due, timezone);
                    item.due_year = parts.year;
                    item.due_month = parts.month;
                    item.due_day = parts.day;
                    item.due_hour = parts.hour;
                    item.due_minute = parts.minute;
                });
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    plan_end_date: Some(Some(due)),
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
            actions.on_clear_task_due(move |task_id| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                if !before.has_due {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);

                update_task_row(&window, &task_id, |item| {
                    item.due_label = SharedString::default();
                    item.has_due = false;
                    item.overdue = false;
                });
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    plan_end_date: Some(None),
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

        // A range task spans a start date and the due date. Turning the range
        // off also drops the start date: keeping a hidden value would come back
        // the next time the switch is flipped, which the user never asked for.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            actions.on_set_task_range(move |task_id, is_range| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                if before.is_range_date == is_range {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);

                update_task_row(&window, &task_id, |item| {
                    item.is_range_date = is_range;
                    if !is_range {
                        item.start_label = SharedString::default();
                        item.has_start = false;
                    }
                });
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    is_range_date: Some(Some(is_range)),
                    plan_start_date: if is_range { None } else { Some(None) },
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
            actions.on_update_task_start(move |task_id, year, month, day, hour, minute| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                let parts = DateTimeParts {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                };
                let timezone = DisplayTimezone::from_setting(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                );
                let Some(start) = from_display_parts(parts, timezone) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                if before.has_start
                    && before.start_year == year
                    && before.start_month == month
                    && before.start_day == day
                    && before.start_hour == hour
                    && before.start_minute == minute
                {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);

                let display = DateTimeDisplaySettings::new(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                    window
                        .global::<UiSettingsState>()
                        .get_current_datetime_format()
                        .as_str(),
                );
                update_task_row(&window, &task_id, |item| {
                    item.start_label = SharedString::from(format_due(&start, &display));
                    item.has_start = true;
                    item.is_range_date = true;
                    let parts = to_display_parts(&start, timezone);
                    item.start_year = parts.year;
                    item.start_month = parts.month;
                    item.start_day = parts.day;
                    item.start_hour = parts.hour;
                    item.start_minute = parts.minute;
                });
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    plan_start_date: Some(Some(start)),
                    is_range_date: Some(Some(true)),
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
            actions.on_clear_task_start(move |task_id| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                if !before.has_start {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);

                update_task_row(&window, &task_id, |item| {
                    item.start_label = SharedString::default();
                    item.has_start = false;
                });
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    plan_start_date: Some(None),
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
            let debouncer = Arc::new(EditDebouncer::default());
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
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    title: Some(title.to_string()),
                    ..Default::default()
                };
                spawn_debounced_task_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    &debouncer,
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
            let platform = Arc::clone(&self.platform);
            let runtime = self.runtime.clone();
            let startup_timezone = self.timezone;
            actions.on_update_task_status(move |task_id, status| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                if before.status == status {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);
                let completed = status == TaskStatus::Completed;
                let timezone = DisplayTimezone::from_setting(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                );
                let due = before.has_due.then(|| {
                    from_display_parts(
                        DateTimeParts {
                            year: before.due_year,
                            month: before.due_month,
                            day: before.due_day,
                            hour: before.due_hour,
                            minute: before.due_minute,
                        },
                        timezone,
                    )
                });
                let overdue = due
                    .flatten()
                    .is_some_and(|due| is_overdue(Some(&due), completed, &Utc::now()));

                update_task_row(&window, &task_id, |item| {
                    item.status = status;
                    item.completed = completed;
                    item.overdue = overdue;
                });
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    status: Some(from_status(status)),
                    ..Default::default()
                };
                // Only stepping into "completed" hands the series on, and only
                // stepping out of it takes the successor back; moving between
                // the other states leaves the series alone.
                let was_completed = before.status == TaskStatus::Completed;
                let step = if completed && !was_completed {
                    plan_next_occurrence(&window, &state, &task_id, startup_timezone)
                        .map(SeriesStep::HandOver)
                } else if !completed && was_completed {
                    take_untouched_successor(&state, &task_id).map(SeriesStep::Withdraw)
                } else {
                    None
                };
                spawn_completion_patch(
                    &weak,
                    &state,
                    &repositories,
                    &platform,
                    &runtime,
                    startup_timezone,
                    task_id.to_string(),
                    patch,
                    snapshot,
                    step,
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            actions.on_update_task_priority(move |task_id, priority| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = task_row(&window, &task_id) else {
                    return;
                };
                if before.priority == priority {
                    return;
                }
                let snapshot = TaskRowSnapshot::capture(&before);

                update_task_row(&window, &task_id, |item| item.priority = priority);
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    priority: Some(priority_value(priority)),
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
            let debouncer = Arc::new(EditDebouncer::default());
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
                sync_selected_task(&window, &task_id);

                let patch = PartialTask {
                    description: Some(Some(notes.to_string())),
                    ..Default::default()
                };
                spawn_debounced_task_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    &debouncer,
                    task_id.to_string(),
                    patch,
                    snapshot,
                );
            });
        }

        deletion::bind(
            window,
            &self.state,
            &self.repositories,
            &self.platform,
            &self.runtime,
            self.timezone,
        );
    }

    // -- Ordering -----------------------------------------------------------

    fn bind_task_ordering(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_change_task_sort(move |sort| {
                state.lock().expect("shared state poisoned").task_sort = sort;
                let Some(window) = weak.upgrade() else { return };
                window.global::<AppState>().set_task_sort(sort);
                refresh_tasks(&window, &state, timezone);
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_change_show_completed_tasks(move |show| {
                state
                    .lock()
                    .expect("shared state poisoned")
                    .hide_completed_tasks = !show;
                let Some(window) = weak.upgrade() else { return };
                window.global::<AppState>().set_show_completed_tasks(show);
                refresh_tasks(&window, &state, timezone);
            });
        }

        // Reorder: only top-level tasks move; their subtasks go with them. The
        // move is worked out on the visible top-level rows, whose new
        // neighbours tell storage where the task belongs. A failure reloads,
        // because the stored order is the only thing that can correct the list.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_reorder_task(move |task_id, target_index| {
                let Some(window) = weak.upgrade() else { return };
                let rows: Vec<TaskItem> = window.global::<AppState>().get_tasks().iter().collect();
                let Some(target) = ordering::root_position(&rows, target_index) else {
                    return;
                };
                move_top_level_task(
                    &window,
                    &state,
                    &repositories,
                    &runtime,
                    timezone,
                    &task_id,
                    |_| Some(target),
                );
            });
        }

        // Keyboard and screen-reader equivalent: one top-level row at a time.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_move_task_by(move |task_id, delta| {
                let Some(window) = weak.upgrade() else { return };
                move_top_level_task(
                    &window,
                    &state,
                    &repositories,
                    &runtime,
                    timezone,
                    &task_id,
                    |from| from.checked_add_signed(delta as isize),
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_move_task_to_list(move |task_id, list_id| {
                let updates = {
                    let mut guard = state.lock().expect("shared state poisoned");
                    ordering::move_task_to_list(&mut guard.trees, &task_id, &list_id)
                };
                let Some(updates) = updates else {
                    tracing::warn!(%task_id, %list_id, "cannot move the task to that list");
                    return;
                };
                apply_move(&weak, &state, &repositories, &runtime, timezone, updates);
            });
        }

        // Dropping on a project takes the task out of its list.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_move_task_to_project(move |task_id| {
                let updates = {
                    let mut guard = state.lock().expect("shared state poisoned");
                    ordering::move_task_to_project(&mut guard.trees, &task_id)
                };
                let Some(updates) = updates else {
                    tracing::warn!(%task_id, "cannot move the task out of its list");
                    return;
                };
                apply_move(&weak, &state, &repositories, &runtime, timezone, updates);
            });
        }
    }

    // -- Projects and task lists --------------------------------------------

    // -- Recurrence ---------------------------------------------------------

    fn bind_recurrence(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        // Open: fill in the working copy from the task's own rule, so the
        // dialog starts from what the task actually does today.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_open_recurrence(move |task_id| {
                let Some(window) = weak.upgrade() else { return };
                let display = display_settings(&window, timezone);

                let found = {
                    let guard = state.lock().expect("shared state poisoned");
                    guard
                        .task_with_project(task_id.as_str())
                        .map(|(_, task)| (task.recurrence_rule.clone(), task.plan_end_date))
                };
                let Some((rule, due)) = found else {
                    tracing::warn!(%task_id, "cannot edit a repeat schedule: unknown task");
                    return;
                };

                // A task with no due date has no first occurrence to count
                // from, so the preview starts from now.
                let anchor = due.unwrap_or_else(Utc::now);
                let app_state = window.global::<AppState>();
                let recurrence =
                    to_recurrence_state(task_id.as_str(), rule.as_ref(), &anchor, display.timezone);
                let preview_count = recurrence.preview_count;
                app_state.set_recurrence(recurrence);
                publish_recurrence_preview(
                    &window,
                    rule.as_ref(),
                    &anchor,
                    &display,
                    preview_count,
                );
                app_state.set_recurrence_open(true);
            });
        }

        // Preview: the dialog reports its whole draft after every edit, because
        // expanding a rule into dates is a domain judgement and .slint holds no
        // business rules.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_preview_recurrence(move |draft| {
                let Some(window) = weak.upgrade() else { return };
                let display = display_settings(&window, timezone);

                let found = {
                    let guard = state.lock().expect("shared state poisoned");
                    let task = guard.task_with_project(draft.task_id.as_str());
                    task.map(|(_, task)| {
                        (
                            task.recurrence_rule.clone(),
                            task.plan_end_date,
                            guard.current_user,
                        )
                    })
                };
                let Some((existing, due, user_id)) = found else {
                    return;
                };

                let anchor = due.unwrap_or_else(Utc::now);
                // The author is irrelevant to a preview; the draft is never
                // written from here.
                let rule = rule_from_state(
                    &draft,
                    existing.as_ref(),
                    display.timezone,
                    user_id.unwrap_or_else(UserId::new),
                );
                publish_recurrence_preview(
                    &window,
                    rule.as_ref(),
                    &anchor,
                    &display,
                    draft.preview_count,
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_save_recurrence(move |draft| {
                let Some(window) = weak.upgrade() else { return };
                let display = display_settings(&window, timezone);
                let task_id = draft.task_id.to_string();

                let (found, user_id) = {
                    let guard = state.lock().expect("shared state poisoned");
                    let found = guard
                        .task_with_project(&task_id)
                        .map(|(project, task)| (project.id, task.recurrence_rule.clone()));
                    (found, guard.current_user)
                };
                let (Some((project_id, existing)), Some(user_id)) = (found, user_id) else {
                    tracing::error!(%task_id, "cannot save a repeat schedule: unknown project or user");
                    report_error(&weak, "recurrence.save-failed");
                    return;
                };
                let Ok(parsed_task_id) = TaskId::try_from_str(&task_id) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let rule = rule_from_state(&draft, existing.as_ref(), display.timezone, user_id);
                if rule.is_none() && existing.is_none() {
                    return;
                }
                // A rule the task does not have yet still needs to be linked to
                // it; an edited one is already linked.
                let associate = existing.is_none();
                let existing_id = existing.map(|rule| rule.id);

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let (result, code) = match rule {
                        Some(rule) => (
                            write_recurrence(
                                repositories.as_ref(),
                                &project_id,
                                &parsed_task_id,
                                rule,
                                associate,
                                &user_id,
                            )
                            .await,
                            "recurrence.save-failed",
                        ),
                        None => (
                            erase_recurrence(
                                repositories.as_ref(),
                                &project_id,
                                &parsed_task_id,
                                existing_id,
                            )
                            .await,
                            "recurrence.delete-failed",
                        ),
                    };

                    match result {
                        Ok(()) => {
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, %task_id, "failed to save the repeat schedule");
                            report_error(&weak, code);
                        }
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_clear_recurrence(move |task_id| {
                let task_id = task_id.to_string();
                let found = {
                    let guard = state.lock().expect("shared state poisoned");
                    guard
                        .task_with_project(&task_id)
                        .map(|(project, task)| {
                            (project.id, task.recurrence_rule.as_ref().map(|rule| rule.id))
                        })
                };
                let Some((project_id, rule_id)) = found else {
                    tracing::warn!(%task_id, "cannot clear a repeat schedule: unknown task");
                    return;
                };
                let Ok(parsed_task_id) = TaskId::try_from_str(&task_id) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let result = erase_recurrence(
                        repositories.as_ref(),
                        &project_id,
                        &parsed_task_id,
                        rule_id,
                    )
                    .await;

                    match result {
                        Ok(()) => {
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, %task_id, "failed to clear the repeat schedule");
                            report_error(&weak, "recurrence.delete-failed");
                        }
                    }
                });
            });
        }
    }

    fn bind_reminders(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let platform = Arc::clone(&self.platform);
            let runtime = self.runtime.clone();
            actions.on_request_notification_permission(move || {
                if let Some(window) = weak.upgrade() {
                    window
                        .global::<UiSettingsState>()
                        .set_notification_permission_loading(true);
                }
                let weak = weak.clone();
                let state = Arc::clone(&state);
                let platform = Arc::clone(&platform);
                runtime.spawn(async move {
                    match platform.request_notification_permission().await {
                        Ok(permission) => {
                            publish_notification_permission(&weak, permission);
                            if permission == PermissionState::Granted {
                                let reminders = reminder_specs(&state);
                                schedule_reminders(platform.as_ref(), reminders).await;
                            }
                        }
                        Err(error) => {
                            tracing::error!(%error, "failed to request notification permission");
                            clear_notification_permission_loading(&weak);
                            report_error(&weak, "notification.permission-failed");
                        }
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            actions.on_add_relative_reminder(move |task_id, from_start, minutes_before| {
                let Some(window) = weak.upgrade() else { return };
                let Some(task) = task_row(&window, &task_id) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let display_timezone = DisplayTimezone::from_setting(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                );
                let reference_parts = if from_start && task.has_start {
                    DateTimeParts {
                        year: task.start_year,
                        month: task.start_month,
                        day: task.start_day,
                        hour: task.start_hour,
                        minute: task.start_minute,
                    }
                } else if !from_start && task.has_due {
                    DateTimeParts {
                        year: task.due_year,
                        month: task.due_month,
                        day: task.due_day,
                        hour: task.due_hour,
                        minute: task.due_minute,
                    }
                } else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let Some(reference) = from_display_parts(reference_parts, display_timezone) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let reminder =
                    reference - chrono::Duration::minutes(i64::from(minutes_before.max(0)));
                let parts = to_display_parts(&reminder, display_timezone);
                window.global::<Actions>().invoke_add_reminder(
                    task_id,
                    parts.year,
                    parts.month,
                    parts.day,
                    parts.hour,
                    parts.minute,
                );
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let platform = Arc::clone(&self.platform);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_add_reminder(move |task_id, year, month, day, hour, minute| {
                let Some(window) = weak.upgrade() else { return };
                let display_timezone = DisplayTimezone::from_setting(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                );
                let Some(reminder) = from_display_parts(
                    DateTimeParts {
                        year,
                        month,
                        day,
                        hour,
                        minute,
                    },
                    display_timezone,
                ) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                if reminder <= Utc::now() {
                    report_error(&weak, "reminder.must-be-future");
                    return;
                }

                let task_id = task_id.to_string();
                let found = {
                    let guard = state.lock().expect("shared state poisoned");
                    guard.task_with_project(&task_id).map(|(project, task)| {
                        (
                            project.id,
                            task.title.clone(),
                            task.reminders.clone(),
                            guard.current_user,
                        )
                    })
                };
                let Some((project_id, title, mut reminders, Some(user_id))) = found else {
                    report_error(&weak, "reminder.save-failed");
                    return;
                };
                if reminders.contains(&reminder) {
                    return;
                }
                reminders.push(reminder);
                reminders.sort_unstable();
                let Ok(parsed_task_id) = TaskId::try_from_str(&task_id) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                let platform = Arc::clone(&platform);
                runtime.spawn(async move {
                    let granted = match ensure_notification_permission(platform.as_ref()).await {
                        Ok(permission) => {
                            publish_notification_permission(&weak, permission);
                            permission == PermissionState::Granted
                        }
                        Err(error) => {
                            tracing::error!(%error, "failed to request notification permission");
                            report_error(&weak, "notification.permission-failed");
                            return;
                        }
                    };
                    if !granted {
                        report_error(&weak, "notification.permission-denied");
                        return;
                    }

                    let patch = PartialTask {
                        reminders: Some(reminders),
                        ..Default::default()
                    };
                    match task_facades::update_task(
                        repositories.as_ref(),
                        &project_id,
                        &parsed_task_id,
                        &patch,
                        &user_id,
                    )
                    .await
                    {
                        Ok(_) => {
                            if let Err(error) =
                                schedule_reminder(platform.as_ref(), &task_id, &title, reminder)
                                    .await
                            {
                                tracing::error!(%error, %task_id, "failed to schedule reminder");
                                report_error(&weak, "notification.schedule-failed");
                            }
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, %task_id, "failed to save reminder");
                            report_error(&weak, "reminder.save-failed");
                        }
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let platform = Arc::clone(&self.platform);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_remove_reminder(move |task_id, reminder_key| {
                let Ok(reminder) = DateTime::parse_from_rfc3339(reminder_key.as_str()) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let reminder = reminder.with_timezone(&Utc);
                let task_id = task_id.to_string();
                let found = {
                    let guard = state.lock().expect("shared state poisoned");
                    guard.task_with_project(&task_id).map(|(project, task)| {
                        (project.id, task.reminders.clone(), guard.current_user)
                    })
                };
                let Some((project_id, mut reminders, Some(user_id))) = found else {
                    report_error(&weak, "reminder.save-failed");
                    return;
                };
                let previous_len = reminders.len();
                reminders.retain(|candidate| *candidate != reminder);
                if reminders.len() == previous_len {
                    return;
                }
                let Ok(parsed_task_id) = TaskId::try_from_str(&task_id) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                let platform = Arc::clone(&platform);
                runtime.spawn(async move {
                    let patch = PartialTask {
                        reminders: Some(reminders),
                        ..Default::default()
                    };
                    match task_facades::update_task(
                        repositories.as_ref(),
                        &project_id,
                        &parsed_task_id,
                        &patch,
                        &user_id,
                    )
                    .await
                    {
                        Ok(_) => {
                            let id = NotificationId::scheduled(&task_id, &reminder);
                            if let Err(error) = platform.cancel_notification(&id).await {
                                tracing::warn!(%error, %task_id, "failed to cancel reminder");
                            }
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, %task_id, "failed to remove reminder");
                            report_error(&weak, "reminder.save-failed");
                        }
                    }
                });
            });
        }
    }

    fn bind_project_management(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        // Every one of these reloads instead of patching the cached tree: they
        // change what the tree contains rather than a field of one row, and the
        // sidebar, the task list and the selection all follow from it.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_create_project(move |name, color| {
                let Some(name) = project_editor::validated_name(&name) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let (user_id, order_index) = {
                    let state = state.lock().expect("shared state poisoned");
                    (
                        state.current_user,
                        next_order_index(
                            state
                                .trees
                                .iter()
                                .filter(|tree| !tree.deleted)
                                .map(|tree| tree.order_index),
                        ),
                    )
                };
                let Some(user_id) = user_id else {
                    tracing::error!("cannot create a project: no current user");
                    report_error(&weak, "project.save-failed");
                    return;
                };

                let project = project_editor::new_project(
                    name,
                    project_editor::validated_color(&color),
                    order_index,
                    user_id,
                );

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        project_facades::create_project(repositories.as_ref(), &project, &user_id)
                            .await
                            .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_update_project(move |project_id, name, color| {
                let Some(name) = project_editor::validated_name(&name) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let Some((project_id, user_id)) = project_write(&weak, &state, &project_id) else {
                    return;
                };
                let patch =
                    project_editor::project_patch(name, project_editor::validated_color(&color));

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        project_facades::update_project(
                            repositories.as_ref(),
                            &project_id,
                            &patch,
                            &user_id,
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_archive_project(move |project_id, archived| {
                let Some((project_id, user_id)) = project_write(&weak, &state, &project_id) else {
                    return;
                };
                let patch = project_editor::archive_patch(archived);

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        project_facades::update_project(
                            repositories.as_ref(),
                            &project_id,
                            &patch,
                            &user_id,
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_delete_project(move |project_id| {
                let Some((project_id, user_id)) = project_write(&weak, &state, &project_id) else {
                    return;
                };

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        project_facades::delete_project(
                            repositories.as_ref(),
                            &project_id,
                            &user_id,
                            &Utc::now(),
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let timezone = self.timezone;
            actions.on_toggle_archived_projects(move || {
                let showing = {
                    let mut guard = state.lock().expect("shared state poisoned");
                    guard.show_archived_projects = !guard.show_archived_projects;
                    guard.show_archived_projects
                };
                let Some(window) = weak.upgrade() else { return };
                window
                    .global::<AppState>()
                    .set_show_archived_projects(showing);
                refresh_projects(&window, &state);
                publish_search(&window, &state, timezone, false);
                clear_selection(&window);
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_create_task_list(move |project_id, name| {
                let Some(name) = project_editor::validated_name(&name) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let Some((project_id, user_id)) = project_write(&weak, &state, &project_id) else {
                    return;
                };
                let order_index = {
                    let state = state.lock().expect("shared state poisoned");
                    state
                        .trees
                        .iter()
                        .find(|tree| tree.id == project_id)
                        .map(|tree| {
                            next_order_index(
                                tree.task_lists
                                    .iter()
                                    .filter(|list| !list.deleted)
                                    .map(|list| list.order_index),
                            )
                        })
                        .unwrap_or(0)
                };
                let list = project_editor::new_task_list(project_id, name, order_index, user_id);

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        task_list_facades::create_task_list(
                            repositories.as_ref(),
                            &project_id,
                            &list,
                            &user_id,
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_update_task_list(move |list_id, name| {
                let Some(name) = project_editor::validated_name(&name) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let Some((project_id, list_id, user_id)) = list_write(&weak, &state, &list_id)
                else {
                    return;
                };
                let patch = project_editor::task_list_patch(name);

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        task_list_facades::update_task_list(
                            repositories.as_ref(),
                            &project_id,
                            &list_id,
                            &patch,
                            &user_id,
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_delete_task_list(move |list_id| {
                let Some((project_id, list_id, user_id)) = list_write(&weak, &state, &list_id)
                else {
                    return;
                };

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        task_list_facades::delete_task_list(
                            repositories.as_ref(),
                            &project_id,
                            &list_id,
                            &user_id,
                            &Utc::now(),
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }
    }

    // -- Tag management ----------------------------------------------------

    fn bind_tag_management(&self, window: &AppWindow) {
        let actions = window.global::<Actions>();

        // Every keystroke in the "add a tag" field, and every change to the
        // text the IME is composing there, narrows the suggestions.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            actions.on_tag_query_changed(move |query| {
                let Some(window) = weak.upgrade() else { return };
                state.lock().expect("shared state poisoned").tag_query = query.to_string();
                let tags: Vec<TagItem> = window.global::<AppState>().get_tags().iter().collect();
                publish_task_tags(&window, &tags, &query);
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_create_tag(move |project_id, name, color| {
                let Some(name) = tag_editor::validated_name(&name) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let Some((project_id, user_id)) = project_write(&weak, &state, &project_id) else {
                    return;
                };
                let order_index = {
                    let state = state.lock().expect("shared state poisoned");
                    next_order_index(
                        state
                            .tags_by_project
                            .get(&project_id)
                            .into_iter()
                            .flatten()
                            .filter(|tag| !tag.deleted)
                            .filter_map(|tag| tag.order_index),
                    )
                };
                let tag = tag_editor::new_tag(
                    name,
                    tag_editor::validated_color(&color),
                    order_index,
                    user_id,
                );

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        tag_facades::create_tag(repositories.as_ref(), &project_id, &tag, &user_id)
                            .await
                            .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_update_tag(move |project_id, tag_id, name, color| {
                let Some(name) = tag_editor::validated_name(&name) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let Some((project_id, tag_id, user_id)) =
                    tag_write(&weak, &state, &project_id, &tag_id)
                else {
                    return;
                };
                let patch = tag_editor::tag_patch(name, tag_editor::validated_color(&color));

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        tag_facades::update_tag(
                            repositories.as_ref(),
                            &project_id,
                            &tag_id,
                            &patch,
                            &user_id,
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_delete_tag(move |project_id, tag_id| {
                let Some((project_id, tag_id, user_id)) =
                    tag_write(&weak, &state, &project_id, &tag_id)
                else {
                    return;
                };

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        tag_facades::delete_tag(
                            repositories.as_ref(),
                            &project_id,
                            &tag_id,
                            &user_id,
                            &Utc::now(),
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_add_task_tag(move |task_id, name| {
                let Some(name) = tag_editor::validated_name(&name) else {
                    report_error(&weak, "input.validation-failed");
                    return;
                };
                let task_id = task_id.to_string();
                let (project_id, user_id) = {
                    let state = state.lock().expect("shared state poisoned");
                    (state.project_of_task(&task_id), state.current_user)
                };
                let (Some(project_id), Some(user_id)) = (project_id, user_id) else {
                    report_error(&weak, "task.save-failed");
                    return;
                };
                let Ok(task_id) = TaskId::try_from_str(&task_id) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        task_facades::add_task_tag(
                            repositories.as_ref(),
                            &project_id,
                            &task_id,
                            &name,
                            &user_id,
                        )
                        .await
                        .map(|_| ())
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_set_task_tag(move |task_id, tag_id, assigned| {
                let raw_task_id = task_id.to_string();
                let project_id = state
                    .lock()
                    .expect("shared state poisoned")
                    .project_of_task(&raw_task_id);
                let Some(project_id) = project_id else {
                    report_error(&weak, "entity.not-found");
                    return;
                };
                let Ok(task_id) = TaskId::try_from_str(&raw_task_id) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };
                let Ok(tag_id) = TagId::try_from_str(tag_id.as_str()) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };
                let user_id = {
                    let state = state.lock().expect("shared state poisoned");
                    if state.tag_in_project(&project_id, &tag_id).is_none() {
                        drop(state);
                        report_error(&weak, "entity.not-found");
                        return;
                    }
                    state.current_user
                };
                let Some(user_id) = user_id else {
                    report_error(&weak, "permission.denied");
                    return;
                };

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        if assigned {
                            task_facades::add_task_tag_relation(
                                repositories.as_ref(),
                                &project_id,
                                &task_id,
                                &tag_id,
                                &user_id,
                            )
                            .await
                            .map(|_| ())
                        } else {
                            task_facades::remove_task_tag_relation(
                                repositories.as_ref(),
                                &project_id,
                                &task_id,
                                &tag_id,
                            )
                            .await
                            .map(|_| ())
                        }
                    }
                });
            });
        }

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_set_tag_bookmarked(move |project_id, tag_id, bookmarked| {
                let Some((project_id, tag_id, user_id)) =
                    tag_write(&weak, &state, &project_id, &tag_id)
                else {
                    return;
                };

                let existing = {
                    let state = state.lock().expect("shared state poisoned");
                    state
                        .tag_bookmarks
                        .iter()
                        .find(|bookmark| {
                            bookmark.user_id == user_id
                                && bookmark.project_id == project_id
                                && bookmark.tag_id == tag_id
                        })
                        .cloned()
                };

                if bookmarked && existing.is_some() || !bookmarked && existing.is_none() {
                    return;
                }

                let new_bookmark = bookmarked.then(|| {
                    let order_index = {
                        let state = state.lock().expect("shared state poisoned");
                        next_order_index(
                            state
                                .tag_bookmarks
                                .iter()
                                .filter(|bookmark| bookmark.project_id == project_id)
                                .map(|bookmark| bookmark.order_index),
                        )
                    };
                    tag_editor::new_bookmark(user_id, project_id, tag_id, order_index)
                });

                spawn_reloading(&weak, &state, &repositories, &runtime, timezone, {
                    let repositories = Arc::clone(&repositories);
                    move || async move {
                        if let Some(bookmark) = new_bookmark {
                            tag_bookmark_facades::create_bookmark(repositories.as_ref(), &bookmark)
                                .await
                                .map(|_| ())
                        } else if let Some(bookmark) = existing {
                            tag_bookmark_facades::delete_bookmark(
                                repositories.as_ref(),
                                &bookmark.id,
                                &user_id,
                                &project_id,
                                &tag_id,
                            )
                            .await
                            .map(|_| ())
                        } else {
                            Ok(())
                        }
                    }
                });
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
            let weak = window.as_weak();
            let platform = Arc::clone(&self.platform);
            actions.on_open_help(move || {
                if let Err(error) = platform.open_url(HELP_URL) {
                    let ui_error = UiError::from(error);
                    tracing::error!(%ui_error, "failed to open help page");
                    report_error(&weak, ui_error.code());
                }
            });
        }
    }

    /// Scans the installed fonts in the background and publishes the result.
    ///
    /// Enumerating fonts reads the font directories, which is far too slow for
    /// the UI thread, and the appearance settings only need the list once.
    fn load_font_options(&self, window: &AppWindow) {
        if !self
            .platform
            .capabilities()
            .has(Capability::FontEnumeration)
        {
            return;
        }
        let platform = Arc::clone(&self.platform);
        let settings = Arc::clone(&self.settings);
        let weak = window.as_weak();
        let spawned = std::thread::Builder::new()
            .name("flequit-font-scan".to_string())
            .spawn(move || {
                let fonts = match platform.available_fonts() {
                    Ok(fonts) => fonts,
                    Err(PlatformError::Unsupported(_)) => return,
                    Err(error) => {
                        tracing::warn!(%error, "failed to list the installed fonts");
                        return;
                    }
                };
                let posted = weak.upgrade_in_event_loop(move |window| {
                    settings.set_available_fonts(&window, fonts);
                });
                if let Err(error) = posted {
                    tracing::warn!(%error, "could not publish the font list");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "failed to start the font scan");
        }
    }

    fn bind_system_theme(&self, window: &AppWindow) {
        let platform = Arc::clone(&self.platform);
        let weak = window.as_weak();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("flequit-system-theme".to_string())
            .spawn(move || {
                match platform.system_theme() {
                    Ok(theme) => {
                        let posted = weak.upgrade_in_event_loop(move |window| {
                            apply_system_theme(&window, theme)
                        });
                        if posted.is_err() {
                            return;
                        }
                    }
                    Err(PlatformError::Unsupported(_)) => return,
                    Err(error) => tracing::warn!(%error, "failed to detect system theme"),
                }

                let watcher = match platform.subscribe_system_theme() {
                    Ok(watcher) => watcher,
                    Err(PlatformError::Unsupported(_)) => return,
                    Err(error) => {
                        tracing::warn!(%error, "failed to subscribe to system theme changes");
                        return;
                    }
                };

                while !thread_stop.load(Ordering::Relaxed) {
                    match watcher.try_recv() {
                        Ok(Some(theme)) => {
                            let posted = weak.upgrade_in_event_loop(move |window| {
                                apply_system_theme(&window, theme)
                            });
                            if posted.is_err() {
                                break;
                            }
                        }
                        Ok(None) => std::thread::sleep(Duration::from_millis(250)),
                        Err(error) => {
                            tracing::warn!(%error, "system theme watcher stopped");
                            break;
                        }
                    }
                }
            });

        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                tracing::warn!(%error, "failed to start system theme monitor");
                return;
            }
        };
        let monitor = ThemeMonitor {
            stop,
            thread: Some(thread),
        };
        let previous = self
            .theme_monitor
            .lock()
            .expect("theme monitor poisoned")
            .replace(monitor);
        drop(previous);
    }

    /// Subscribes to OS lifecycle transitions and returns the subscription.
    ///
    /// The caller must keep the returned handle alive for as long as the window
    /// exists; dropping it unsubscribes. Returns `None` on platforms that do not
    /// report lifecycle transitions, which is every desktop target.
    #[must_use = "dropping the observer unsubscribes it"]
    pub fn observe_lifecycle(&self, window: &AppWindow) -> Option<Arc<dyn LifecycleObserver>> {
        let observer: Arc<dyn LifecycleObserver> = Arc::new(LifecycleReactor {
            weak: window.as_weak(),
            state: Arc::clone(&self.state),
            repositories: Arc::clone(&self.repositories),
            platform: Arc::clone(&self.platform),
            runtime: self.runtime.clone(),
            timezone: self.timezone,
        });

        match self.platform.subscribe_lifecycle(Arc::clone(&observer)) {
            Ok(()) => Some(observer),
            Err(PlatformError::Unsupported(_)) => None,
            Err(error) => {
                tracing::warn!(%error, "could not subscribe to lifecycle events");
                None
            }
        }
    }

    /// Deletes the task whose deletion is still offering undo.
    ///
    /// Run after the event loop has ended: the undo offer goes with the window,
    /// and the task must not survive the session it was deleted in.
    pub fn flush_pending_deletion(&self) -> impl Future<Output = ()> + Send + 'static {
        deletion::flush(&self.state, &self.repositories, &self.platform)
    }

    /// Loads projects and tasks, then publishes them to the UI.
    ///
    /// Returns immediately; the work happens on the Tokio runtime and the
    /// result is applied on the UI thread.
    pub fn load_initial(&self, window: &AppWindow) {
        window.global::<AppState>().set_loading(true);

        let weak = window.as_weak();
        let repositories = Arc::clone(&self.repositories);
        let platform = Arc::clone(&self.platform);
        let state = Arc::clone(&self.state);
        let timezone = self.timezone;

        self.runtime.spawn(async move {
            let notification_permission = match platform.notification_permission().await {
                Ok(permission) => {
                    publish_notification_permission(&weak, permission);
                    permission
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to read notification permission");
                    PermissionState::NotDetermined
                }
            };
            let account = initialization_facades::load_current_account(repositories.as_ref()).await;
            match account {
                Ok(Some(account)) => {
                    state.lock().expect("shared state poisoned").current_user =
                        Some(account.user_id);
                    let name = account.display_name.unwrap_or_default();
                    let email = account.email.unwrap_or_default();
                    let provider = account.provider;
                    let local = provider == "local";
                    let posted = weak.upgrade_in_event_loop(move |window| {
                        let settings = window.global::<UiSettingsState>();
                        settings.set_account_name(name.into());
                        settings.set_account_email(email.into());
                        settings.set_account_provider(provider.into());
                        settings.set_local_account(local);
                    });
                    if let Err(error) = posted {
                        tracing::error!(%error, "could not publish current account to settings");
                    }
                }
                Ok(None) => tracing::warn!("no current account; writes will be rejected"),
                Err(error) => {
                    let ui_error = UiError::from(error);
                    tracing::error!(%ui_error, "failed to load the current account");
                    report_error(&weak, ui_error.code());
                }
            }

            match user_facades::list_users(repositories.as_ref()).await {
                Ok(users) => query::set_users(
                    &state,
                    users
                        .into_iter()
                        .filter(|user| !user.deleted)
                        .map(|user| UserEntry {
                            id: user.id.as_str(),
                            display_name: user.display_name,
                            handle: user.handle_id,
                        })
                        .collect(),
                ),
                Err(error) => {
                    let ui_error = UiError::from(error);
                    tracing::warn!(%ui_error, "failed to load users; @user search is unavailable");
                }
            }

            if let Some(reminders) = reload_projects(&weak, &state, &repositories, timezone).await
                && notification_permission == PermissionState::Granted
            {
                schedule_reminders(platform.as_ref(), reminders).await;
            }
        });
    }
}

/// Reacts to OS lifecycle transitions on behalf of the ViewModel.
///
/// Held by the caller: [`flequit_platform::LifecycleHub`] keeps only a weak
/// reference, so dropping this unsubscribes.
struct LifecycleReactor<R> {
    weak: Weak<AppWindow>,
    state: Arc<Mutex<SharedState>>,
    repositories: Arc<R>,
    platform: Arc<dyn Platform>,
    runtime: Handle,
    timezone: DisplayTimezone,
}

impl<R> LifecycleObserver for LifecycleReactor<R>
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    fn on_event(&self, event: LifecycleEvent) {
        match event {
            // Every mutation is persisted as it happens, which is what makes the
            // app survive the OS killing it while suspended. The one exception
            // is a deletion still offering undo, so it is settled now.
            LifecycleEvent::Suspend => {
                tracing::info!("suspending");
                if let Some(window) = self.weak.upgrade() {
                    window.global::<AppState>().set_undo_delete_visible(false);
                }
                self.runtime.spawn(deletion::flush(
                    &self.state,
                    &self.repositories,
                    &self.platform,
                ));
            }
            LifecycleEvent::Resume => {
                // The database may have been changed by a share extension, or
                // simply be hours stale, so the tree is re-read rather than
                // trusted.
                let weak = self.weak.clone();
                let state = Arc::clone(&self.state);
                let repositories = Arc::clone(&self.repositories);
                let timezone = self.timezone;
                self.runtime.spawn(async move {
                    reload_projects(&weak, &state, &repositories, timezone).await;
                });
            }
            LifecycleEvent::LowMemory => tracing::warn!("the os asked us to free memory"),
        }
    }
}

fn apply_system_theme(window: &AppWindow, theme: SystemTheme) {
    match theme {
        SystemTheme::Light => window.global::<Theme>().set_system_dark(false),
        SystemTheme::Dark => window.global::<Theme>().set_system_dark(true),
        SystemTheme::Unspecified => {}
    }
}

// -- Free functions ---------------------------------------------------------

type ReminderSpec = (String, String, DateTime<Utc>);

fn reminder_specs(state: &Arc<Mutex<SharedState>>) -> Vec<ReminderSpec> {
    let state = state.lock().expect("shared state poisoned");
    reminder_specs_from_trees(&state.trees)
}

fn reminder_specs_from_trees(trees: &[ProjectTree]) -> Vec<ReminderSpec> {
    /// The reminders of `task` and its live subtasks at every depth.
    fn collect(task: &TaskTree, specs: &mut Vec<ReminderSpec>) {
        if task.deleted {
            return;
        }
        specs.extend(
            task.reminders
                .iter()
                .map(|reminder| (task.id.as_str(), task.title.clone(), *reminder)),
        );
        for child in &task.sub_tasks {
            collect(child, specs);
        }
    }

    let mut specs = Vec::new();
    for project in trees.iter().filter(|project| !project.deleted) {
        let in_lists = project
            .task_lists
            .iter()
            .filter(|list| !list.deleted)
            .flat_map(|list| list.tasks.iter());
        for task in in_lists.chain(project.tasks.iter()) {
            collect(task, &mut specs);
        }
    }
    specs
}

async fn ensure_notification_permission(
    platform: &dyn Platform,
) -> Result<PermissionState, PlatformError> {
    match platform.notification_permission().await? {
        PermissionState::NotDetermined => platform.request_notification_permission().await,
        permission => Ok(permission),
    }
}

async fn schedule_reminder(
    platform: &dyn Platform,
    task_id: &str,
    task_title: &str,
    scheduled_at: DateTime<Utc>,
) -> Result<NotificationId, PlatformError> {
    platform
        .schedule_notification(
            NotificationRequest {
                title: "Flequit".to_string(),
                body: task_title.to_string(),
                payload: Some(task_id.to_string()),
            },
            scheduled_at,
        )
        .await
}

async fn schedule_reminders(platform: &dyn Platform, reminders: Vec<ReminderSpec>) {
    let now = Utc::now();
    for (task_id, title, scheduled_at) in reminders
        .into_iter()
        .filter(|(_, _, scheduled_at)| *scheduled_at > now)
    {
        if let Err(error) = schedule_reminder(platform, &task_id, &title, scheduled_at).await {
            tracing::warn!(%error, %task_id, "failed to restore scheduled reminder");
        }
    }
}

fn permission_key(permission: PermissionState) -> &'static str {
    match permission {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        PermissionState::NotDetermined => "not-determined",
    }
}

fn publish_notification_permission(weak: &Weak<AppWindow>, permission: PermissionState) {
    let posted = weak.upgrade_in_event_loop(move |window| {
        let settings = window.global::<UiSettingsState>();
        settings.set_notification_permission(permission_key(permission).into());
        settings.set_notification_permission_loading(false);
    });
    if let Err(error) = posted {
        tracing::error!(%error, "could not publish notification permission");
    }
}

fn clear_notification_permission_loading(weak: &Weak<AppWindow>) {
    let posted = weak.upgrade_in_event_loop(move |window| {
        window
            .global::<UiSettingsState>()
            .set_notification_permission_loading(false);
    });
    if let Err(error) = posted {
        tracing::error!(%error, "could not clear notification permission state");
    }
}

/// Reloads the project hierarchy and republishes both models.
///
/// Mutations reload rather than patching the cached tree: the tree is the source
/// for task ordering and list membership, and keeping a second copy in sync with
/// storage is the kind of duplication that drifts silently.
/// Reloads the project tree, coalescing requests that arrive while one runs.
///
/// Callers do not need to know whether another reload is in flight: a request
/// that arrives during one is absorbed by it and returns `None` immediately.
/// See [`ReloadGate`] for why.
async fn reload_projects<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    timezone: DisplayTimezone,
) -> Option<Vec<ReminderSpec>>
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    if !state.lock().expect("shared state poisoned").reload.begin() {
        return None;
    }

    loop {
        let reminders = reload_projects_now(weak, state, repositories, timezone).await;
        if !state.lock().expect("shared state poisoned").reload.finish() {
            // The last pass is the one that saw every write in the burst.
            return reminders;
        }
    }
}

/// One pass of the reload: read everything, then publish it to the UI thread.
async fn reload_projects_now<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    timezone: DisplayTimezone,
) -> Option<Vec<ReminderSpec>>
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let loaded = initialization_facades::load_all_project_trees(repositories.as_ref()).await;

    let mut reminders = None;
    let outcome = match loaded {
        Ok(trees) => {
            reminders = Some(reminder_specs_from_trees(&trees));
            let tags_by_project = load_tags(repositories.as_ref(), &trees).await;
            let tag_names = tags_by_project
                .values()
                .flatten()
                .map(|tag| (tag.id, tag.name.clone()))
                .collect();
            let user_id = state.lock().expect("shared state poisoned").current_user;
            let tag_bookmarks = match user_id {
                Some(user_id) => {
                    tag_bookmark_facades::list_bookmarks_by_user(repositories.as_ref(), &user_id)
                        .await
                        .unwrap_or_else(|error| {
                            let ui_error = UiError::from(error);
                            tracing::warn!(%ui_error, "failed to load tag bookmarks");
                            Vec::new()
                        })
                }
                None => Vec::new(),
            };
            Ok((trees, tags_by_project, tag_names, tag_bookmarks))
        }
        Err(error) => {
            let ui_error = UiError::from(error);
            tracing::error!(%ui_error, "failed to load project trees");
            Err(ui_error.code())
        }
    };

    let state = Arc::clone(state);
    let posted = weak.upgrade_in_event_loop(move |window| {
        let app_state = window.global::<AppState>();
        app_state.set_loading(false);

        match outcome {
            Ok((trees, tags_by_project, tag_names, tag_bookmarks)) => {
                {
                    let mut guard = state.lock().expect("shared state poisoned");
                    guard.trees = trees;
                    ordering::normalize(&mut guard.trees);
                    deletion::hide_pending(&mut guard);
                    guard.tags_by_project = tags_by_project;
                    guard.tag_names = tag_names;
                    guard.tag_bookmarks = tag_bookmarks;
                }
                refresh_projects(&window, &state);
                let rewrote = query::follow_data(&window, &state);
                publish_search(&window, &state, timezone, rewrote);
                // A task the list does not show (a subtask under a closed
                // task, or one filtered out) stays open in the pane while it
                // still exists.
                let selected_task_id = app_state.get_selected_task_id();
                if !selected_task_id.is_empty() {
                    if indexed_task_row(&window, &selected_task_id).is_some() {
                        sync_selected_task(&window, &selected_task_id);
                    } else if !show_from_tree(&window, &state, &selected_task_id, timezone) {
                        clear_selection(&window);
                    }
                }
                refresh_tags(&window, &state);
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

    reminders
}

/// Loads every project's tags for display, task association and search.
///
/// Tags are stored per project, but the task list can show several projects at
/// once, so ids are resolved against a single flattened table. A project whose
/// tags cannot be read loses its tag labels rather than the whole reload: the
/// tasks themselves are already in hand and are more useful than an error.
async fn load_tags<R>(repositories: &R, trees: &[ProjectTree]) -> HashMap<ProjectId, Vec<Tag>>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let mut tags_by_project = HashMap::new();

    for tree in trees.iter().filter(|tree| !tree.deleted) {
        match tag_facades::search_tags(repositories, &tree.id, None, None, None).await {
            Ok(mut tags) => {
                tags.retain(|tag| !tag.deleted);
                tags.sort_by(|left, right| {
                    left.order_index
                        .unwrap_or(i32::MAX)
                        .cmp(&right.order_index.unwrap_or(i32::MAX))
                        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                });
                tags_by_project.insert(tree.id, tags);
            }
            Err(error) => {
                let ui_error = UiError::from(error);
                tracing::warn!(%ui_error, project = %tree.id, "failed to load tags");
            }
        }
    }

    tags_by_project
}

fn next_order_index(order_indexes: impl Iterator<Item = i32>) -> i32 {
    order_indexes
        .max()
        .map_or(0, |index| index.saturating_add(1))
}

/// Rebuilds the sidebar project model from the cached tree snapshot.
fn refresh_projects(window: &AppWindow, state_arc: &Arc<Mutex<SharedState>>) {
    let state = state_arc.lock().expect("shared state poisoned");
    let items: Vec<ProjectItem> = state
        .trees
        .iter()
        .filter(|tree| !tree.deleted)
        .filter(|tree| state.show_archived_projects || !tree.is_archived)
        .map(|tree| {
            let expanded = state.expanded_projects.contains(&tree.id.as_str());
            to_project_item(tree, expanded)
        })
        .collect();

    tracing::debug!(projects = items.len(), "publishing sidebar projects");
    window
        .global::<AppState>()
        .set_projects(ModelRc::new(VecModel::from(items)));
    drop(state);
    query::highlight_sidebar(window, state_arc);
}

/// Publishes tags for the selected project and all sidebar bookmarks.
fn refresh_tags(window: &AppWindow, state_arc: &Arc<Mutex<SharedState>>) {
    let state = state_arc.lock().expect("shared state poisoned");
    let selected_task_id = window
        .global::<AppState>()
        .get_selected_task_id()
        .to_string();
    let selected_project = state.project_of_task(&selected_task_id).or_else(|| {
        state
            .trees
            .iter()
            .find(|tree| tree.id.as_str() == state.context_project_id && !tree.deleted)
            .map(|tree| tree.id)
    });
    let assigned_tags: HashSet<TagId> = state
        .task_with_project(&selected_task_id)
        .map(|(_, task)| task.tag_ids.iter().copied().collect())
        .unwrap_or_default();

    let tags = selected_project
        .and_then(|project_id| {
            state.tags_by_project.get(&project_id).map(|tags| {
                tags.iter()
                    .map(|tag| {
                        let bookmarked = state.tag_bookmarks.iter().any(|bookmark| {
                            bookmark.project_id == project_id && bookmark.tag_id == tag.id
                        });
                        to_tag_item(
                            tag,
                            &project_id.as_str(),
                            bookmarked,
                            assigned_tags.contains(&tag.id),
                        )
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();

    let mut bookmarks = state.tag_bookmarks.iter().collect::<Vec<_>>();
    bookmarks.sort_by_key(|bookmark| bookmark.order_index);
    let bookmarks = bookmarks
        .into_iter()
        .filter_map(|bookmark| {
            state
                .tag_in_project(&bookmark.project_id, &bookmark.tag_id)
                .map(|tag| to_bookmarked_tag_item(tag, bookmark))
        })
        .collect::<Vec<_>>();
    let tag_query = state.tag_query.clone();
    drop(state);

    publish_task_tags(window, &tags, &tag_query);
    let app_state = window.global::<AppState>();
    app_state.set_tags(ModelRc::new(VecModel::from(tags)));
    app_state.set_bookmarked_tags(ModelRc::new(VecModel::from(bookmarks)));
    query::highlight_sidebar(window, state_arc);
}

/// Splits the project's tags into the ones on the selected task and the ones
/// suggested for the "add a tag" field.
///
/// The pane lists only the assigned tags, so a tag shown there always means it
/// is set; the rest are reached by typing, narrowed by `tag_suggestion`.
fn publish_task_tags(window: &AppWindow, tags: &[TagItem], query: &str) {
    let (assigned, unassigned): (Vec<TagItem>, Vec<TagItem>) =
        tags.iter().cloned().partition(|tag| tag.assigned);
    let limit = usize::try_from(
        window
            .global::<UiSettingsState>()
            .get_tag_suggestion_count(),
    )
    .unwrap_or(tag_suggestion::DEFAULT_LIMIT)
    .min(tag_suggestion::MAX_LIMIT);
    let suggestions: Vec<TagItem> =
        tag_suggestion::suggest(&unassigned, |tag| tag.name.as_str(), query, limit)
            .into_iter()
            .cloned()
            .collect();

    let app_state = window.global::<AppState>();
    app_state.set_assigned_tags(ModelRc::new(VecModel::from(assigned)));
    app_state.set_tag_suggestions(ModelRc::new(VecModel::from(suggestions)));
}

/// Parses a project id and resolves the author required for a write.
fn project_write(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    raw_id: &str,
) -> Option<(ProjectId, UserId)> {
    let Ok(project_id) = ProjectId::try_from_str(raw_id) else {
        tracing::warn!(
            project_id = raw_id,
            "project action received a malformed id"
        );
        report_error(weak, "input.malformed-id");
        return None;
    };

    let state = state.lock().expect("shared state poisoned");
    if !state
        .trees
        .iter()
        .any(|tree| tree.id == project_id && !tree.deleted)
    {
        tracing::warn!(%project_id, "project action targeted an unknown project");
        drop(state);
        report_error(weak, "entity.not-found");
        return None;
    }
    let Some(user_id) = state.current_user else {
        tracing::error!(%project_id, "cannot update a project: no current user");
        drop(state);
        report_error(weak, "permission.denied");
        return None;
    };

    Some((project_id, user_id))
}

/// Parses a tag id and confirms that it belongs to the requested project.
fn tag_write(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    raw_project_id: &str,
    raw_tag_id: &str,
) -> Option<(ProjectId, TagId, UserId)> {
    let (project_id, user_id) = project_write(weak, state, raw_project_id)?;
    let Ok(tag_id) = TagId::try_from_str(raw_tag_id) else {
        tracing::warn!(tag_id = raw_tag_id, "tag action received a malformed id");
        report_error(weak, "input.malformed-id");
        return None;
    };

    if state
        .lock()
        .expect("shared state poisoned")
        .tag_in_project(&project_id, &tag_id)
        .is_none()
    {
        tracing::warn!(%project_id, %tag_id, "tag action targeted an unknown tag");
        report_error(weak, "entity.not-found");
        return None;
    }

    Some((project_id, tag_id, user_id))
}

/// Parses a task-list id and resolves its project and write author.
fn list_write(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    raw_id: &str,
) -> Option<(ProjectId, TaskListId, UserId)> {
    let Ok(list_id) = TaskListId::try_from_str(raw_id) else {
        tracing::warn!(list_id = raw_id, "task-list action received a malformed id");
        report_error(weak, "input.malformed-id");
        return None;
    };

    let state = state.lock().expect("shared state poisoned");
    let project_id = state.trees.iter().find_map(|tree| {
        (!tree.deleted
            && tree
                .task_lists
                .iter()
                .any(|list| list.id == list_id && !list.deleted))
        .then_some(tree.id)
    });
    let Some(project_id) = project_id else {
        tracing::warn!(%list_id, "task-list action targeted an unknown list");
        drop(state);
        report_error(weak, "entity.not-found");
        return None;
    };
    let Some(user_id) = state.current_user else {
        tracing::error!(%list_id, "cannot update a task list: no current user");
        drop(state);
        report_error(weak, "permission.denied");
        return None;
    };

    Some((project_id, list_id, user_id))
}

/// Runs a tree mutation and reloads the canonical hierarchy on success.
fn spawn_reloading<R, Operation, OperationFuture>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    timezone: DisplayTimezone,
    operation: Operation,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
    Operation: FnOnce() -> OperationFuture + Send + 'static,
    OperationFuture: Future<Output = Result<(), ServiceError>> + Send + 'static,
{
    let Some(window) = weak.upgrade() else {
        return;
    };
    let app_state = window.global::<AppState>();
    app_state.set_error_message(SharedString::default());
    app_state.set_loading(true);
    drop(window);

    let weak = weak.clone();
    let state = Arc::clone(state);
    let repositories = Arc::clone(repositories);
    runtime.spawn(async move {
        match operation().await {
            Ok(()) => {
                reload_projects(&weak, &state, &repositories, timezone).await;
            }
            Err(error) => {
                let ui_error = UiError::from(error);
                tracing::error!(%ui_error, "failed to update the project tree");
                report_error(&weak, ui_error.code());
            }
        }
    });
}

/// How the user's dates are rendered right now.
///
/// The saved timezone wins over the one resolved at startup, so changing it in
/// settings takes effect without a restart; an unset value keeps the fallback.
fn display_settings(window: &AppWindow, fallback: DisplayTimezone) -> DateTimeDisplaySettings {
    let ui_settings = window.global::<UiSettingsState>();
    let timezone = ui_settings.get_timezone();

    DateTimeDisplaySettings {
        timezone: if timezone.is_empty() {
            fallback
        } else {
            DisplayTimezone::from_setting(timezone.as_str())
        },
        format: ui_settings.get_current_datetime_format().to_string(),
    }
}

/// Publishes the next few dates a draft rule would fire on.
///
/// An empty list is a real answer: a rule can be well formed and still have
/// nothing ahead of it, and the dialog says so.
fn publish_recurrence_preview(
    window: &AppWindow,
    rule: Option<&RecurrenceRule>,
    anchor: &DateTime<Utc>,
    display: &DateTimeDisplaySettings,
    requested_count: i32,
) {
    let dates: Vec<SharedString> = rule
        .map(|rule| {
            occurrence::next_occurrences(
                rule,
                anchor,
                display.timezone,
                preview_limit(rule, requested_count),
            )
        })
        .unwrap_or_default()
        .iter()
        .map(|date| SharedString::from(format_due(date, display)))
        .collect();

    window
        .global::<AppState>()
        .set_recurrence_preview(ModelRc::new(VecModel::from(dates)));
}

/// Stores a task's repeat schedule.
///
/// `create_recurrence_rule` saves under the rule's own id, so an edited rule is
/// written back in place and keeps the link the task already has to it. Only a
/// rule the task did not have needs `associate`.
async fn write_recurrence<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    rule: RecurrenceRule,
    associate: bool,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let rule_id = rule.id;
    recurrence_facades::create_recurrence_rule(repositories, project_id, rule, user_id).await?;

    if associate {
        recurrence_facades::create_task_recurrence(repositories, project_id, task_id, &rule_id)
            .await?;
    }
    Ok(())
}

/// Detaches a repeat schedule from a task and removes the rule behind it.
///
/// The link goes first: a rule that outlives its link is invisible, while a
/// task pointing at a rule that is gone would fail to load.
async fn erase_recurrence<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    rule_id: Option<RecurrenceRuleId>,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    recurrence_facades::delete_task_recurrence(repositories, project_id, task_id).await?;

    if let Some(rule_id) = rule_id {
        recurrence_facades::delete_recurrence_rule(repositories, project_id, rule_id.to_string())
            .await?;
    }
    Ok(())
}

/// A new task with default values, at the top level (`parent` is `None`) or
/// under `parent`. A subtask never belongs to a list.
fn new_task(
    project_id: ProjectId,
    list_id: Option<TaskListId>,
    parent: Option<TaskId>,
    title: String,
    order_index: i32,
    user_id: UserId,
) -> Task {
    let now = Utc::now();
    Task {
        id: TaskId::new(),
        project_id,
        list_id: if parent.is_some() { None } else { list_id },
        parent_task_id: parent,
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
        reminders: Vec::new(),
        order_index,
        is_archived: false,
        assigned_user_ids: Vec::new(),
        tag_ids: Vec::new(),
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    }
}

fn priority_value(priority: TaskPriority) -> i32 {
    match priority {
        TaskPriority::None => 0,
        TaskPriority::Low => 1,
        TaskPriority::Medium => 3,
        TaskPriority::High => 6,
    }
}

/// Waits for a pause in task text input, then persists only the newest value.
#[allow(clippy::too_many_arguments)]
fn spawn_debounced_task_patch<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    debouncer: &Arc<EditDebouncer<TaskRowSnapshot>>,
    task_id: String,
    patch: PartialTask,
    snapshot: TaskRowSnapshot,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let revision = debouncer.schedule(task_id.clone(), snapshot);
    let weak = weak.clone();
    let state = Arc::clone(state);
    let repositories = Arc::clone(repositories);
    let debouncer = Arc::clone(debouncer);
    let task_runtime = runtime.clone();
    runtime.spawn(async move {
        tokio::time::sleep(TEXT_SAVE_DEBOUNCE).await;
        let Some(snapshot) = debouncer.take_if_latest(&task_id, revision) else {
            return;
        };
        spawn_task_patch(
            &weak,
            &state,
            &repositories,
            &task_runtime,
            task_id,
            patch,
            snapshot,
        );
    });
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

        if let Err(error) = result {
            let ui_error = UiError::from(error);
            tracing::error!(%ui_error, %task_id, "failed to update task");
            rollback_task_row(&weak, snapshot);
            report_error(&weak, ui_error.code());
        }
    });
}

/// The task a repeating task hands its series on to when it is completed.
///
/// Read from the cached tree on the UI thread, since the display timezone
/// decides which calendar day "next" falls on. `None` for a task that does not
/// repeat or whose series has run out.
fn plan_next_occurrence(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    task_id: &SharedString,
    fallback: DisplayTimezone,
) -> Option<Successor> {
    let display = display_settings(window, fallback);
    let state = state.lock().expect("shared state poisoned");
    let user_id = state.current_user?;
    let (project, task) = state.task_with_project(task_id.as_str())?;
    // The successor joins the end of where the completed task lives.
    let siblings: &[TaskTree] = match (task.parent_task_id, task.list_id) {
        (Some(parent), _) => project
            .find_task(&parent)
            .map_or(&[], |parent| parent.sub_tasks.as_slice()),
        (None, Some(list_id)) => project
            .task_lists
            .iter()
            .find(|list| list.id == list_id)
            .map_or(&[], |list| list.tasks.as_slice()),
        (None, None) => &project.tasks,
    };
    let order_index = next_order_index(siblings.iter().map(|sibling| sibling.order_index));
    let created = recurrence::next_task(task, Utc::now(), display.timezone, order_index, user_id)?;
    Some(Successor {
        created,
        rule_before: task.recurrence_rule.clone()?,
    })
}

/// The successor to withdraw when a completion is undone, if it is still
/// exactly as it was created.
///
/// The record is dropped either way: a successor the user has made their own
/// stays for good, even if the completion is later redone and undone again.
fn take_untouched_successor(
    state: &Arc<Mutex<SharedState>>,
    task_id: &SharedString,
) -> Option<Successor> {
    let mut state = state.lock().expect("shared state poisoned");
    let completed_id = TaskId::try_from_str(task_id.as_str()).ok()?;
    let successor = state.successors.remove(&completed_id)?;
    let (_, current) = state.task_with_project(&successor.created.task.id.to_string())?;
    recurrence::is_untouched(&successor.created, current).then_some(successor)
}

/// Persists a status change together with what it does to a repeating task's
/// series.
///
/// The steps run in order on one background task: the series only moves once
/// the status itself was saved, and the reload that shows the result must not
/// race the status write.
#[allow(clippy::too_many_arguments)]
fn spawn_completion_patch<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    platform: &Arc<dyn Platform>,
    runtime: &Handle,
    timezone: DisplayTimezone,
    task_id: String,
    patch: PartialTask,
    snapshot: TaskRowSnapshot,
    step: Option<SeriesStep>,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let Some(step) = step else {
        spawn_task_patch(weak, state, repositories, runtime, task_id, patch, snapshot);
        return;
    };

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
    let state = Arc::clone(state);
    let repositories = Arc::clone(repositories);
    let platform = Arc::clone(platform);
    runtime.spawn(async move {
        let result = task_facades::update_task(
            repositories.as_ref(),
            &project_id,
            &parsed_id,
            &patch,
            &user_id,
        )
        .await;
        if let Err(error) = result {
            let ui_error = UiError::from(error);
            tracing::error!(%ui_error, %task_id, "failed to update task");
            rollback_task_row(&weak, snapshot);
            report_error(&weak, ui_error.code());
            return;
        }

        match step {
            SeriesStep::HandOver(successor) => {
                let result =
                    hand_over_recurrence(repositories.as_ref(), &project_id, &parsed_id, &successor, &user_id)
                        .await;
                match result {
                    Ok(()) => {
                        let next = &successor.created.task;
                        let reminders = next
                            .reminders
                            .iter()
                            .map(|at| (next.id.to_string(), next.title.clone(), *at))
                            .collect();
                        schedule_reminders(platform.as_ref(), reminders).await;
                        state
                            .lock()
                            .expect("shared state poisoned")
                            .successors
                            .insert(parsed_id, successor);
                    }
                    Err(error) => {
                        let ui_error = UiError::from(error);
                        tracing::error!(%ui_error, %task_id, "failed to create the next recurring task");
                        report_error(&weak, ui_error.code());
                    }
                }
            }
            SeriesStep::Withdraw(successor) => {
                let result = withdraw_successor(
                    repositories.as_ref(),
                    &project_id,
                    &parsed_id,
                    &successor,
                    &user_id,
                )
                .await;
                match result {
                    Ok(()) => {
                        let next_id = successor.created.task.id.to_string();
                        for reminder in &successor.created.task.reminders {
                            let id = NotificationId::scheduled(&next_id, reminder);
                            if let Err(error) = platform.cancel_notification(&id).await {
                                tracing::warn!(%error, %next_id, "failed to cancel withdrawn task reminder");
                            }
                        }
                        if let Err(error) = weak.upgrade_in_event_loop(move |window| {
                            if window.global::<AppState>().get_selected_task_id() == next_id {
                                clear_selection(&window);
                            }
                        }) {
                            tracing::error!(%error, "could not clear the withdrawn task selection");
                        }
                    }
                    Err(error) => {
                        let ui_error = UiError::from(error);
                        tracing::error!(%ui_error, %task_id, "failed to withdraw the next recurring task");
                        report_error(&weak, ui_error.code());
                    }
                }
            }
        }
        reload_projects(&weak, &state, &repositories, timezone).await;
    });
}

/// Creates the next task in a series, with its own copies of the completed
/// task's subtasks at every depth, and moves the rule over to it.
///
/// The completed task gives up its link, so the rule is never shared: editing
/// or clearing the schedule on one task cannot reach the other. The copied
/// subtasks carry no rule of their own — only the task repeats.
async fn hand_over_recurrence<R>(
    repositories: &R,
    project_id: &ProjectId,
    completed_id: &TaskId,
    successor: &Successor,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let next = &successor.created.task;
    let Some(rule) = next.recurrence_rule.clone() else {
        return Ok(());
    };
    task_facades::create_task(repositories, project_id, next, user_id).await?;
    for tag_id in &next.tag_ids {
        task_facades::add_task_tag_relation(repositories, project_id, &next.id, tag_id, user_id)
            .await?;
    }
    // Parents come before their children, so each parent exists when its
    // children are created.
    for sub_task in &successor.created.sub_tasks {
        task_facades::create_task(repositories, project_id, sub_task, user_id).await?;
        for tag_id in &sub_task.tag_ids {
            task_facades::add_task_tag_relation(
                repositories,
                project_id,
                &sub_task.id,
                tag_id,
                user_id,
            )
            .await?;
        }
    }
    recurrence_facades::delete_task_recurrence(repositories, project_id, completed_id).await?;
    write_recurrence(repositories, project_id, &next.id, rule, true, user_id).await
}

/// Reverses `hand_over_recurrence`: deletes the successor and gives the rule
/// back to the task it came from, as that task had it.
///
/// The successor's link goes first, so a failure part way leaves at worst a
/// task without a schedule, never two tasks sharing one rule. Deleting the
/// task takes its copied subtasks with it, in the same transaction.
async fn withdraw_successor<R>(
    repositories: &R,
    project_id: &ProjectId,
    completed_id: &TaskId,
    successor: &Successor,
    user_id: &UserId,
) -> Result<(), ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let next_id = &successor.created.task.id;
    recurrence_facades::delete_task_recurrence(repositories, project_id, next_id).await?;
    task_facades::delete_task(repositories, project_id, next_id, user_id, &Utc::now()).await?;
    write_recurrence(
        repositories,
        project_id,
        completed_id,
        successor.rule_before.clone(),
        true,
        user_id,
    )
    .await
}

/// Persists a batch of `order_index` (and list) changes.
///
/// Applied one task at a time because the repository has no bulk write. The
/// first failure stops the batch and reloads: a half-applied order is worse
/// than a stale one, and only storage can say what the order really is.
fn spawn_order_updates<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    project_id: ProjectId,
    updates: Vec<(TaskId, PartialTask)>,
    timezone: DisplayTimezone,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    if updates.is_empty() {
        return;
    }

    let Some(user_id) = state.lock().expect("shared state poisoned").current_user else {
        tracing::error!("cannot reorder tasks: no current user");
        report_error(weak, "task.save-failed");
        return;
    };

    let weak = weak.clone();
    let state = Arc::clone(state);
    let repositories = Arc::clone(repositories);
    runtime.spawn(async move {
        for (task_id, patch) in updates {
            let result = task_facades::update_task(
                repositories.as_ref(),
                &project_id,
                &task_id,
                &patch,
                &user_id,
            )
            .await;

            if let Err(error) = result {
                let ui_error = UiError::from(error);
                tracing::error!(%ui_error, %task_id, "failed to save the task order");
                report_error(&weak, ui_error.code());
                reload_projects(&weak, &state, &repositories, timezone).await;
                return;
            }
        }
    });
}

/// Completes (or reopens) a subtask that only the open task's pane lists: its
/// own row is hidden under a closed task or by the filter, and it is not the
/// task in the pane.
///
/// The pane's summary is the only place it shows, so that is updated at once;
/// the reload after the write puts the list and the pane back in step with
/// storage either way.
fn toggle_subtask_in_pane<R>(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    timezone: DisplayTimezone,
    task_id: &SharedString,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let app_state = window.global::<AppState>();
    let mut parent = app_state.get_selected_task();
    let subtasks = parent.subtasks.clone();
    let Some(index) = subtasks.iter().position(|sub| sub.id == *task_id) else {
        return;
    };
    let Some(mut summary) = subtasks.row_data(index) else {
        return;
    };
    let completed = !summary.completed;
    summary.completed = completed;
    summary.overdue = summary.overdue && !completed;
    subtasks.set_row_data(index, summary);
    parent.subtask_done_count =
        i32::try_from(subtasks.iter().filter(|sub| sub.completed).count()).unwrap_or(i32::MAX);
    app_state.set_selected_task(parent);

    let (project_id, user_id) = {
        let state = state.lock().expect("shared state poisoned");
        (state.project_of_task(task_id), state.current_user)
    };
    let (Some(project_id), Some(user_id), Ok(parsed_id)) =
        (project_id, user_id, TaskId::try_from_str(task_id.as_str()))
    else {
        tracing::error!(%task_id, "cannot update a subtask: unknown project or user");
        report_error(&window.as_weak(), "task.save-failed");
        return;
    };
    let patch = PartialTask {
        status: Some(if completed {
            DomainStatus::Completed
        } else {
            DomainStatus::NotStarted
        }),
        ..Default::default()
    };

    let weak = window.as_weak();
    let state = Arc::clone(state);
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
        if let Err(error) = result {
            let ui_error = UiError::from(error);
            tracing::error!(%ui_error, %parsed_id, "failed to update subtask");
            report_error(&weak, ui_error.code());
        }
        reload_projects(&weak, &state, &repositories, timezone).await;
    });
}

/// Moves a visible top-level task to the position `to` picks among the visible
/// top-level rows, and persists the new stored order.
///
/// The stored order is only what the list shows while sorting is manual;
/// rewriting it from a derived order would move rows the user never saw move.
fn move_top_level_task<R>(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    timezone: DisplayTimezone,
    task_id: &SharedString,
    to: impl FnOnce(usize) -> Option<usize>,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    if state.lock().expect("shared state poisoned").task_sort != TaskSort::Manual {
        tracing::debug!(%task_id, "ignoring a reorder while a sort is applied");
        return;
    }
    let rows: Vec<TaskItem> = window.global::<AppState>().get_tasks().iter().collect();
    let Some((previous, next)) = ordering::neighbours_after_move(&rows, task_id.as_str(), to)
    else {
        return;
    };

    let updates = {
        let mut guard = state.lock().expect("shared state poisoned");
        ordering::reorder_task(&mut guard.trees, task_id, previous, next)
    };
    let Some(updates) = updates else {
        tracing::warn!(%task_id, "cannot reorder a task that is not loaded");
        return;
    };
    apply_move(
        &window.as_weak(),
        state,
        repositories,
        runtime,
        timezone,
        updates,
    );
}

/// Shows a move already made in the cached tree, then persists it.
fn apply_move<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    timezone: DisplayTimezone,
    (project_id, updates): ordering::tree::OrderUpdates,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let Some(window) = weak.upgrade() else { return };
    // The task may leave the visible set, and lists change size.
    refresh_projects(&window, state);
    refresh_tasks(&window, state, timezone);
    let selected = window.global::<AppState>().get_selected_task_id();
    if !selected.is_empty() && task_row(&window, &selected).is_none() {
        clear_selection(&window);
    }

    spawn_order_updates(
        weak,
        state,
        repositories,
        runtime,
        project_id,
        updates,
        timezone,
    );
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

/// Reads the current UI row and its visible index for a task.
fn indexed_task_row(window: &AppWindow, task_id: &SharedString) -> Option<(usize, TaskItem)> {
    window
        .global::<AppState>()
        .get_tasks()
        .iter()
        .enumerate()
        .find(|(_, item)| item.id == task_id)
}

/// Reads a task as the UI shows it: its row, or the detail pane's copy when
/// the list does not show it (a subtask under a closed task, or one the
/// search leaves out).
fn task_row(window: &AppWindow, task_id: &SharedString) -> Option<TaskItem> {
    indexed_task_row(window, task_id)
        .map(|(_, item)| item)
        .or_else(|| {
            let app_state = window.global::<AppState>();
            (app_state.get_has_selected_task() && app_state.get_selected_task_id() == *task_id)
                .then(|| app_state.get_selected_task())
        })
}

/// Opens a task at any depth: the tasks above it open so the list shows where
/// it is, and it is selected. A task the search leaves out is still shown in
/// the detail pane.
fn open_task(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    task_id: &SharedString,
    timezone: DisplayTimezone,
) {
    let opened = {
        let mut guard = state.lock().expect("shared state poisoned");
        let ancestors: Vec<String> = guard
            .ancestors_of(task_id)
            .iter()
            .map(|ancestor| ancestor.id.as_str())
            .collect();
        let mut opened = false;
        for ancestor in ancestors {
            if !guard.task_ui.is_expanded(&ancestor) {
                guard.task_ui.expand(&ancestor);
                opened = true;
            }
        }
        opened
    };
    if opened {
        refresh_tasks(window, state, timezone);
    }

    if indexed_task_row(window, task_id).is_some() {
        select_task(window, task_id);
    } else if show_from_tree(window, state, task_id, timezone) {
        window.global::<AppState>().set_active_pane(Pane::Detail);
    } else {
        tracing::warn!(%task_id, "selection target is not loaded");
        clear_selection(window);
    }
}

/// Fills the detail pane with a task straight from the cached tree, for a task
/// the list does not show. Returns whether the task was found.
fn show_from_tree(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    task_id: &SharedString,
    timezone: DisplayTimezone,
) -> bool {
    let display = display_settings(window, timezone);
    let item = {
        let guard = state.lock().expect("shared state poisoned");
        query::task_item_from_tree(&guard, task_id, &display)
    };
    let Some(item) = item else {
        return false;
    };
    let app_state = window.global::<AppState>();
    app_state.set_selected_task_id(task_id.clone());
    app_state.set_selected_task(item);
    app_state.set_has_selected_task(true);
    true
}

/// Selects a task and mirrors the resolved row into `AppState`.
///
/// The view cannot search the model itself — Slint bindings are declarative
/// expressions — so the lookup happens here and the result is published as a
/// plain property.
fn select_task(window: &AppWindow, task_id: &SharedString) {
    let app_state = window.global::<AppState>();

    match indexed_task_row(window, task_id) {
        Some((index, item)) => {
            app_state.set_selected_task_id(task_id.clone());
            app_state.set_selected_task(item);
            app_state.set_has_selected_task(true);
            if let Ok(index) = i32::try_from(index) {
                app_state.set_scroll_to_index(index);
            }
            // Compact shows one pane at a time; move to the detail view.
            app_state.set_active_pane(Pane::Detail);
        }
        None => {
            tracing::warn!(%task_id, "selection target is not in the visible model");
            clear_selection(window);
        }
    }
}

/// Moves the task-list selection without leaving the list pane in compact mode.
fn select_task_at_offset(window: &AppWindow, delta: i32) -> bool {
    let app_state = window.global::<AppState>();
    let tasks = app_state.get_tasks();
    let current = app_state.get_selected_task_id();
    let selected = tasks.iter().position(|task| task.id == current);
    let Some(index) = task_selection_index(tasks.row_count(), selected, delta) else {
        return false;
    };
    let Some(task) = tasks.row_data(index) else {
        return false;
    };

    select_task(window, &task.id);
    app_state.set_active_pane(Pane::List);
    true
}

/// Selects the first or last visible task without leaving the list pane.
fn select_task_boundary(window: &AppWindow, first: bool) -> bool {
    let app_state = window.global::<AppState>();
    let tasks = app_state.get_tasks();
    let Some(index) = task_boundary_index(tasks.row_count(), first) else {
        return false;
    };
    let Some(task) = tasks.row_data(index) else {
        return false;
    };

    select_task(window, &task.id);
    app_state.set_active_pane(Pane::List);
    true
}

fn task_selection_index(row_count: usize, selected: Option<usize>, delta: i32) -> Option<usize> {
    if row_count == 0 {
        return None;
    }
    let Some(current) = selected else {
        return Some(if delta < 0 { row_count - 1 } else { 0 });
    };
    Some(
        current
            .saturating_add_signed(delta as isize)
            .min(row_count - 1),
    )
}

fn task_boundary_index(row_count: usize, first: bool) -> Option<usize> {
    (row_count > 0).then_some(if first { 0 } else { row_count - 1 })
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
    app_state.set_has_selected_task(false);
    app_state.set_active_pane(Pane::List);
}

/// Applies a change to a single task without rebuilding the model.
///
/// Rebuilding would reset scroll position and discard row identity, so
/// single-value edits always go through here. A task without a row is only in
/// the detail pane, so the change goes there. Either way the summary its
/// parent shows of it follows.
fn update_task_row(window: &AppWindow, task_id: &str, edit: impl FnOnce(&mut TaskItem)) {
    let app_state = window.global::<AppState>();
    let model = app_state.get_tasks();
    match model.iter().position(|item| item.id == task_id) {
        Some(index) => {
            let Some(mut item) = model.row_data(index) else {
                return;
            };
            edit(&mut item);
            model.set_row_data(index, item);
        }
        None if app_state.get_has_selected_task()
            && app_state.get_selected_task_id() == task_id =>
        {
            let mut item = app_state.get_selected_task();
            edit(&mut item);
            app_state.set_selected_task(item);
        }
        None => return,
    }
    sync_subtask_summary(window, task_id);
}

/// Keeps the parent's list of subtasks in step after `child_id` changed in
/// place: its row's progress, and the list in the pane when the parent is open.
fn sync_subtask_summary(window: &AppWindow, child_id: &str) {
    let Some(child) = task_row(window, &SharedString::from(child_id)) else {
        return;
    };
    if child.parent_id.is_empty() {
        return;
    }
    let refresh = |parent: &mut TaskItem| {
        let subtasks = parent.subtasks.clone();
        let Some(index) = subtasks.iter().position(|sub| sub.id == child_id) else {
            return false;
        };
        if let Some(mut sub) = subtasks.row_data(index) {
            sub.title = child.title.clone();
            sub.completed = child.completed;
            sub.due_label = child.due_label.clone();
            sub.has_due = child.has_due;
            sub.overdue = child.overdue;
            subtasks.set_row_data(index, sub);
        }
        parent.subtask_done_count =
            i32::try_from(subtasks.iter().filter(|sub| sub.completed).count()).unwrap_or(i32::MAX);
        true
    };

    let app_state = window.global::<AppState>();
    let model = app_state.get_tasks();
    if let Some(index) = model.iter().position(|item| item.id == child.parent_id)
        && let Some(mut parent) = model.row_data(index)
        && refresh(&mut parent)
    {
        model.set_row_data(index, parent);
    }
    if app_state.get_selected_task_id() == child.parent_id {
        let mut parent = app_state.get_selected_task();
        if refresh(&mut parent) {
            app_state.set_selected_task(parent);
        }
    }
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
    fn edit_debouncer_keeps_first_snapshot_and_only_releases_latest_revision() {
        let debouncer = EditDebouncer::default();

        let first = debouncer.schedule("task-1".to_string(), "before".to_string());
        let latest = debouncer.schedule("task-1".to_string(), "intermediate".to_string());

        assert_eq!(debouncer.take_if_latest("task-1", first), None);
        assert_eq!(
            debouncer.take_if_latest("task-1", latest),
            Some("before".to_string())
        );
        assert_eq!(debouncer.take_if_latest("task-1", latest), None);
    }

    #[test]
    fn keyboard_task_navigation_stays_in_bounds() {
        assert_eq!(task_selection_index(0, None, 1), None);
        assert_eq!(task_selection_index(3, None, 1), Some(0));
        assert_eq!(task_selection_index(3, None, -1), Some(2));
        assert_eq!(task_selection_index(3, Some(0), -1), Some(0));
        assert_eq!(task_selection_index(3, Some(1), 1), Some(2));
        assert_eq!(task_selection_index(3, Some(2), 1), Some(2));
        assert_eq!(task_boundary_index(0, true), None);
        assert_eq!(task_boundary_index(3, true), Some(0));
        assert_eq!(task_boundary_index(3, false), Some(2));
    }

    #[test]
    fn a_new_subtask_belongs_to_its_parent_and_no_list() {
        let parent = TaskId::new();
        let user_id = UserId::new();

        let subtask = new_task(
            ProjectId::new(),
            Some(TaskListId::new()),
            Some(parent),
            "Pick up bread".to_string(),
            3,
            user_id,
        );

        assert_eq!(subtask.parent_task_id, Some(parent));
        assert_eq!(subtask.list_id, None);
        assert_eq!(subtask.title, "Pick up bread");
        assert_eq!(subtask.order_index, 3);
        assert_eq!(subtask.status, DomainStatus::NotStarted);
        assert_eq!(subtask.updated_by, user_id);
    }

    #[test]
    fn a_new_top_level_task_keeps_its_place() {
        let list_id = TaskListId::new();
        let in_list = new_task(
            ProjectId::new(),
            Some(list_id),
            None,
            "a".into(),
            0,
            UserId::new(),
        );
        let loose = new_task(ProjectId::new(), None, None, "b".into(), 0, UserId::new());

        assert_eq!(in_list.list_id, Some(list_id));
        assert_eq!(in_list.parent_task_id, None);
        assert_eq!(loose.list_id, None);
    }

    #[test]
    fn a_new_tree_item_follows_the_highest_existing_order() {
        assert_eq!(next_order_index([2, 7, 4].into_iter()), 8);
        assert_eq!(next_order_index([].into_iter()), 0);
        assert_eq!(next_order_index([i32::MAX].into_iter()), i32::MAX);
    }

    #[test]
    fn ui_priorities_map_to_domain_bucket_values() {
        assert_eq!(priority_value(TaskPriority::None), 0);
        assert_eq!(priority_value(TaskPriority::Low), 1);
        assert_eq!(priority_value(TaskPriority::Medium), 3);
        assert_eq!(priority_value(TaskPriority::High), 6);
    }

    fn tag(name: &str, deleted: bool) -> Tag {
        let now = Utc::now();
        Tag {
            id: TagId::new(),
            name: name.to_string(),
            color: None,
            order_index: Some(0),
            created_at: now,
            updated_at: now,
            deleted,
            updated_by: UserId::new(),
        }
    }

    fn task_with_tags(tag_ids: Vec<TagId>) -> TaskTree {
        let now = Utc::now();
        TaskTree {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            list_id: Some(TaskListId::new()),
            parent_task_id: None,
            title: "Buy milk".to_string(),
            description: None,
            status: DomainStatus::NotStarted,
            priority: 0,
            plan_start_date: None,
            plan_end_date: None,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: None,
            reminders: Vec::new(),
            assigned_user_ids: Vec::new(),
            order_index: 0,
            is_archived: false,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
            sub_tasks: Vec::new(),
            tag_ids,
        }
    }

    #[test]
    fn tag_ids_the_cache_does_not_know_are_dropped_rather_than_shown() {
        let known = tag("home", false);
        let unknown = TagId::new();
        let mut state = SharedState::default();
        state.tag_names.insert(known.id, known.name.clone());

        let task = task_with_tags(vec![known.id, unknown]);

        // The cache is filled by the same load that produces the rows, so a
        // missing name means the id is stale, not that the label is blank.
        assert_eq!(state.tag_names_of(&task), ["home"]);
    }

    #[test]
    fn a_tag_is_only_found_in_the_project_that_owns_it() {
        let owner = ProjectId::new();
        let other = ProjectId::new();
        let home = tag("home", false);
        let mut state = SharedState::default();
        state.tags_by_project.insert(owner, vec![home.clone()]);

        assert_eq!(
            state.tag_in_project(&owner, &home.id).map(|tag| tag.id),
            Some(home.id)
        );
        assert!(state.tag_in_project(&other, &home.id).is_none());
    }

    #[test]
    fn a_deleted_tag_is_not_writable() {
        let project_id = ProjectId::new();
        let gone = tag("home", true);
        let mut state = SharedState::default();
        state.tags_by_project.insert(project_id, vec![gone.clone()]);

        assert!(state.tag_in_project(&project_id, &gone.id).is_none());
    }

    pub(super) fn task_named(title: &str) -> TaskTree {
        TaskTree {
            title: title.to_string(),
            ..task_with_tags(Vec::new())
        }
    }

    pub(super) fn list_named(name: &str, tasks: Vec<TaskTree>) -> TaskListTree {
        let now = Utc::now();
        TaskListTree {
            id: TaskListId::new(),
            project_id: ProjectId::new(),
            name: name.to_string(),
            description: None,
            color: None,
            order_index: 0,
            is_archived: false,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
            tasks,
        }
    }

    pub(super) fn project_named(name: &str, task_lists: Vec<TaskListTree>) -> ProjectTree {
        let now = Utc::now();
        ProjectTree {
            id: ProjectId::new(),
            name: name.to_string(),
            description: None,
            color: None,
            order_index: 0,
            is_archived: false,
            status: None,
            owner_id: None,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
            task_lists,
            tasks: Vec::new(),
        }
    }

    #[test]
    fn every_live_task_is_a_candidate_and_hidden_ones_are_not() {
        let mut project = project_named(
            "Work",
            vec![
                list_named(
                    "Inbox",
                    vec![
                        task_named("Buy milk"),
                        TaskTree {
                            deleted: true,
                            ..task_named("Deleted")
                        },
                        TaskTree {
                            is_archived: true,
                            ..task_named("Archived")
                        },
                    ],
                ),
                list_named("Later", vec![task_named("Someday")]),
            ],
        );
        project.task_lists[1].is_archived = true;
        let mut archived = project_named(
            "Old",
            vec![list_named("Errands", vec![task_named("Shelved")])],
        );
        archived.is_archived = true;
        let mut other = project_named(
            "Home",
            vec![list_named("Errands", vec![task_named("Milk")])],
        );
        other.tasks.push(TaskTree {
            list_id: None,
            ..task_named("Loose")
        });

        let mut state = SharedState {
            trees: vec![project, archived, other],
            ..SharedState::default()
        };

        // The query decides what is listed; only deleted or archived rows are
        // out of reach, and archived projects only while they are hidden.
        let titles: Vec<&str> = state
            .live_tasks()
            .map(|(_, _, task)| task.title.as_str())
            .collect();
        assert_eq!(titles, ["Buy milk", "Milk", "Loose"]);

        state.show_archived_projects = true;
        let titles: Vec<&str> = state
            .live_tasks()
            .map(|(_, _, task)| task.title.as_str())
            .collect();
        assert_eq!(titles, ["Buy milk", "Shelved", "Milk", "Loose"]);
    }

    #[test]
    fn reminders_are_only_collected_from_rows_that_still_exist() {
        let due = Utc::now();
        let child = TaskTree {
            reminders: vec![due],
            ..task_named("Find the card")
        };
        let child_id = child.id.as_str();
        let gone_child = TaskTree {
            deleted: true,
            reminders: vec![due],
            ..task_named("Never mind")
        };
        let live = TaskTree {
            reminders: vec![due],
            sub_tasks: vec![child, gone_child],
            ..task_named("Call the dentist")
        };
        let live_id = live.id.as_str();
        let removed = TaskTree {
            deleted: true,
            reminders: vec![due],
            ..task_named("Cancelled")
        };
        let hidden_list = TaskListTree {
            deleted: true,
            ..list_named(
                "Gone",
                vec![TaskTree {
                    reminders: vec![due],
                    ..task_named("In a deleted list")
                }],
            )
        };
        let trees = vec![project_named(
            "Work",
            vec![list_named("Inbox", vec![live, removed]), hidden_list],
        )];

        let specs = reminder_specs_from_trees(&trees);

        assert_eq!(
            specs,
            vec![
                (live_id, "Call the dentist".to_string(), due),
                (child_id, "Find the card".to_string(), due),
            ],
            "a reminder on a row the user deleted must not still fire"
        );
    }
}
