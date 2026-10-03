//! Connects the search box to the task list, the sidebar and quick add.
//!
//! The query is the only state that decides which tasks are listed. Sidebar
//! items rewrite it, their highlight is derived from it, and the quick-add
//! destination defaults to the list or project it narrows to. The rules live
//! in `viewmodels::search`; this module moves data between them and Slint.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::task::TaskTree;
use flequit_model::models::task_projects::task_list::TaskListTree;
use flequit_model::types::id_types::{ProjectId, TaskListId};
use flequit_model::types::task_types::TaskStatus as DomainStatus;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::{
    SharedState, clear_selection, display_settings, next_order_index, refresh_tags, report_error,
};
use crate::adapters::color::parse_hex;
use crate::adapters::datetime::{
    DateTimeDisplaySettings, DateTimeParts, DisplayTimezone, format_due, from_display_parts,
    to_display_parts,
};
use crate::adapters::task::{Placement, live_children, to_task_item};
use crate::bindings::{
    Actions, AddTargetItem, AddTargetProjectItem, AppState, AppWindow, FilterHighlight, FilterKind,
    I18n, QueryEdit, SearchSuggestion, SearchSuggestionKind, TaskItem,
};
use crate::viewmodels::ordering;
use crate::viewmodels::search::{
    DueSpec, EvalContext, Highlight, ItemKey, Lang, ListEntry, NameIndex, Prepared, ProjectEntry,
    Reference, SearchUnit, Segment, StatusKey, Suggestion, SuggestionKind, UnitStatus, UnitText,
    tag_key,
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
        actions.on_choose_add_target(move |target| {
            {
                let mut guard = state.lock().expect("shared state poisoned");
                let default = default_add_target(&guard);
                guard.add_target_override = Some((target.to_string(), default));
            }
            if let Some(window) = weak.upgrade() {
                publish_add_target(&window, &state);
            }
        });
    }

    // Picking another project lands on that project's default place, the
    // same one a query naming the project would suggest.
    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_choose_add_target_project(move |project_id| {
            {
                let mut guard = state.lock().expect("shared state poisoned");
                let Some(place) = default_in_project(&guard, &project_id) else {
                    return;
                };
                let default = default_add_target(&guard);
                guard.add_target_override = Some((place, default));
            }
            if let Some(window) = weak.upgrade() {
                publish_add_target(&window, &state);
            }
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_set_quick_add_due(move |year, month, day, hour, minute| {
            let Some(window) = weak.upgrade() else { return };
            let display = display_settings(&window, timezone);
            let parts = DateTimeParts {
                year,
                month,
                day,
                hour,
                minute,
            };
            let Some(due) = from_display_parts(parts, display.timezone) else {
                report_error(&weak, "input.validation-failed");
                return;
            };
            state.lock().expect("shared state poisoned").quick_add_due = Some(due);
            publish_quick_add_due(&window, &state, timezone);
        });
    }

    {
        let weak = window.as_weak();
        let state = Arc::clone(state);
        actions.on_clear_quick_add_due(move || {
            state.lock().expect("shared state poisoned").quick_add_due = None;
            if let Some(window) = weak.upgrade() {
                publish_quick_add_due(&window, &state, timezone);
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
    publish_quick_add_due(window, state, timezone);
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

/// The prefix that marks a destination as a project itself rather than a list.
const PROJECT_TARGET_PREFIX: &str = "project:";

/// Where the quick-add field puts a task.
///
/// Stored and passed through Slint as a key: a list id as it is, or the
/// project id behind [`PROJECT_TARGET_PREFIX`]. Keys saved before projects
/// could take tasks are plain list ids and still read the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddTarget<'a> {
    List(&'a str),
    Project(&'a str),
}

impl<'a> AddTarget<'a> {
    fn parse(key: &'a str) -> Self {
        match key.strip_prefix(PROJECT_TARGET_PREFIX) {
            Some(project_id) => Self::Project(project_id),
            None => Self::List(key),
        }
    }
}

fn project_target(project_id: &str) -> String {
    format!("{PROJECT_TARGET_PREFIX}{project_id}")
}

/// Where a new task goes, resolved against the loaded tree.
pub(super) struct Destination {
    pub project_id: ProjectId,
    /// `None` for a task directly under the project.
    pub list_id: Option<TaskListId>,
    /// Places the task after everything already there.
    pub order_index: i32,
}

/// Resolves a destination key to a live list or project.
pub(super) fn destination(state: &SharedState, key: &str) -> Option<Destination> {
    match AddTarget::parse(key) {
        AddTarget::List(id) => state
            .live_lists()
            .find(|(_, list)| list.id.as_str() == id)
            .map(|(tree, list)| Destination {
                project_id: tree.id,
                list_id: Some(list.id),
                order_index: next_order_index(list.tasks.iter().map(|task| task.order_index)),
            }),
        AddTarget::Project(id) => state
            .live_projects()
            .find(|tree| tree.id.as_str() == id)
            .map(|tree| Destination {
                project_id: tree.id,
                list_id: None,
                order_index: next_order_index(tree.tasks.iter().map(|task| task.order_index)),
            }),
    }
}

/// The project that owns a destination, for a live one.
fn project_of_target(state: &SharedState, key: &str) -> Option<String> {
    destination(state, key).map(|destination| destination.project_id.as_str())
}

/// Whether `key` is somewhere a task can be added right now.
fn is_live_target(state: &SharedState, key: &str) -> bool {
    destination(state, key).is_some()
}

/// The destination the query suggests: its one list, or a place in its one
/// project (the last used there, else its first list, else the project
/// itself), else the last place used, else the first there is.
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
        && is_live_target(state, list)
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
    if let [project] = projects.as_slice()
        && let Some(place) = default_in_project(state, project)
    {
        return Some(place);
    }

    state
        .recent_add_targets
        .iter()
        .find(|key| is_live_target(state, key))
        .cloned()
        .or_else(|| state.live_lists().next().map(|(_, list)| list.id.as_str()))
        .or_else(|| {
            state
                .live_projects()
                .next()
                .map(|tree| project_target(&tree.id.as_str()))
        })
}

/// The place a task goes in `project_id` when nothing narrower is chosen: the
/// last place used there, else its first list, else the project itself.
/// `None` for a project that is not live.
fn default_in_project(state: &SharedState, project_id: &str) -> Option<String> {
    let tree = state
        .live_projects()
        .find(|tree| tree.id.as_str() == project_id)?;
    let recent = state
        .recent_add_targets
        .iter()
        .find(|key| project_of_target(state, key).as_deref() == Some(project_id));
    let first_list = state
        .live_lists()
        .find(|(owner, _)| owner.id == tree.id)
        .map(|(_, list)| list.id.as_str());
    Some(
        recent
            .cloned()
            .or(first_list)
            .unwrap_or_else(|| project_target(project_id)),
    )
}

/// Settles the destination and the project the tag manager works on.
fn settle_add_target(state: &mut SharedState, selected_task_id: &str) {
    let default = default_add_target(state);
    let target = match state.add_target_override.take() {
        Some((chosen, replaced)) if replaced == default && is_live_target(state, &chosen) => {
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
        .or_else(|| {
            state
                .add_target
                .as_deref()
                .and_then(|key| project_of_target(state, key))
        })
        .unwrap_or_default();
}

/// Publishes the destination as its two crumbs, with the alternatives each
/// crumb offers: every live project, and the places in the destination's
/// project (none, then its lists).
pub(super) fn publish_add_target(window: &AppWindow, state: &Arc<Mutex<SharedState>>) {
    let app_state = window.global::<AppState>();
    let selected_task_id = app_state.get_selected_task_id().to_string();
    let (key, project, list_name, context, projects, places) = {
        let mut guard = state.lock().expect("shared state poisoned");
        settle_add_target(&mut guard, &selected_task_id);
        let key = guard.add_target.clone().unwrap_or_default();
        let project = destination(&guard, &key).and_then(|destination| {
            guard
                .live_projects()
                .find(|tree| tree.id == destination.project_id)
                .map(|tree| (tree.id, tree.name.clone(), tree.color.clone()))
        });
        let list_name = match AddTarget::parse(&key) {
            AddTarget::List(id) => guard
                .live_lists()
                .find(|(_, list)| list.id.as_str() == id)
                .map(|(_, list)| list.name.clone())
                .unwrap_or_default(),
            AddTarget::Project(_) => String::new(),
        };
        let projects: Vec<AddTargetProjectItem> = guard
            .live_projects()
            .map(|tree| {
                let color = tree.color.as_deref().and_then(parse_hex);
                AddTargetProjectItem {
                    id: SharedString::from(tree.id.as_str()),
                    name: SharedString::from(tree.name.as_str()),
                    color_brush: color.unwrap_or_default().into(),
                    has_color: color.is_some(),
                }
            })
            .collect();
        let places: Vec<AddTargetItem> = project
            .as_ref()
            .map(|(project_id, _, _)| {
                let own = AddTargetItem {
                    key: SharedString::from(project_target(&project_id.as_str())),
                    list_name: SharedString::default(),
                };
                let lists = guard
                    .live_lists()
                    .filter(|(owner, _)| owner.id == *project_id)
                    .map(|(_, list)| AddTargetItem {
                        key: SharedString::from(list.id.as_str()),
                        list_name: SharedString::from(list.name.as_str()),
                    });
                std::iter::once(own).chain(lists).collect()
            })
            .unwrap_or_default();
        (
            key,
            project,
            list_name,
            guard.context_project_id.clone(),
            projects,
            places,
        )
    };
    let (project_id, project_name, color) = project
        .map(|(id, name, color)| (id.as_str(), name, color))
        .unwrap_or_default();
    let color = color.as_deref().and_then(parse_hex);
    app_state.set_add_target(key.into());
    app_state.set_add_target_project_id(project_id.into());
    app_state.set_add_target_project_name(project_name.into());
    app_state.set_add_target_list_name(list_name.into());
    app_state.set_add_target_color(color.unwrap_or_default().into());
    app_state.set_add_target_has_color(color.is_some());
    app_state.set_add_target_projects(ModelRc::new(VecModel::from(projects)));
    app_state.set_add_targets(ModelRc::new(VecModel::from(places)));
    app_state.set_selected_project_id(context.into());
}

/// Publishes the due date the next added task gets. Without one, the picker
/// starts from now.
pub(super) fn publish_quick_add_due(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    timezone: DisplayTimezone,
) {
    let display = display_settings(window, timezone);
    let due = state.lock().expect("shared state poisoned").quick_add_due;
    let now = Utc::now();
    let parts = to_display_parts(due.as_ref().unwrap_or(&now), display.timezone);
    let app_state = window.global::<AppState>();
    app_state.set_quick_add_has_due(due.is_some());
    app_state.set_quick_add_due_label(
        due.map(|due| SharedString::from(format_due(&due, &display)))
            .unwrap_or_default(),
    );
    app_state.set_quick_add_due_year(parts.year);
    app_state.set_quick_add_due_month(parts.month);
    app_state.set_quick_add_due_day(parts.day);
    app_state.set_quick_add_due_hour(parts.hour);
    app_state.set_quick_add_due_minute(parts.minute);
}

/// Records that a task was added to `target`, so it leads the picker and
/// becomes the fallback destination.
pub(super) fn note_task_added(state: &Arc<Mutex<SharedState>>, target: &str, task_id: String) {
    let mut guard = state.lock().expect("shared state poisoned");
    guard.recent_add_targets.retain(|key| key != target);
    guard.recent_add_targets.insert(0, target.to_string());
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

/// One task of a [`Row`], with the owned values its search unit borrows.
struct Node<'a> {
    task: &'a TaskTree,
    /// The tasks above it, top-level first.
    ancestors: Vec<&'a TaskTree>,
    ancestor_texts: Vec<UnitText<'a>>,
    /// Its own tags and those of every task above it.
    tags: Vec<&'a str>,
    assignees: Vec<String>,
    /// Indexes of its live subtasks in [`Row::nodes`].
    children: Vec<usize>,
}

/// A top-level task and every live task below it, in display order.
struct Row<'a> {
    project_id: String,
    /// Empty when the task is directly under the project.
    list_id: String,
    /// The top-level task first, then each subtask after its parent.
    nodes: Vec<Node<'a>>,
    /// Titles and notes of every subtask, for the top-level task's `@subtask:`.
    descendant_texts: Vec<UnitText<'a>>,
}

impl<'a> Row<'a> {
    fn new(
        state: &'a SharedState,
        project: &'a ProjectTree,
        list: Option<&'a TaskListTree>,
        task: &'a TaskTree,
    ) -> Self {
        let mut row = Self {
            project_id: project.id.as_str(),
            list_id: list.map(|list| list.id.as_str()).unwrap_or_default(),
            nodes: Vec::new(),
            descendant_texts: Vec::new(),
        };
        row.push(state, task, Vec::new(), &[]);
        row.descendant_texts = row.nodes[1..]
            .iter()
            .map(|node| UnitText {
                title: &node.task.title,
                notes: node.task.description.as_deref(),
            })
            .collect();
        row
    }

    fn push(
        &mut self,
        state: &'a SharedState,
        task: &'a TaskTree,
        ancestors: Vec<&'a TaskTree>,
        inherited_tags: &[&'a str],
    ) -> usize {
        let mut tags = state.tag_names_of(task);
        tags.extend(inherited_tags.iter().copied());
        let index = self.nodes.len();
        self.nodes.push(Node {
            task,
            ancestor_texts: ancestors
                .iter()
                .map(|ancestor| UnitText {
                    title: &ancestor.title,
                    notes: ancestor.description.as_deref(),
                })
                .collect(),
            ancestors: ancestors.clone(),
            tags: tags.clone(),
            assignees: task
                .assigned_user_ids
                .iter()
                .map(|id| id.as_str())
                .collect(),
            children: Vec::new(),
        });
        let mut below = ancestors;
        below.push(task);
        for child in live_children(task) {
            let child_index = self.push(state, child, below.clone(), &tags);
            self.nodes[index].children.push(child_index);
        }
        index
    }

    fn unit(&self, index: usize) -> SearchUnit<'_> {
        let node = &self.nodes[index];
        SearchUnit {
            project_id: &self.project_id,
            list_id: &self.list_id,
            title: &node.task.title,
            notes: node.task.description.as_deref(),
            ancestors: &node.ancestor_texts,
            descendants: if index == 0 {
                &self.descendant_texts
            } else {
                &[]
            },
            due: node.task.plan_end_date.as_ref(),
            status: unit_status(&node.task.status),
            assignees: &node.assignees,
            tags: &node.tags,
        }
    }

    /// Whether each task of the row matches, by position in [`Self::nodes`].
    fn evaluate(&self, query: &Prepared, context: &EvalContext) -> Vec<bool> {
        (0..self.nodes.len())
            .map(|index| query.matches(&self.unit(index), context))
            .collect()
    }
}

/// What the search makes of one row, and how to publish it.
struct Listing<'s, 'a> {
    row: &'s Row<'a>,
    matches: Vec<bool>,
    /// Whether each task or something below it matches.
    reaches: Vec<bool>,
    filtering: bool,
    hide_completed: bool,
}

impl Listing<'_, '_> {
    /// Appends the row's visible tasks, the top-level one first.
    fn publish(
        &self,
        state: &SharedState,
        now: &DateTime<Utc>,
        display: &DateTimeDisplaySettings,
        items: &mut Vec<TaskItem>,
    ) {
        self.emit(0, self.filtering, state, now, display, items);
    }

    /// `searching` says the task is on the way to a match: the top-level task
    /// while filtering, or a subtask of a task shown only for its subtasks.
    fn emit(
        &self,
        index: usize,
        searching: bool,
        state: &SharedState,
        now: &DateTime<Utc>,
        display: &DateTimeDisplaySettings,
        items: &mut Vec<TaskItem>,
    ) {
        let node = &self.row.nodes[index];
        if index > 0 && self.hide_completed && node.task.status == DomainStatus::Completed {
            return;
        }
        // Listed only for what matches below it: shown receded and opened up.
        let dimmed = searching && !self.matches[index] && self.reaches[index];
        let id = node.task.id.as_str();
        let expanded = dimmed || state.task_ui.is_expanded(&id);
        let placement = Placement {
            list_id: (!self.row.list_id.is_empty()).then_some(self.row.list_id.as_str()),
            ancestors: &node.ancestors,
        };
        let mut item = to_task_item(node.task, placement, expanded, now, display, &node.tags);
        if dimmed {
            item.search_dimmed = true;
            let below = self.below(index).filter(|&at| self.matches[at]).count();
            item.matched_subtask_count = i32::try_from(below).unwrap_or(i32::MAX);
        }
        item.search_match = index > 0 && searching && self.matches[index];
        items.push(item);

        if expanded {
            for &child in &node.children {
                self.emit(child, dimmed, state, now, display, items);
            }
        }
    }

    /// Positions of every task below `index`.
    fn below(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let mut stack: Vec<usize> = self.row.nodes[index].children.clone();
        std::iter::from_fn(move || {
            let at = stack.pop()?;
            stack.extend(self.row.nodes[at].children.iter().copied());
            Some(at)
        })
    }
}

/// Whether each task of `row` or something below it matches, given `matches`.
fn reaches(row: &Row<'_>, matches: &[bool]) -> Vec<bool> {
    let mut reaches = matches.to_vec();
    // Children come after their parent, so walking backwards settles every
    // child before the parent that reads it.
    for index in (0..row.nodes.len()).rev() {
        if row.nodes[index]
            .children
            .iter()
            .any(|&child| reaches[child])
        {
            reaches[index] = true;
        }
    }
    reaches
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
    rows.sort_by(|a, b| ordering::compare(a.nodes[0].task, b.nodes[0].task, guard.task_sort));

    let mut items: Vec<TaskItem> = Vec::new();
    for row in &rows {
        let root = row.nodes[0].task;
        let matches = row.evaluate(&query, &context);
        let reaches = reaches(row, &matches);
        let just_added = guard.just_added.as_deref() == Some(root.id.as_str().as_str());
        if !reaches[0] && !just_added {
            continue;
        }
        if guard.hide_completed_tasks && root.status == DomainStatus::Completed && !just_added {
            continue;
        }
        let listing = Listing {
            row,
            matches,
            reaches,
            // A task just added stays listed as it is, even if it misses.
            filtering: filtering && !just_added,
            hide_completed: guard.hide_completed_tasks,
        };
        listing.publish(&guard, &context.now, &display, &mut items);
    }

    tracing::debug!(rows = items.len(), filtering, "publishing task list");
    let app_state = window.global::<AppState>();
    app_state.set_tasks(ModelRc::new(VecModel::from(items)));
    publish_counts(window, &rows, &context);
    drop(rows);
    drop(guard);
}

/// A task at any depth as the detail pane shows it, built from the cached
/// tree for a task the list does not show.
pub(super) fn task_item_from_tree(
    state: &SharedState,
    task_id: &str,
    display: &DateTimeDisplaySettings,
) -> Option<TaskItem> {
    let (_, task) = state.task_with_project(task_id)?;
    if task.deleted {
        return None;
    }
    let ancestors = state.ancestors_of(task_id);
    if ancestors.iter().any(|ancestor| ancestor.deleted) {
        return None;
    }
    let root = ancestors.first().copied().unwrap_or(task);
    let list_id = root.list_id.map(|id| id.as_str());
    let mut tags = state.tag_names_of(task);
    for ancestor in &ancestors {
        tags.extend(state.tag_names_of(ancestor));
    }
    let placement = Placement {
        list_id: list_id.as_deref(),
        ancestors: &ancestors,
    };
    Some(to_task_item(
        task,
        placement,
        state.task_ui.is_expanded(task_id),
        &Utc::now(),
        display,
        &tags,
    ))
}

/// Counts, for each due and state button, the rows its query alone would list.
fn publish_counts(window: &AppWindow, rows: &[Row<'_>], context: &EvalContext) {
    let count = |reference: Reference| {
        let query = Prepared::single(reference);
        let listed = rows
            .iter()
            .filter(|row| {
                row.evaluate(&query, context)
                    .into_iter()
                    .any(|matched| matched)
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
    fn a_project_without_lists_takes_tasks_itself() {
        let mut state = state();
        let mut loose = project_named("Loose", Vec::new());
        loose.tasks.push(task_named("Read"));
        let loose_id = loose.id.as_str();
        state.trees.push(loose);
        state.search_index = build_index(&state);

        search(&mut state, &format!("@project:{{{loose_id}}}"));
        let target = default_add_target(&state).expect("a destination");
        assert_eq!(target, project_target(&loose_id));
        let resolved = destination(&state, &target).expect("a live destination");
        assert_eq!(resolved.list_id, None);
        assert_eq!(resolved.order_index, 1);

        // A project chosen before stays the default when the query has none.
        search(&mut state, "");
        state.recent_add_targets = vec![target.clone()];
        assert_eq!(default_add_target(&state), Some(target));
    }

    #[test]
    fn picking_a_project_lands_on_its_last_place_else_its_first_list_else_itself() {
        let mut state = state();
        let work = state.trees[0].id.as_str();
        let later = list_id(&state, "Later");
        let inbox = list_id(&state, "Inbox");
        let empty = project_named("Empty", Vec::new());
        let empty_id = empty.id.as_str();
        state.trees.push(empty);

        assert_eq!(default_in_project(&state, &work), Some(inbox));
        state.recent_add_targets = vec![later.clone()];
        assert_eq!(default_in_project(&state, &work), Some(later));
        assert_eq!(
            default_in_project(&state, &empty_id),
            Some(project_target(&empty_id))
        );
        assert_eq!(default_in_project(&state, "not-a-project"), None);
    }

    #[test]
    fn an_old_destination_key_is_a_list_id() {
        let state = state();
        let inbox = list_id(&state, "Inbox");
        let resolved = destination(&state, &inbox).expect("the list");
        assert_eq!(resolved.list_id.map(|id| id.as_str()), Some(inbox));
        assert!(destination(&state, "project:not-a-project").is_none());
    }

    /// Buy milk › Check the fridge (completed) › Wipe the shelf
    fn nested() -> SharedState {
        let mut state = state();
        let root = &mut state.trees[0].task_lists[0].tasks[0];
        let mut child = TaskTree {
            list_id: None,
            parent_task_id: Some(root.id),
            status: DomainStatus::Completed,
            ..task_named("Check the fridge")
        };
        child.sub_tasks.push(TaskTree {
            list_id: None,
            parent_task_id: Some(child.id),
            ..task_named("Wipe the shelf")
        });
        root.sub_tasks.push(child);
        state
    }

    /// The rows the task list shows for `query`, as (title, depth, dimmed, marked).
    fn listed(state: &SharedState, query: &str) -> Vec<(String, i32, bool, bool)> {
        let context = EvalContext {
            now: Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap(),
            timezone: DisplayTimezone::Utc,
        };
        let display = DateTimeDisplaySettings {
            timezone: DisplayTimezone::Utc,
            format: "%Y-%m-%d %H:%M".to_string(),
        };
        let mut session = crate::viewmodels::search::SearchSession::default();
        session.restore(query, &state.search_index, Lang::En);
        let prepared = session.prepared(&state.search_index);

        let mut items = Vec::new();
        for (project, list, task) in state.live_tasks() {
            let row = Row::new(state, project, list, task);
            let matches = row.evaluate(&prepared, &context);
            let reaches = reaches(&row, &matches);
            if !reaches[0] {
                continue;
            }
            Listing {
                row: &row,
                matches,
                reaches,
                filtering: !prepared.is_empty(),
                hide_completed: false,
            }
            .publish(state, &context.now, &display, &mut items);
        }
        items
            .into_iter()
            .map(|item| {
                (
                    item.title.to_string(),
                    item.depth,
                    item.search_dimmed,
                    item.search_match,
                )
            })
            .collect()
    }

    fn row(title: &str, depth: i32, dimmed: bool, marked: bool) -> (String, i32, bool, bool) {
        (title.to_string(), depth, dimmed, marked)
    }

    #[test]
    fn subtasks_are_rows_of_their_own_once_opened() {
        let mut state = nested();
        assert_eq!(listed(&state, ""), [row("Buy milk", 0, false, false)]);

        let root = state.trees[0].task_lists[0].tasks[0].id.as_str();
        state.task_ui.expand(&root);
        assert_eq!(
            listed(&state, ""),
            [
                row("Buy milk", 0, false, false),
                row("Check the fridge", 1, false, false),
            ]
        );
    }

    #[test]
    fn a_task_is_listed_for_a_subtask_that_matches_on_its_own() {
        let state = nested();

        // Only the subtask is done: the task is shown for it, receded and open.
        assert_eq!(
            listed(&state, "@done"),
            [
                row("Buy milk", 0, true, false),
                row("Check the fridge", 1, false, true),
            ]
        );
        // The subtasks inherit the task's text, so the task itself matches and
        // nothing below it is marked.
        assert_eq!(listed(&state, "milk"), [row("Buy milk", 0, false, false)]);
        assert!(listed(&state, "@done shelf").is_empty());
    }

    #[test]
    fn a_deep_match_opens_the_way_down_to_it() {
        let state = nested();

        assert_eq!(
            listed(&state, "wipe"),
            [
                row("Buy milk", 0, true, false),
                row("Check the fridge", 1, true, false),
                row("Wipe the shelf", 2, false, true),
            ]
        );
    }

    #[test]
    fn a_hidden_subtask_still_opens_in_the_pane_with_its_way_back_up() {
        let state = nested();
        let display = DateTimeDisplaySettings {
            timezone: DisplayTimezone::Utc,
            format: "%Y-%m-%d %H:%M".to_string(),
        };
        let root = &state.trees[0].task_lists[0].tasks[0];
        let grandchild = root.sub_tasks[0].sub_tasks[0].id.as_str();

        let item = task_item_from_tree(&state, &grandchild, &display).expect("loaded");

        assert_eq!(item.depth, 2);
        assert_eq!(item.parent_id, root.sub_tasks[0].id.as_str());
        assert_eq!(item.list_id, root.list_id.unwrap().as_str());
        let ancestors: Vec<String> = item.ancestors.iter().map(|a| a.title.to_string()).collect();
        assert_eq!(ancestors, ["Buy milk", "Check the fridge"]);
    }
}
