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
    initialization_facades, project_facades, recurrence_facades, subtask_facades,
    tag_bookmark_facades, tag_facades, task_facades, task_list_facades, user_facades,
};
use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::subtask::{PartialSubTask, SubTask};
use flequit_model::models::task_projects::tag::Tag;
use flequit_model::models::task_projects::task::{PartialTask, Task, TaskTree};
use flequit_model::models::task_projects::task_list::TaskListTree;
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::types::id_types::{
    ProjectId, RecurrenceRuleId, SubTaskId, TagId, TaskId, TaskListId, UserId,
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
    ProjectItem, SettingsState as UiSettingsState, SubTaskItem, TaskItem, TaskPriority, TaskSort,
    TaskStatus, Theme,
};
use crate::viewmodels::TaskListUiViewModel;
use crate::viewmodels::ordering;
use crate::viewmodels::project_editor;
use crate::viewmodels::recurrence::{occurrence, preview_limit, rule_from_state};
use crate::viewmodels::reload_gate::ReloadGate;
use crate::viewmodels::search::{NameIndex, QueryEdit as SearchEdit, SearchSession, UserEntry};
use crate::viewmodels::settings::{SearchMemory, SettingsStore, SettingsViewModel, UserSettings};

mod query;
use crate::viewmodels::tag_editor;
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
    /// Tag names by id, for every loaded project.
    ///
    /// Tasks carry only tag ids, and both the row labels and `#tag` searches
    /// need names, so they are resolved once per load instead of per row.
    tag_names: HashMap<TagId, String>,
    /// Full tag rows grouped by their owning project.
    tags_by_project: HashMap<ProjectId, Vec<Tag>>,
    /// Sidebar pins belonging to the current user.
    tag_bookmarks: Vec<TagBookmark>,
    /// Author recorded on every write. Resolved from the current account.
    current_user: Option<UserId>,
    /// Keeps a burst of edits from running one full reload each.
    reload: ReloadGate,
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

    /// Locates a task and the project that owns it, using the cached tree.
    fn task_with_project(&self, task_id: &str) -> Option<(&ProjectTree, &TaskTree)> {
        self.trees.iter().find_map(|tree| {
            tree.task_lists
                .iter()
                .flat_map(|list| list.tasks.iter())
                .find(|task| task.id.as_str() == task_id)
                .map(|task| (tree, task))
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

    /// Every live task, with the project and list that own it.
    ///
    /// The search query decides which of these are listed. Archived projects
    /// take part only while they are shown in the sidebar.
    fn live_tasks(&self) -> impl Iterator<Item = (&ProjectTree, &TaskListTree, &TaskTree)> {
        self.live_lists()
            .flat_map(|(tree, list)| list.tasks.iter().map(move |task| (tree, list, task)))
            .filter(|(_, _, task)| !task.deleted && !task.is_archived)
    }

    /// Every live task list, with its project.
    fn live_lists(&self) -> impl Iterator<Item = (&ProjectTree, &TaskListTree)> {
        self.trees
            .iter()
            .filter(|tree| !tree.deleted && (self.show_archived_projects || !tree.is_archived))
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

/// The subtask counterpart of `TaskRowSnapshot`.
struct SubTaskRowSnapshot {
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
}

impl SubTaskRowSnapshot {
    fn capture(item: &SubTaskItem) -> Self {
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
        }
    }

    fn restore(self, item: &mut SubTaskItem) {
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

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            actions.on_select_task(move |task_id| {
                if let Some(window) = weak.upgrade() {
                    // Picking a task leaves whatever subtask was open behind.
                    clear_subtask_selection(&window);
                    select_task(&window, &task_id);
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
                    refresh_tags(&window, &state);
                }

                let app_state = window.global::<AppState>();
                match subtask_row(&window, &subtask_id) {
                    Some(item) => {
                        app_state.set_selected_subtask_id(subtask_id);
                        app_state.set_selected_subtask(item);
                        app_state.set_has_selected_subtask(true);
                    }
                    None => {
                        tracing::warn!(%subtask_id, "selection target is not in the visible model");
                        clear_subtask_selection(&window);
                    }
                }
            });
        }

        {
            let weak = window.as_weak();
            actions.on_go_to_parent_task(move || {
                let Some(window) = weak.upgrade() else { return };
                let parent_id = window.global::<AppState>().get_selected_task().id;
                clear_subtask_selection(&window);
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
                let target_list = list_id.as_str();
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
                    reminders: Vec::new(),
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
                            query::note_task_added(&state, &target_list, task.id.as_str());
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

        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_add_subtask(move |task_id, title| {
                let title = title.trim().to_string();
                let task_id = task_id.to_string();
                if title.is_empty() || task_id.is_empty() {
                    return;
                }

                let (project_and_order, user_id) = {
                    let state = state.lock().expect("shared state poisoned");
                    let task = state
                        .trees
                        .iter()
                        .flat_map(|tree| tree.task_lists.iter().map(move |list| (tree, list)))
                        .flat_map(|(tree, list)| list.tasks.iter().map(move |task| (tree, task)))
                        .find(|(_, task)| task.id.as_str() == task_id.as_str());
                    let project_and_order = task.map(|(tree, task)| {
                        let next_order = next_subtask_order(
                            task.sub_tasks.iter().map(|subtask| subtask.order_index),
                        );
                        (tree.id, next_order)
                    });
                    (project_and_order, state.current_user)
                };

                let (Some((project_id, order_index)), Some(user_id)) = (project_and_order, user_id)
                else {
                    tracing::error!(%task_id, "cannot add a subtask: unknown task or user");
                    report_error(&weak, "task.save-failed");
                    return;
                };
                let Ok(parsed_task_id) = TaskId::try_from_str(&task_id) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let subtask = new_subtask(parsed_task_id, title, order_index, user_id);

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let result = subtask_facades::create_sub_task(
                        repositories.as_ref(),
                        &project_id,
                        &subtask,
                        &user_id,
                    )
                    .await;

                    match subtask_save_result(result) {
                        Ok(_) => {
                            state
                                .lock()
                                .expect("shared state poisoned")
                                .task_ui
                                .expand(&task_id);
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            tracing::error!(%error, %task_id, "failed to create subtask");
                            report_error(&weak, error.code());
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
            let runtime = self.runtime.clone();
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
                        Ok(_) => {
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, "failed to update subtask");
                            report_error(&weak, ui_error.code());
                        }
                    }
                });
            });
        }

        // The remaining subtask edits follow the task ones: the row is updated
        // in place first, and a failed write puts the snapshot back.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let debouncer = Arc::new(EditDebouncer::default());
            actions.on_update_subtask_title(move |subtask_id, title| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = subtask_row(&window, &subtask_id) else {
                    return;
                };
                if before.title == title {
                    return;
                }
                let snapshot = SubTaskRowSnapshot::capture(&before);

                update_subtask_row(&window, &subtask_id, |item| item.title = title.clone());
                sync_selected_subtask(&window, &subtask_id);

                let patch = PartialSubTask {
                    title: Some(title.to_string()),
                    ..Default::default()
                };
                spawn_debounced_subtask_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    &debouncer,
                    subtask_id.to_string(),
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
            actions.on_update_subtask_notes(move |subtask_id, notes| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = subtask_row(&window, &subtask_id) else {
                    return;
                };
                if before.notes == notes {
                    return;
                }
                let snapshot = SubTaskRowSnapshot::capture(&before);

                update_subtask_row(&window, &subtask_id, |item| item.notes = notes.clone());
                sync_selected_subtask(&window, &subtask_id);

                let patch = PartialSubTask {
                    description: Some(Some(notes.to_string())),
                    ..Default::default()
                };
                spawn_debounced_subtask_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    &debouncer,
                    subtask_id.to_string(),
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
            actions.on_update_subtask_status(move |subtask_id, status| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = subtask_row(&window, &subtask_id) else {
                    return;
                };
                if before.status == status {
                    return;
                }
                let snapshot = SubTaskRowSnapshot::capture(&before);
                let completed = matches!(status, TaskStatus::Completed);

                update_subtask_row(&window, &subtask_id, |item| {
                    item.status = status;
                    item.completed = completed;
                    item.overdue = item.overdue && !completed;
                });
                sync_selected_subtask(&window, &subtask_id);

                let patch = PartialSubTask {
                    status: Some(from_status(status)),
                    completed: Some(completed),
                    ..Default::default()
                };
                spawn_subtask_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    subtask_id.to_string(),
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
            actions.on_update_subtask_priority(move |subtask_id, priority| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = subtask_row(&window, &subtask_id) else {
                    return;
                };
                if before.priority == priority {
                    return;
                }
                let snapshot = SubTaskRowSnapshot::capture(&before);

                update_subtask_row(&window, &subtask_id, |item| item.priority = priority);
                sync_selected_subtask(&window, &subtask_id);

                let patch = PartialSubTask {
                    priority: Some(Some(priority_value(priority))),
                    ..Default::default()
                };
                spawn_subtask_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    subtask_id.to_string(),
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
            actions.on_update_subtask_due(move |subtask_id, year, month, day, hour, minute| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = subtask_row(&window, &subtask_id) else {
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
                let snapshot = SubTaskRowSnapshot::capture(&before);
                let completed = before.completed;

                let display = DateTimeDisplaySettings::new(
                    window.global::<UiSettingsState>().get_timezone().as_str(),
                    window
                        .global::<UiSettingsState>()
                        .get_current_datetime_format()
                        .as_str(),
                );
                update_subtask_row(&window, &subtask_id, |item| {
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
                sync_selected_subtask(&window, &subtask_id);

                let patch = PartialSubTask {
                    plan_end_date: Some(Some(due)),
                    ..Default::default()
                };
                spawn_subtask_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    subtask_id.to_string(),
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
            actions.on_clear_subtask_due(move |subtask_id| {
                let Some(window) = weak.upgrade() else { return };
                let Some(before) = subtask_row(&window, &subtask_id) else {
                    return;
                };
                if !before.has_due {
                    return;
                }
                let snapshot = SubTaskRowSnapshot::capture(&before);

                update_subtask_row(&window, &subtask_id, |item| {
                    item.due_label = SharedString::default();
                    item.has_due = false;
                    item.overdue = false;
                });
                sync_selected_subtask(&window, &subtask_id);

                let patch = PartialSubTask {
                    plan_end_date: Some(None),
                    ..Default::default()
                };
                spawn_subtask_patch(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    subtask_id.to_string(),
                    patch,
                    snapshot,
                );
            });
        }

        // Deleting reloads instead of editing the row: the parent's subtask
        // counters are derived, and only the reload recomputes them.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_delete_subtask(move |subtask_id| {
                let project_id = {
                    let state = state.lock().expect("shared state poisoned");
                    state.project_of_subtask(&subtask_id)
                };
                let Some(project_id) = project_id else {
                    tracing::error!(%subtask_id, "cannot delete a subtask: unknown project");
                    report_error(&weak, "task.delete-failed");
                    return;
                };
                let Ok(parsed_id) = SubTaskId::try_from_str(subtask_id.as_str()) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                runtime.spawn(async move {
                    let result = subtask_facades::delete_sub_task(
                        repositories.as_ref(),
                        &project_id,
                        &parsed_id,
                    )
                    .await;

                    match subtask_save_result(result) {
                        Ok(()) => {
                            let deleted_id = subtask_id.clone();
                            if let Err(error) = weak.upgrade_in_event_loop(move |window| {
                                if window.global::<AppState>().get_selected_subtask_id()
                                    == deleted_id
                                {
                                    clear_subtask_selection(&window);
                                }
                            }) {
                                tracing::error!(%error, "could not clear the deleted subtask selection");
                            }
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            tracing::error!(%error, %subtask_id, "failed to delete subtask");
                            report_error(&weak, error.code());
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
            actions.on_delete_task(move |task_id| {
                let (project_id, user_id, reminders) = {
                    let state = state.lock().expect("shared state poisoned");
                    let task = state.task_with_project(&task_id);
                    (
                        task.map(|(project, _)| project.id),
                        state.current_user,
                        task.map(|(_, task)| task.reminders.clone()).unwrap_or_default(),
                    )
                };
                let (Some(project_id), Some(user_id)) = (project_id, user_id) else {
                    tracing::error!(%task_id, "cannot delete a task: unknown project or user");
                    report_error(&weak, "task.delete-failed");
                    return;
                };
                let Ok(parsed_id) = TaskId::try_from_str(task_id.as_str()) else {
                    report_error(&weak, "input.malformed-id");
                    return;
                };

                let weak = weak.clone();
                let state = Arc::clone(&state);
                let repositories = Arc::clone(&repositories);
                let platform = Arc::clone(&platform);
                runtime.spawn(async move {
                    let result = task_facades::delete_task(
                        repositories.as_ref(),
                        &project_id,
                        &parsed_id,
                        &user_id,
                        &Utc::now(),
                    )
                    .await;

                    match result {
                        Ok(_) => {
                            for reminder in reminders {
                                let id = NotificationId::scheduled(&task_id, &reminder);
                                if let Err(error) = platform.cancel_notification(&id).await {
                                    tracing::warn!(%error, %task_id, "failed to cancel deleted task reminder");
                                }
                            }
                            let deleted_id = task_id.clone();
                            if let Err(error) = weak.upgrade_in_event_loop(move |window| {
                                if window.global::<AppState>().get_selected_task_id() == deleted_id {
                                    clear_selection(&window);
                                }
                            }) {
                                tracing::error!(%error, "could not clear the deleted task selection");
                            }
                            reload_projects(&weak, &state, &repositories, timezone).await;
                        }
                        Err(error) => {
                            let ui_error = UiError::from(error);
                            tracing::error!(%ui_error, %task_id, "failed to delete task");
                            report_error(&weak, ui_error.code());
                        }
                    }
                });
            });
        }
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

        // Reorder: the row is moved in the published model first, and its new
        // neighbours are what tell storage where it belongs. A failure reloads,
        // because the stored order is the only thing that can correct the list.
        {
            let weak = window.as_weak();
            let state = Arc::clone(&self.state);
            let repositories = Arc::clone(&self.repositories);
            let runtime = self.runtime.clone();
            let timezone = self.timezone;
            actions.on_reorder_task(move |task_id, target_index| {
                // The stored order is only what the list shows while sorting is
                // manual; rewriting it from a derived order would move rows the
                // user never saw move.
                if state.lock().expect("shared state poisoned").task_sort != TaskSort::Manual {
                    tracing::debug!(%task_id, "ignoring a reorder while a sort is applied");
                    return;
                }

                let Some(window) = weak.upgrade() else { return };
                let app_state = window.global::<AppState>();
                let mut rows: Vec<TaskItem> = app_state.get_tasks().iter().collect();

                let Some(from) = rows.iter().position(|row| row.id == task_id) else {
                    return;
                };
                let to = (target_index.max(0) as usize).min(rows.len() - 1);
                if from == to {
                    return;
                }

                let row = rows.remove(from);
                let list_id = row.list_id.clone();
                rows.insert(to, row);

                // Only rows of the same list can anchor the move; the visible
                // list may span several of them.
                let previous = rows[..to]
                    .iter()
                    .rev()
                    .find(|row| row.list_id == list_id)
                    .and_then(|row| TaskId::try_from_str(row.id.as_str()).ok());
                let next = rows[to + 1..]
                    .iter()
                    .find(|row| row.list_id == list_id)
                    .and_then(|row| TaskId::try_from_str(row.id.as_str()).ok());

                app_state.set_tasks(ModelRc::new(VecModel::from(rows)));

                let updates = {
                    let mut guard = state.lock().expect("shared state poisoned");
                    ordering::reorder_task(&mut guard.trees, &task_id, previous, next)
                };
                let Some((project_id, updates)) = updates else {
                    tracing::warn!(%task_id, "cannot reorder a task that is not loaded");
                    return;
                };

                spawn_order_updates(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    project_id,
                    updates,
                    timezone,
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
                let Some((project_id, updates)) = updates else {
                    tracing::warn!(%task_id, %list_id, "cannot move the task to that list");
                    return;
                };

                let Some(window) = weak.upgrade() else { return };
                // The task may leave the visible set, and both lists change size.
                refresh_projects(&window, &state);
                refresh_tasks(&window, &state, timezone);

                let selected = window.global::<AppState>().get_selected_task_id();
                if !selected.is_empty() && task_row(&window, &selected).is_none() {
                    clear_selection(&window);
                }

                spawn_order_updates(
                    &weak,
                    &state,
                    &repositories,
                    &runtime,
                    project_id,
                    updates,
                    timezone,
                );
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
    runtime: Handle,
    timezone: DisplayTimezone,
}

impl<R> LifecycleObserver for LifecycleReactor<R>
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    fn on_event(&self, event: LifecycleEvent) {
        match event {
            // Nothing to flush: every mutation is persisted as it happens, which
            // is what makes the app survive the OS killing it while suspended.
            LifecycleEvent::Suspend => tracing::info!("suspending"),
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
    trees
        .iter()
        .filter(|project| !project.deleted)
        .flat_map(|project| project.task_lists.iter())
        .filter(|list| !list.deleted)
        .flat_map(|list| list.tasks.iter())
        .filter(|task| !task.deleted)
        .flat_map(|task| {
            task.reminders
                .iter()
                .map(|reminder| (task.id.as_str(), task.title.clone(), *reminder))
        })
        .collect()
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
                    guard.tags_by_project = tags_by_project;
                    guard.tag_names = tag_names;
                    guard.tag_bookmarks = tag_bookmarks;
                }
                refresh_projects(&window, &state);
                let rewrote = query::follow_data(&window, &state);
                publish_search(&window, &state, timezone, rewrote);
                let selected_task_id = app_state.get_selected_task_id();
                if !selected_task_id.is_empty() {
                    if task_row(&window, &selected_task_id).is_some() {
                        sync_selected_task(&window, &selected_task_id);
                    } else {
                        clear_selection(&window);
                    }
                }
                // A subtask that disappeared (deleted here or by another
                // client) sends the pane back to its parent task.
                let selected_subtask_id = app_state.get_selected_subtask_id();
                if !selected_subtask_id.is_empty() {
                    if subtask_row(&window, &selected_subtask_id).is_some() {
                        sync_selected_subtask(&window, &selected_subtask_id);
                    } else {
                        clear_subtask_selection(&window);
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
        .trees
        .iter()
        .flat_map(|tree| tree.task_lists.iter())
        .flat_map(|list| list.tasks.iter())
        .find(|task| task.id.as_str() == selected_task_id)
        .map(|task| task.tag_ids.iter().copied().collect())
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
    drop(state);

    let app_state = window.global::<AppState>();
    app_state.set_tags(ModelRc::new(VecModel::from(tags)));
    app_state.set_bookmarked_tags(ModelRc::new(VecModel::from(bookmarks)));
    query::highlight_sidebar(window, state_arc);
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

fn new_subtask(task_id: TaskId, title: String, order_index: i32, user_id: UserId) -> SubTask {
    let now = Utc::now();
    SubTask {
        id: SubTaskId::new(),
        task_id,
        title,
        description: None,
        status: DomainStatus::NotStarted,
        priority: None,
        plan_start_date: None,
        plan_end_date: None,
        do_start_date: None,
        do_end_date: None,
        is_range_date: None,
        recurrence_rule: None,
        assigned_user_ids: Vec::new(),
        tag_ids: Vec::new(),
        order_index,
        completed: false,
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    }
}

fn next_subtask_order(order_indexes: impl Iterator<Item = i32>) -> i32 {
    order_indexes
        .max()
        .map_or(0, |index| index.saturating_add(1))
}

fn priority_value(priority: TaskPriority) -> i32 {
    match priority {
        TaskPriority::None => 0,
        TaskPriority::Low => 1,
        TaskPriority::Medium => 3,
        TaskPriority::High => 6,
    }
}

fn subtask_save_result(result: Result<bool, ServiceError>) -> Result<(), UiError> {
    result.map(drop).map_err(UiError::from)
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

/// Restores a subtask row after a failed save.
fn rollback_subtask_row(weak: &Weak<AppWindow>, snapshot: SubTaskRowSnapshot) {
    let posted = weak.upgrade_in_event_loop(move |window| {
        let id = SharedString::from(snapshot.id.as_str());
        update_subtask_row(&window, &id, move |item| snapshot.restore(item));
        sync_selected_subtask(&window, &id);
    });
    if let Err(error) = posted {
        tracing::error!(%error, "could not roll back the optimistic subtask update");
    }
}

/// Waits for a pause in subtask text input, then persists only the newest value.
#[allow(clippy::too_many_arguments)]
fn spawn_debounced_subtask_patch<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    debouncer: &Arc<EditDebouncer<SubTaskRowSnapshot>>,
    subtask_id: String,
    patch: PartialSubTask,
    snapshot: SubTaskRowSnapshot,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let revision = debouncer.schedule(subtask_id.clone(), snapshot);
    let weak = weak.clone();
    let state = Arc::clone(state);
    let repositories = Arc::clone(repositories);
    let debouncer = Arc::clone(debouncer);
    let subtask_runtime = runtime.clone();
    runtime.spawn(async move {
        tokio::time::sleep(TEXT_SAVE_DEBOUNCE).await;
        let Some(snapshot) = debouncer.take_if_latest(&subtask_id, revision) else {
            return;
        };
        spawn_subtask_patch(
            &weak,
            &state,
            &repositories,
            &subtask_runtime,
            subtask_id,
            patch,
            snapshot,
        );
    });
}

/// Persists a subtask patch, rolling the row back when the write fails.
fn spawn_subtask_patch<R>(
    weak: &Weak<AppWindow>,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    runtime: &Handle,
    subtask_id: String,
    patch: PartialSubTask,
    snapshot: SubTaskRowSnapshot,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let (project_id, user_id) = {
        let state = state.lock().expect("shared state poisoned");
        (state.project_of_subtask(&subtask_id), state.current_user)
    };
    let (Some(project_id), Some(user_id)) = (project_id, user_id) else {
        tracing::error!(%subtask_id, "cannot update a subtask: unknown project or user");
        rollback_subtask_row(weak, snapshot);
        report_error(weak, "task.save-failed");
        return;
    };
    let Ok(parsed_id) = SubTaskId::try_from_str(&subtask_id) else {
        rollback_subtask_row(weak, snapshot);
        report_error(weak, "input.malformed-id");
        return;
    };

    let weak = weak.clone();
    let repositories = Arc::clone(repositories);
    runtime.spawn(async move {
        let result = subtask_facades::update_sub_task(
            repositories.as_ref(),
            &project_id,
            &parsed_id,
            &patch,
            &user_id,
        )
        .await;

        if let Err(error) = result {
            let ui_error = UiError::from(error);
            tracing::error!(%ui_error, %subtask_id, "failed to update subtask");
            rollback_subtask_row(&weak, snapshot);
            report_error(&weak, ui_error.code());
        }
    });
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

/// Reads the current UI row for a task.
fn task_row(window: &AppWindow, task_id: &SharedString) -> Option<TaskItem> {
    indexed_task_row(window, task_id).map(|(_, item)| item)
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

    clear_subtask_selection(window);
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

    clear_subtask_selection(window);
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

/// Reads the current UI row for a subtask, wherever its parent task is.
fn subtask_row(window: &AppWindow, subtask_id: &SharedString) -> Option<SubTaskItem> {
    window
        .global::<AppState>()
        .get_tasks()
        .iter()
        .find_map(|task| task.subtasks.iter().find(|sub| sub.id == subtask_id))
}

/// Applies a change to a single subtask row without rebuilding the model.
///
/// The subtask models are nested inside the task rows, and the nested `ModelRc`
/// is shared with `selected-task`, so writing here updates both views of it.
fn update_subtask_row(window: &AppWindow, subtask_id: &str, edit: impl FnOnce(&mut SubTaskItem)) {
    for task in window.global::<AppState>().get_tasks().iter() {
        let subtasks = task.subtasks;
        let Some(index) = subtasks.iter().position(|sub| sub.id == subtask_id) else {
            continue;
        };
        if let Some(mut item) = subtasks.row_data(index) {
            edit(&mut item);
            subtasks.set_row_data(index, item);
        }
        return;
    }
}

/// Keeps the subtask detail pane in step after a row changed in place.
fn sync_selected_subtask(window: &AppWindow, subtask_id: &SharedString) {
    let app_state = window.global::<AppState>();
    if app_state.get_selected_subtask_id() != subtask_id {
        return;
    }
    if let Some(item) = subtask_row(window, subtask_id) {
        app_state.set_selected_subtask(item);
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
    app_state.set_has_selected_task(false);
    clear_subtask_selection(window);
    app_state.set_active_pane(Pane::List);
}

/// Drops the subtask selection, leaving the parent task selected.
fn clear_subtask_selection(window: &AppWindow) {
    let app_state = window.global::<AppState>();
    app_state.set_selected_subtask_id(SharedString::default());
    app_state.set_has_selected_subtask(false);
    app_state.set_selected_subtask(SubTaskItem::default());
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
    fn a_new_subtask_has_default_values() {
        let task_id = TaskId::new();
        let user_id = UserId::new();

        let subtask = new_subtask(task_id, "Pick up bread".to_string(), 3, user_id);

        assert_eq!(subtask.task_id, task_id);
        assert_eq!(subtask.title, "Pick up bread");
        assert_eq!(subtask.order_index, 3);
        assert_eq!(subtask.status, DomainStatus::NotStarted);
        assert!(!subtask.completed);
        assert_eq!(subtask.updated_by, user_id);
    }

    #[test]
    fn a_new_subtask_follows_the_highest_existing_order() {
        assert_eq!(next_subtask_order([2, 7, 4].into_iter()), 8);
        assert_eq!(next_subtask_order([].into_iter()), 0);
        assert_eq!(next_subtask_order([i32::MAX].into_iter()), i32::MAX);
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
            list_id: TaskListId::new(),
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

    #[test]
    fn a_subtask_save_failure_becomes_a_ui_error() {
        let result = subtask_save_result(Err(ServiceError::ValidationError(
            "title is required".to_string(),
        )));

        assert_eq!(
            result.expect_err("the save should fail").code(),
            "input.validation-failed"
        );
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
        let other = project_named(
            "Home",
            vec![list_named("Errands", vec![task_named("Milk")])],
        );

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
        assert_eq!(titles, ["Buy milk", "Milk"]);

        state.show_archived_projects = true;
        let titles: Vec<&str> = state
            .live_tasks()
            .map(|(_, _, task)| task.title.as_str())
            .collect();
        assert_eq!(titles, ["Buy milk", "Shelved", "Milk"]);
    }

    #[test]
    fn reminders_are_only_collected_from_rows_that_still_exist() {
        let due = Utc::now();
        let live = TaskTree {
            reminders: vec![due],
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
            vec![(live_id, "Call the dentist".to_string(), due)],
            "a reminder on a row the user deleted must not still fire"
        );
    }
}
