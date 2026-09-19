//! Connects the search box to the task list, the sidebar and quick add.
//!
//! The query is the only state that decides which tasks are listed. Sidebar
//! items rewrite it, their highlight is derived from it, and the quick-add
//! destination defaults to the list or project it narrows to. The rules live
//! in `viewmodels::search`; this module moves data between them and Slint.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::subtask::SubTaskTree;
use flequit_model::models::task_projects::task::TaskTree;
use flequit_model::models::task_projects::task_list::TaskListTree;
use flequit_model::types::task_types::TaskStatus as DomainStatus;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::{SharedState, clear_selection, display_settings, refresh_tags, report_error};
use crate::adapters::color::parse_hex;
use crate::adapters::datetime::DisplayTimezone;
use crate::adapters::to_task_item;
use crate::bindings::{
    Actions, AddTargetItem, AppState, AppWindow, FilterHighlight, FilterKind, I18n, QueryEdit,
    SearchSuggestion, SearchSuggestionKind, TaskItem,
};
use crate::viewmodels::ordering;
use crate::viewmodels::search::{
    DueSpec, EvalContext, Highlight, ItemKey, Lang, ListEntry, NameIndex, Prepared, ProjectEntry,
    Reference, SearchUnit, Segment, StatusKey, Suggestion, SuggestionKind, UnitStatus, UnitText,
    fold, tag_key,
};
use crate::viewmodels::settings::RECENT_ADD_TARGETS;

use super::SearchEdit;

/// Registers the search box, sidebar filter and quick-add callbacks.
pub(super) fn bind(window: &AppWindow, state: &Arc<Mutex<SharedState>>, timezone: DisplayTimezone) {
    let actions = window.global::<Actions>();

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_search_changed(move |text| {
            {
                let mut guard = state.lock().expect("shared state poisoned");
                let guard = &mut *guard;
                guard.search.edit(text.as_str(), &guard.search_index);
                guard.just_added = None;
            }
            if let Some(window) = weak.upgrade() {
                publish_search(&window, &state, timezone, false);
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_search_committed(move || {
            {
                let mut guard = state.lock().expect("shared state poisoned");
                let guard = &mut *guard;
                guard.search.commit(&guard.search_index);
            }
            if let Some(window) = weak.upgrade() {
                publish_search(&window, &state, timezone, false);
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_search_suggestion_chosen(move |position| {
            let Some(window) = weak.upgrade() else { return };
            let lang = ui_lang(&window);
            let chosen = {
                let mut guard = state.lock().expect("shared state poisoned");
                let guard = &mut *guard;
                let tags = tag_names(guard);
                guard.search.choose_suggestion(
                    usize::try_from(position).unwrap_or(usize::MAX),
                    &guard.search_index,
                    &tags,
                    lang,
                )
            };
            publish_search(&window, &state, timezone, chosen);
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_search_candidate_chosen(move |position| {
            {
                let mut guard = state.lock().expect("shared state poisoned");
                guard
                    .search
                    .choose_candidate(usize::try_from(position).unwrap_or(usize::MAX));
            }
            if let Some(window) = weak.upgrade() {
                publish_search(&window, &state, timezone, false);
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_search_candidates_dismissed(move || {
            state
                .lock()
                .expect("shared state poisoned")
                .search
                .dismiss_candidates();
            if let Some(window) = weak.upgrade() {
                publish_search(&window, &state, timezone, false);
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_search_candidates_reopened(move || {
            {
                let mut guard = state.lock().expect("shared state poisoned");
                guard.search.reopen_candidates();
                // Closed by a click outside it; ask the view to open it again.
                guard.candidates_shown = false;
            }
            if let Some(window) = weak.upgrade() {
                publish_search(&window, &state, timezone, false);
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_apply_filter(move |kind, key, edit| {
            let Some(window) = weak.upgrade() else { return };
            let lang = ui_lang(&window);
            let applied = {
                let mut guard = state.lock().expect("shared state poisoned");
                let guard = &mut *guard;
                match sidebar_item(&window, &guard.search_index, kind, key.as_str(), lang) {
                    Some((item_key, segment)) => {
                        guard.search.apply_item(
                            &item_key,
                            segment,
                            search_edit(edit),
                            &guard.search_index,
                        );
                        guard.just_added = None;
                        true
                    }
                    None => false,
                }
            };
            if !applied {
                report_error(&weak, "entity.not-found");
                return;
            }
            // Replacing the query is navigation; combining keeps the overlay
            // open for the next item.
            if matches!(edit, QueryEdit::Click | QueryEdit::Replace) {
                window.global::<AppState>().set_sidebar_open(false);
            }
            clear_selection(&window);
            publish_search(&window, &state, timezone, true);
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_locale_changed(move || {
            let Some(window) = weak.upgrade() else { return };
            let lang = ui_lang(&window);
            let rewrote = {
                let mut guard = state.lock().expect("shared state poisoned");
                let guard = &mut *guard;
                let before = guard.search.text();
                guard.search.refresh(&guard.search_index, lang, true);
                guard.search.text() != before
            };
            publish_search(&window, &state, timezone, rewrote);
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_choose_add_target(move |list_id| {
            {
                let mut guard = state.lock().expect("shared state poisoned");
                let default = default_add_target(&guard);
                guard.add_target_override = Some((list_id.to_string(), default));
            }
            if let Some(window) = weak.upgrade() {
                publish_add_target(&window, &state);
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_filter_add_targets(move |filter| {
            state
                .lock()
                .expect("shared state poisoned")
                .add_target_filter = filter.to_string();
            if let Some(window) = weak.upgrade() {
                publish_add_targets(&window, &state);
            }
        });
    }
}

/// The search vocabulary for the current UI language.
pub(super) fn ui_lang(window: &AppWindow) -> Lang {
    Lang::from_locale(window.global::<I18n>().get_current_locale().as_str())
}

/// Brings the search box up to date with freshly loaded data.
///
/// Rebuilds the name index, restores the stored query on the first load, and
/// otherwise rewrites confirmed tokens whose target was renamed or deleted.
/// Returns whether the search box text changed.
pub(super) fn follow_data(window: &AppWindow, state: &Arc<Mutex<SharedState>>) -> bool {
    let lang = ui_lang(window);
    let mut guard = state.lock().expect("shared state poisoned");
    let guard = &mut *guard;
    guard.search_index = build_index(guard);
    let before = guard.search.text();
    match guard.pending_search.take() {
        Some(saved) => guard.search.restore(&saved, &guard.search_index, lang),
        None => guard.search.refresh(&guard.search_index, lang, false),
    }
    guard.search.text() != before
}

/// Records the users an `@` token can name and rebuilds the index.
pub(super) fn set_users(
    state: &Arc<Mutex<SharedState>>,
    users: Vec<crate::viewmodels::search::UserEntry>,
) {
    let mut guard = state.lock().expect("shared state poisoned");
    guard.users = users;
    guard.search_index = build_index(&guard);
}

fn build_index(state: &SharedState) -> NameIndex {
    let projects = state
        .trees
        .iter()
        .filter(|tree| !tree.deleted)
        .map(|tree| ProjectEntry {
            id: tree.id.as_str(),
            name: tree.name.clone(),
            color: tree.color.clone().unwrap_or_default(),
            list_count: tree
                .task_lists
                .iter()
                .filter(|list| !list.deleted && !list.is_archived)
                .count(),
            archived: tree.is_archived,
        })
        .collect();
    let lists = state
        .trees
        .iter()
        .filter(|tree| !tree.deleted)
        .flat_map(|tree| tree.task_lists.iter())
        .filter(|list| !list.deleted && !list.is_archived)
        .map(|list| ListEntry {
            id: list.id.as_str(),
            project_id: list.project_id.as_str(),
            name: list.name.clone(),
        })
        .collect();
    NameIndex::new(projects, lists, state.users.clone())
}

fn tag_names(state: &SharedState) -> Vec<String> {
    state
        .tags_by_project
        .values()
        .flatten()
        .filter(|tag| !tag.deleted)
        .map(|tag| tag.name.clone())
        .collect()
}

/// Publishes the search box and everything the query drives.
///
/// `rewrote` says the text changed under the user (a sidebar item, a picked
/// suggestion, a rename), so the caret is moved to the end.
pub(super) fn publish_search(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    timezone: DisplayTimezone,
    rewrote: bool,
) {
    let lang = ui_lang(window);
    let (view, open_candidates) = {
        let mut guard = state.lock().expect("shared state poisoned");
        let guard = &mut *guard;
        let tags = tag_names(guard);
        let view = guard.search.view(&guard.search_index, &tags, lang);
        let open = !view.candidates.is_empty();
        let newly_open = open && !guard.candidates_shown;
        guard.candidates_shown = open;
        (view, newly_open)
    };

    let app_state = window.global::<AppState>();
    if app_state.get_search_query() != view.text.as_str() {
        app_state.set_search_query(SharedString::from(view.text.as_str()));
    }
    if rewrote {
        app_state.set_search_caret_request(app_state.get_search_caret_request().wrapping_add(1));
    }
    app_state.set_search_suggestions(suggestion_model(&view.suggestions));
    app_state.set_search_candidates(suggestion_model(&view.candidates));
    if open_candidates {
        app_state.set_search_candidates_request(
            app_state.get_search_candidates_request().wrapping_add(1),
        );
    }
    app_state.set_search_unresolved(SharedString::from(view.unresolved.join(" ")));
    app_state.set_search_ambiguous(SharedString::from(view.ambiguous.join(" ")));
    app_state.set_search_syntax_issue(view.syntax_issue);

    publish_highlights(window, &view.highlights);
    publish_add_target(window, state);
    refresh_tasks(window, state, timezone);
    refresh_tags(window, state);
    remember(state);
}

fn suggestion_model(suggestions: &[Suggestion]) -> ModelRc<SearchSuggestion> {
    let items = suggestions
        .iter()
        .map(|suggestion| {
            let color = parse_hex(&suggestion.color);
            SearchSuggestion {
                label: SharedString::from(suggestion.label.as_str()),
                kind: match suggestion.kind {
                    SuggestionKind::Due => SearchSuggestionKind::Due,
                    SuggestionKind::Status => SearchSuggestionKind::Status,
                    SuggestionKind::Project => SearchSuggestionKind::Project,
                    SuggestionKind::TaskList => SearchSuggestionKind::TaskList,
                    SuggestionKind::User => SearchSuggestionKind::User,
                    SuggestionKind::Tag => SearchSuggestionKind::Tag,
                },
                detail: SharedString::from(suggestion.detail.as_str()),
                color_brush: color.unwrap_or_default().into(),
                has_color: color.is_some(),
                count: i32::try_from(suggestion.count).unwrap_or(i32::MAX),
                archived: suggestion.archived,
            }
        })
        .collect::<Vec<_>>();
    ModelRc::new(VecModel::from(items))
}

/// Saves the query and quick-add history if they changed since the last save.
fn remember(state: &Arc<Mutex<SharedState>>) {
    let mut guard = state.lock().expect("shared state poisoned");
    // Nothing to save until the stored query has been restored.
    if guard.pending_search.is_some() {
        return;
    }
    let current = (guard.search.canonical(), guard.recent_add_targets.clone());
    if current == guard.remembered {
        return;
    }
    if let Some(memory) = &guard.search_memory {
        memory.remember(current.0.clone(), current.1.clone());
    }
    guard.remembered = current;
}

/// The sidebar item a filter action names, and the token it writes.
fn sidebar_item(
    window: &AppWindow,
    index: &NameIndex,
    kind: FilterKind,
    key: &str,
    lang: Lang,
) -> Option<(ItemKey, Segment)> {
    let reference = match kind {
        FilterKind::Tag => {
            return Some((tag_key(key), Segment::Text(format!("#{key}"))));
        }
        FilterKind::Due => {
            let filters = window.global::<AppState>().get_due_filters();
            let query = filters.iter().find(|filter| filter.key == key)?.query;
            Reference::Due(DueSpec::parse(query.trim_start_matches('@'))?)
        }
        FilterKind::Status => Reference::Status(StatusKey::parse(key)?),
        FilterKind::Project => Reference::Project(key.to_string()),
        FilterKind::TaskList => Reference::List(key.to_string()),
    };
    let text = index.render(&reference, lang, false)?;
    Some((
        ItemKey::Ref(reference.clone()),
        Segment::Bound { text, reference },
    ))
}

fn search_edit(edit: QueryEdit) -> SearchEdit {
    match edit {
        QueryEdit::Click => SearchEdit::Click,
        QueryEdit::ToggleAnd => SearchEdit::ToggleAnd,
        QueryEdit::ToggleOr => SearchEdit::ToggleOr,
        QueryEdit::Replace => SearchEdit::Replace,
        QueryEdit::And => SearchEdit::And,
        QueryEdit::Or => SearchEdit::Or,
        QueryEdit::Exclude => SearchEdit::Exclude,
        QueryEdit::Remove => SearchEdit::Remove,
    }
}

fn ui_highlight(highlights: &[(ItemKey, Highlight)], key: &ItemKey) -> FilterHighlight {
    // The strongest use wins when an item appears more than once.
    let mut result = FilterHighlight::None;
    for (candidate, highlight) in highlights {
        if candidate != key {
            continue;
        }
        let next = match highlight {
            Highlight::None => FilterHighlight::None,
            Highlight::Weak => FilterHighlight::Weak,
            Highlight::Strong => FilterHighlight::Strong,
            Highlight::Excluded => FilterHighlight::Excluded,
        };
        if rank(next) > rank(result) {
            result = next;
        }
    }
    result
}

fn rank(highlight: FilterHighlight) -> u8 {
    match highlight {
        FilterHighlight::None => 0,
        FilterHighlight::Weak => 1,
        FilterHighlight::Excluded => 2,
        FilterHighlight::Strong => 3,
    }
}

/// Recomputes the sidebar highlight from the current query.
///
/// Called after the sidebar models are rebuilt, which resets it.
pub(super) fn highlight_sidebar(window: &AppWindow, state: &Arc<Mutex<SharedState>>) {
    let highlights = state
        .lock()
        .expect("shared state poisoned")
        .search
        .highlights();
    publish_highlights(window, &highlights);
}

fn publish_highlights(window: &AppWindow, highlights: &[(ItemKey, Highlight)]) {
    let app_state = window.global::<AppState>();

    let due = app_state.get_due_filters();
    for index in 0..due.row_count() {
        let Some(mut item) = due.row_data(index) else {
            continue;
        };
        let level = DueSpec::parse(item.query.trim_start_matches('@'))
            .map(|spec| ui_highlight(highlights, &ItemKey::Ref(Reference::Due(spec))))
            .unwrap_or(FilterHighlight::None);
        if item.highlight != level {
            item.highlight = level;
            due.set_row_data(index, item);
        }
    }

    let statuses = app_state.get_status_filters();
    for index in 0..statuses.row_count() {
        let Some(mut item) = statuses.row_data(index) else {
            continue;
        };
        let level = StatusKey::parse(item.key.as_str())
            .map(|status| ui_highlight(highlights, &ItemKey::Ref(Reference::Status(status))))
            .unwrap_or(FilterHighlight::None);
        if item.highlight != level {
            item.highlight = level;
            statuses.set_row_data(index, item);
        }
    }

    let projects = app_state.get_projects();
    for index in 0..projects.row_count() {
        let Some(mut item) = projects.row_data(index) else {
            continue;
        };
        let lists = item.task_lists.clone();
        for list_index in 0..lists.row_count() {
            let Some(mut list) = lists.row_data(list_index) else {
                continue;
            };
            let level = ui_highlight(
                highlights,
                &ItemKey::Ref(Reference::List(list.id.to_string())),
            );
            if list.highlight != level {
                list.highlight = level;
                lists.set_row_data(list_index, list);
            }
        }
        let level = ui_highlight(
            highlights,
            &ItemKey::Ref(Reference::Project(item.id.to_string())),
        );
        if item.highlight != level {
            item.highlight = level;
            projects.set_row_data(index, item);
        }
    }

    let tags = app_state.get_bookmarked_tags();
    for index in 0..tags.row_count() {
        let Some(mut item) = tags.row_data(index) else {
            continue;
        };
        let level = ui_highlight(highlights, &tag_key(item.name.as_str()));
        if item.highlight != level {
            item.highlight = level;
            tags.set_row_data(index, item);
        }
    }
}

// -- Quick-add destination --------------------------------------------------

/// Whether `list_id` is a list a task can be added to right now.
fn is_live_list(state: &SharedState, list_id: &str) -> bool {
    state
        .live_lists()
        .any(|(_, list)| list.id.as_str() == list_id)
}

/// The destination the query suggests: its one list, or a list of its one
/// project, else the last list used, else the first list there is.
fn default_add_target(state: &SharedState) -> Option<String> {
    let strong = state.search.strong_references();
    let lists: Vec<&String> = strong
        .iter()
        .filter_map(|reference| match reference {
            Reference::List(id) => Some(id),
            _ => None,
        })
        .collect();
    if let [list] = lists.as_slice()
        && is_live_list(state, list)
    {
        return Some((*list).clone());
    }

    let projects: Vec<&String> = strong
        .iter()
        .filter_map(|reference| match reference {
            Reference::Project(id) => Some(id),
            _ => None,
        })
        .collect();
    if let [project] = projects.as_slice() {
        let in_project: Vec<String> = state
            .live_lists()
            .filter(|(tree, _)| tree.id.as_str() == **project)
            .map(|(_, list)| list.id.as_str())
            .collect();
        let recent = state
            .recent_add_targets
            .iter()
            .find(|id| in_project.contains(id));
        if let Some(list) = recent.or(in_project.first()) {
            return Some(list.clone());
        }
    }

    state
        .recent_add_targets
        .iter()
        .find(|id| is_live_list(state, id))
        .cloned()
        .or_else(|| state.live_lists().next().map(|(_, list)| list.id.as_str()))
}

/// Settles the destination and the project the tag manager works on.
fn settle_add_target(state: &mut SharedState, selected_task_id: &str) {
    let default = default_add_target(state);
    let target = match state.add_target_override.take() {
        Some((chosen, replaced)) if replaced == default && is_live_list(state, &chosen) => {
            state.add_target_override = Some((chosen.clone(), replaced));
            Some(chosen)
        }
        _ => default,
    };
    state.add_target = target;

    let strong = state.search.strong_references();
    let project_of_list = |id: &str| state.project_of_list(id).map(|project| project.as_str());
    let from_query = strong.iter().find_map(|reference| match reference {
        Reference::List(id) => project_of_list(id),
        Reference::Project(id) => Some(id.clone()),
        _ => None,
    });
    state.context_project_id = from_query
        .or_else(|| {
            state
                .project_of_task(selected_task_id)
                .map(|project| project.as_str())
        })
        .or_else(|| state.add_target.as_deref().and_then(project_of_list))
        .unwrap_or_default();
}

pub(super) fn publish_add_target(window: &AppWindow, state: &Arc<Mutex<SharedState>>) {
    let app_state = window.global::<AppState>();
    let selected_task_id = app_state.get_selected_task_id().to_string();
    let (list_id, project_name, list_name, color, context) = {
        let mut guard = state.lock().expect("shared state poisoned");
        settle_add_target(&mut guard, &selected_task_id);
        let found = guard.add_target.as_deref().and_then(|id| {
            guard
                .live_lists()
                .find(|(_, list)| list.id.as_str() == id)
                .map(|(tree, list)| (tree.name.clone(), list.name.clone(), tree.color.clone()))
        });
        let (project_name, list_name, color) = found.unwrap_or_default();
        (
            guard.add_target.clone().unwrap_or_default(),
            project_name,
            list_name,
            color,
            guard.context_project_id.clone(),
        )
    };
    let color = color.as_deref().and_then(parse_hex);
    app_state.set_add_target_list_id(list_id.into());
    app_state.set_add_target_project_name(project_name.into());
    app_state.set_add_target_list_name(list_name.into());
    app_state.set_add_target_color(color.unwrap_or_default().into());
    app_state.set_add_target_has_color(color.is_some());
    app_state.set_selected_project_id(context.into());
}

/// Fills the destination picker, recent lists first.
fn publish_add_targets(window: &AppWindow, state: &Arc<Mutex<SharedState>>) {
    let items = {
        let guard = state.lock().expect("shared state poisoned");
        let needle = fold(&guard.add_target_filter);
        let matches = |tree: &ProjectTree, list: &TaskListTree| {
            needle.is_empty()
                || fold(&list.name).contains(&needle)
                || fold(&tree.name).contains(&needle)
        };
        let item = |tree: &ProjectTree, list: &TaskListTree, recent: bool| {
            let color = tree.color.as_deref().and_then(parse_hex);
            AddTargetItem {
                list_id: SharedString::from(list.id.as_str()),
                project_name: SharedString::from(tree.name.as_str()),
                list_name: SharedString::from(list.name.as_str()),
                color_brush: color.unwrap_or_default().into(),
                has_color: color.is_some(),
                recent,
            }
        };
        let recent: Vec<AddTargetItem> = guard
            .recent_add_targets
            .iter()
            .filter_map(|id| guard.live_lists().find(|(_, list)| list.id.as_str() == *id))
            .filter(|(tree, list)| matches(tree, list))
            .map(|(tree, list)| item(tree, list, true))
            .collect();
        let rest = guard
            .live_lists()
            .filter(|(_, list)| !guard.recent_add_targets.contains(&list.id.as_str()))
            .filter(|(tree, list)| matches(tree, list))
            .map(|(tree, list)| item(tree, list, false));
        recent.into_iter().chain(rest).collect::<Vec<_>>()
    };
    window
        .global::<AppState>()
        .set_add_targets(ModelRc::new(VecModel::from(items)));
}

/// Records that a task was added to `list_id`, so it leads the picker and
/// becomes the fallback destination.
pub(super) fn note_task_added(state: &Arc<Mutex<SharedState>>, list_id: &str, task_id: String) {
    let mut guard = state.lock().expect("shared state poisoned");
    guard.recent_add_targets.retain(|id| id != list_id);
    guard.recent_add_targets.insert(0, list_id.to_string());
    guard.recent_add_targets.truncate(RECENT_ADD_TARGETS);
    guard.just_added = Some(task_id);
}

// -- Task list --------------------------------------------------------------

fn unit_status(status: &DomainStatus) -> UnitStatus {
    match status {
        DomainStatus::NotStarted => UnitStatus::NotStarted,
        DomainStatus::InProgress => UnitStatus::InProgress,
        DomainStatus::Waiting => UnitStatus::Waiting,
        DomainStatus::Completed => UnitStatus::Completed,
        DomainStatus::Cancelled => UnitStatus::Cancelled,
    }
}

fn subtask_status(subtask: &SubTaskTree) -> UnitStatus {
    // `completed` predates the status field and is still written by some
    // clients, so either one marks the subtask finished.
    if subtask.completed {
        UnitStatus::Completed
    } else {
        unit_status(&subtask.status)
    }
}

/// A task and its live subtasks, with the owned values their search units
/// borrow.
struct Row<'a> {
    task: &'a TaskTree,
    project_id: String,
    list_id: String,
    tags: Vec<&'a str>,
    assignees: Vec<String>,
    children: Vec<UnitText<'a>>,
    subtasks: Vec<SubRow<'a>>,
}

struct SubRow<'a> {
    subtask: &'a SubTaskTree,
    tags: Vec<&'a str>,
    assignees: Vec<String>,
}

impl<'a> Row<'a> {
    fn new(
        state: &'a SharedState,
        project: &'a ProjectTree,
        list: &'a TaskListTree,
        task: &'a TaskTree,
    ) -> Self {
        let tags = state.tag_names_of(task);
        let live: Vec<&SubTaskTree> = task.sub_tasks.iter().filter(|sub| !sub.deleted).collect();
        let subtasks = live
            .iter()
            .map(|subtask| {
                let mut own: Vec<&str> = subtask
                    .tag_ids
                    .iter()
                    .filter_map(|id| state.tag_names.get(id).map(String::as_str))
                    .collect();
                own.extend(tags.iter().copied());
                SubRow {
                    subtask,
                    tags: own,
                    assignees: subtask
                        .assigned_user_ids
                        .iter()
                        .map(|id| id.as_str())
                        .collect(),
                }
            })
            .collect();
        Self {
            task,
            project_id: project.id.as_str(),
            list_id: list.id.as_str(),
            assignees: task
                .assigned_user_ids
                .iter()
                .map(|id| id.as_str())
                .collect(),
            children: live
                .iter()
                .map(|sub| UnitText {
                    title: &sub.title,
                    notes: sub.description.as_deref(),
                })
                .collect(),
            tags,
            subtasks,
        }
    }

    fn task_unit(&self) -> SearchUnit<'_> {
        SearchUnit {
            project_id: &self.project_id,
            list_id: &self.list_id,
            title: &self.task.title,
            notes: self.task.description.as_deref(),
            parent: None,
            children: &self.children,
            due: self.task.plan_end_date.as_ref(),
            status: unit_status(&self.task.status),
            assignees: &self.assignees,
            tags: &self.tags,
        }
    }

    fn subtask_unit<'s>(&'s self, sub: &'s SubRow<'a>) -> SearchUnit<'s> {
        SearchUnit {
            project_id: &self.project_id,
            list_id: &self.list_id,
            title: &sub.subtask.title,
            notes: sub.subtask.description.as_deref(),
            parent: Some(UnitText {
                title: &self.task.title,
                notes: self.task.description.as_deref(),
            }),
            children: &[],
            due: sub.subtask.plan_end_date.as_ref(),
            status: subtask_status(sub.subtask),
            assignees: &sub.assignees,
            tags: &sub.tags,
        }
    }

    /// Whether the task matches, and which of its subtasks do.
    fn evaluate(&self, query: &Prepared, context: &EvalContext) -> (bool, Vec<String>) {
        let task = query.matches(&self.task_unit(), context);
        let subtasks = self
            .subtasks
            .iter()
            .filter(|sub| query.matches(&self.subtask_unit(sub), context))
            .map(|sub| sub.subtask.id.as_str())
            .collect();
        (task, subtasks)
    }
}

/// Rebuilds the task model from the query, and the sidebar counts with it.
///
/// A full replacement, which is correct here: the visible set changes
/// wholesale when the query changes. Single-row edits use `update_task_row`.
pub(super) fn refresh_tasks(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    timezone: DisplayTimezone,
) {
    let display = display_settings(window, timezone);
    let context = EvalContext {
        now: Utc::now(),
        timezone: display.timezone,
    };
    let guard = state.lock().expect("shared state poisoned");
    let query = guard.search.prepared(&guard.search_index);
    let filtering = !query.is_empty();

    let mut rows: Vec<Row<'_>> = guard
        .live_tasks()
        .map(|(project, list, task)| Row::new(&guard, project, list, task))
        .collect();
    // Stable, so the stored order stays the tiebreaker in every mode.
    rows.sort_by(|a, b| ordering::compare(a.task, b.task, guard.task_sort));

    let mut items: Vec<TaskItem> = Vec::new();
    for row in &rows {
        let (task_matches, matched) = row.evaluate(&query, &context);
        let just_added = guard.just_added.as_deref() == Some(row.task.id.as_str().as_str());
        if !task_matches && matched.is_empty() && !just_added {
            continue;
        }
        // Listed only for its subtasks: shown receded and opened up.
        let dimmed = filtering && !task_matches && !just_added;
        let expanded = guard.task_ui.is_expanded(&row.task.id.as_str()) || dimmed;
        let mut item = to_task_item(row.task, expanded, &context.now, &display, &row.tags);
        if dimmed {
            item.search_dimmed = true;
            item.matched_subtask_count = i32::try_from(matched.len()).unwrap_or(i32::MAX);
            let subtasks = item.subtasks.clone();
            for index in 0..subtasks.row_count() {
                if let Some(mut sub) = subtasks.row_data(index)
                    && matched.iter().any(|id| sub.id == id.as_str())
                {
                    sub.search_match = true;
                    subtasks.set_row_data(index, sub);
                }
            }
        }
        items.push(item);
    }

    tracing::debug!(tasks = items.len(), filtering, "publishing task list");
    let app_state = window.global::<AppState>();
    app_state.set_tasks(ModelRc::new(VecModel::from(items)));
    publish_counts(window, &rows, &context);
    drop(rows);
    drop(guard);
}

/// Counts, for each due and state button, the rows its query alone would list.
fn publish_counts(window: &AppWindow, rows: &[Row<'_>], context: &EvalContext) {
    let count = |reference: Reference| {
        let query = Prepared::single(reference);
        let listed = rows
            .iter()
            .filter(|row| {
                let (task, subtasks) = row.evaluate(&query, context);
                task || !subtasks.is_empty()
            })
            .count();
        i32::try_from(listed).unwrap_or(i32::MAX)
    };

    let app_state = window.global::<AppState>();
    let due = app_state.get_due_filters();
    for index in 0..due.row_count() {
        let Some(mut item) = due.row_data(index) else {
            continue;
        };
        let Some(spec) = DueSpec::parse(item.query.trim_start_matches('@')) else {
            tracing::warn!(query = %item.query, "due filter button has no due keyword");
            continue;
        };
        let value = count(Reference::Due(spec));
        if item.count != value {
            item.count = value;
            due.set_row_data(index, item);
        }
    }

    let statuses = app_state.get_status_filters();
    for index in 0..statuses.row_count() {
        let Some(mut item) = statuses.row_data(index) else {
            continue;
        };
        let Some(status) = StatusKey::parse(item.key.as_str()) else {
            continue;
        };
        let value = count(Reference::Status(status));
        if item.count != value {
            item.count = value;
            statuses.set_row_data(index, item);
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use flequit_model::models::task_projects::subtask::SubTaskTree;
    use flequit_model::types::id_types::{SubTaskId, UserId};

    use super::super::tests::{list_named, project_named, task_named};
    use super::*;

    /// Work (Inbox, Later) and Home (Errands), with list ids pointing back at
    /// their projects as loaded data does.
    fn state() -> SharedState {
        let mut work = project_named(
            "Work",
            vec![
                list_named("Inbox", vec![task_named("Buy milk")]),
                list_named("Later", vec![]),
            ],
        );
        let mut home = project_named("Home", vec![list_named("Errands", vec![])]);
        for project in [&mut work, &mut home] {
            for list in &mut project.task_lists {
                list.project_id = project.id;
            }
        }
        let mut state = SharedState {
            trees: vec![work, home],
            ..SharedState::default()
        };
        state.search_index = build_index(&state);
        state
    }

    fn list_id(state: &SharedState, name: &str) -> String {
        state
            .live_lists()
            .find(|(_, list)| list.name == name)
            .map(|(_, list)| list.id.as_str())
            .expect("the list exists")
    }

    fn search(state: &mut SharedState, canonical: &str) {
        let index = state.search_index.clone();
        state.search.restore(canonical, &index, Lang::En);
    }

    #[test]
    fn the_destination_follows_what_the_query_narrows_to() {
        let mut state = state();
        let later = list_id(&state, "Later");
        let errands = list_id(&state, "Errands");
        let inbox = list_id(&state, "Inbox");

        search(&mut state, &format!("@list:{{{later}}} milk"));
        assert_eq!(default_add_target(&state), Some(later.clone()));

        let home = state.trees[1].id.as_str();
        search(&mut state, &format!("@project:{{{home}}}"));
        assert_eq!(default_add_target(&state), Some(errands.clone()));

        // Two lists OR-ed together narrow to neither.
        search(
            &mut state,
            &format!("@list:{{{later}}} | @list:{{{errands}}}"),
        );
        state.recent_add_targets = vec![errands.clone()];
        assert_eq!(default_add_target(&state), Some(errands));

        state.recent_add_targets.clear();
        assert_eq!(default_add_target(&state), Some(inbox));
    }

    #[test]
    fn a_hand_picked_destination_lasts_until_the_query_moves_the_default() {
        let mut state = state();
        let later = list_id(&state, "Later");
        let errands = list_id(&state, "Errands");

        let default = default_add_target(&state);
        state.add_target_override = Some((errands.clone(), default));
        settle_add_target(&mut state, "");
        assert_eq!(state.add_target.as_deref(), Some(errands.as_str()));

        search(&mut state, &format!("@list:{{{later}}}"));
        settle_add_target(&mut state, "");
        assert_eq!(state.add_target.as_deref(), Some(later.as_str()));
        assert!(state.add_target_override.is_none());
        assert_eq!(state.context_project_id, state.trees[0].id.as_str());
    }

    #[test]
    fn a_task_is_listed_for_a_subtask_that_matches_on_its_own() {
        let mut state = state();
        let now = Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap();
        let subtask = SubTaskTree {
            id: SubTaskId::new(),
            task_id: state.trees[0].task_lists[0].tasks[0].id,
            title: "Check the fridge".into(),
            description: None,
            status: DomainStatus::NotStarted,
            priority: None,
            plan_start_date: None,
            plan_end_date: None,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: None,
            order_index: 0,
            completed: true,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
            assigned_user_ids: Vec::new(),
            tag_ids: Vec::new(),
        };
        let subtask_id = subtask.id.as_str();
        state.trees[0].task_lists[0].tasks[0]
            .sub_tasks
            .push(subtask);
        let context = EvalContext {
            now,
            timezone: DisplayTimezone::Utc,
        };

        let (project, list, task) = state.live_tasks().next().expect("a live task");
        let row = Row::new(&state, project, list, task);
        let query = |text: &str| {
            let mut session = crate::viewmodels::search::SearchSession::default();
            session.restore(text, &state.search_index, Lang::En);
            session.prepared(&state.search_index)
        };

        // The legacy `completed` flag counts as done even while the status lags.
        assert_eq!(
            row.evaluate(&query("@done"), &context),
            (false, vec![subtask_id.clone()])
        );
        // The subtask inherits its parent's text, so both match.
        assert_eq!(
            row.evaluate(&query("milk"), &context),
            (true, vec![subtask_id])
        );
        assert_eq!(
            row.evaluate(&query("@open fridge"), &context),
            (false, vec![])
        );
    }
}
