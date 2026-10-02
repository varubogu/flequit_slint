//! Undoable task deletion.
//!
//! Deleting a task hides it at once and shows a toast offering to undo. Storage
//! is only written once that offer ends: the toast times out or is dismissed,
//! another deletion takes its place, or the app suspends or quits. "Undo" then
//! simply cancels the write that has not happened yet.
//!
//! A task is deleted with every subtask below it, at any depth. Deleting and
//! then restoring from the trash would not be exact: storage drops the tag
//! links, assignments and repeat rule along with the tasks, while a trash
//! restore brings back the tasks alone.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use flequit_core::InfrastructureRepositoriesTrait;
use flequit_core::facades::task_facades;
use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::types::id_types::{ProjectId, TaskId, UserId};
use flequit_platform::{NotificationId, Platform};
use slint::{ComponentHandle, Model, SharedString, Weak};
use tokio::runtime::Handle;

use super::{
    SharedState, clear_selection, indexed_task_row, open_task, refresh_projects, refresh_tasks,
    reload_projects, report_error, select_task,
};
use crate::UiError;
use crate::adapters::datetime::DisplayTimezone;
use crate::bindings::{Actions, AppState, AppWindow, Pane};

/// How long the toast offers to undo before the task is deleted from storage.
const UNDO_WINDOW: Duration = Duration::from_secs(5);

/// A task that is hidden but still in storage.
#[derive(Debug, Clone)]
pub(super) struct PendingDeletion {
    /// Tells this deletion's timer apart from a later deletion's.
    token: u64,
    task_id: TaskId,
    project_id: ProjectId,
    user_id: UserId,
    /// Scheduled notifications to cancel once the task and its subtasks are
    /// gone, by the task each belongs to.
    reminders: Vec<(TaskId, DateTime<Utc>)>,
}

/// Everything the handlers and the background commit need.
struct Deps<R> {
    weak: Weak<AppWindow>,
    state: Arc<Mutex<SharedState>>,
    repositories: Arc<R>,
    platform: Arc<dyn Platform>,
    runtime: Handle,
    timezone: DisplayTimezone,
}

impl<R> Clone for Deps<R> {
    fn clone(&self) -> Self {
        Self {
            weak: self.weak.clone(),
            state: Arc::clone(&self.state),
            repositories: Arc::clone(&self.repositories),
            platform: Arc::clone(&self.platform),
            runtime: self.runtime.clone(),
            timezone: self.timezone,
        }
    }
}

pub(super) fn bind<R>(
    window: &AppWindow,
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    platform: &Arc<dyn Platform>,
    runtime: &Handle,
    timezone: DisplayTimezone,
) where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let deps = Deps {
        weak: window.as_weak(),
        state: Arc::clone(state),
        repositories: Arc::clone(repositories),
        platform: Arc::clone(platform),
        runtime: runtime.clone(),
        timezone,
    };
    let actions = window.global::<Actions>();

    {
        let deps = deps.clone();
        actions.on_delete_task(move |task_id| begin(&deps, &task_id));
    }
    {
        let deps = deps.clone();
        actions.on_undo_delete_task(move || undo(&deps));
    }
    actions.on_dismiss_undo_delete(move || {
        if let Some(window) = deps.weak.upgrade() {
            window.global::<AppState>().set_undo_delete_visible(false);
        }
        let pending = deps
            .state
            .lock()
            .expect("shared state poisoned")
            .pending_deletion
            .take();
        if let Some(pending) = pending {
            spawn_commit(&deps, pending);
        }
    });
}

/// Keeps the pending task hidden in a freshly loaded tree.
///
/// Storage still has it, so every reload would otherwise bring it back.
pub(super) fn hide_pending(state: &mut SharedState) {
    if let Some(task_id) = state
        .pending_deletion
        .as_ref()
        .map(|pending| pending.task_id)
    {
        set_hidden(&mut state.trees, &task_id, true);
    }
}

/// Deletes the pending task from storage now, without touching the UI.
///
/// For when the window is going away (quit) or may never come back (suspend):
/// the undo offer cannot outlive either.
pub(super) fn flush<R>(
    state: &Arc<Mutex<SharedState>>,
    repositories: &Arc<R>,
    platform: &Arc<dyn Platform>,
) -> impl Future<Output = ()> + Send + 'static
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let pending = state
        .lock()
        .expect("shared state poisoned")
        .pending_deletion
        .take();
    let repositories = Arc::clone(repositories);
    let platform = Arc::clone(platform);
    async move {
        if let Some(pending) = pending
            && let Err(error) = delete_from_storage(&pending, &repositories, &platform).await
        {
            tracing::error!(%error, task_id = %pending.task_id, "failed to delete task");
        }
    }
}

fn begin<R>(deps: &Deps<R>, task_id: &SharedString)
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let Some(window) = deps.weak.upgrade() else {
        return;
    };
    let Ok(parsed_id) = TaskId::try_from_str(task_id.as_str()) else {
        report_error(&deps.weak, "input.malformed-id");
        return;
    };

    let (token, title, parent, previous) = {
        let mut guard = deps.state.lock().expect("shared state poisoned");
        let found = guard.task_with_project(task_id).map(|(project, task)| {
            let reminders: Vec<(TaskId, DateTime<Utc>)> = task
                .walk()
                .filter(|task| !task.deleted)
                .flat_map(|task| task.reminders.iter().map(|at| (task.id, *at)))
                .collect();
            (
                project.id,
                task.title.clone(),
                task.parent_task_id,
                reminders,
            )
        });
        let (Some((project_id, title, parent, reminders)), Some(user_id)) =
            (found, guard.current_user)
        else {
            drop(guard);
            tracing::error!(%task_id, "cannot delete a task: unknown project or user");
            report_error(&deps.weak, "task.delete-failed");
            return;
        };

        guard.deletion_token = guard.deletion_token.wrapping_add(1);
        let token = guard.deletion_token;
        // Only one deletion can be undone at a time: the toast has room for
        // one, so the one it no longer shows goes to storage now.
        let previous = guard.pending_deletion.replace(PendingDeletion {
            token,
            task_id: parsed_id,
            project_id,
            user_id,
            reminders,
        });
        set_hidden(&mut guard.trees, &parsed_id, true);
        (token, title, parent, previous)
    };
    if let Some(previous) = previous {
        spawn_commit(deps, previous);
    }

    let app_state = window.global::<AppState>();
    let was_selected = app_state.get_selected_task_id() == *task_id;
    let removed_index = indexed_task_row(&window, task_id).map(|(index, _)| index);
    refresh_projects(&window, &deps.state);
    refresh_tasks(&window, &deps.state, deps.timezone);
    if was_selected {
        match parent {
            // A subtask deleted from its own pane goes back to its parent.
            Some(parent) if removed_index.is_none() => open_task(
                &window,
                &deps.state,
                &SharedString::from(parent.as_str()),
                deps.timezone,
            ),
            _ => select_neighbour(&window, removed_index),
        }
    }

    app_state.set_undo_delete_title(title.into());
    app_state.set_undo_delete_visible(true);

    let deps = deps.clone();
    deps.runtime.clone().spawn(async move {
        tokio::time::sleep(UNDO_WINDOW).await;
        let expired = {
            let mut guard = deps.state.lock().expect("shared state poisoned");
            match &guard.pending_deletion {
                Some(pending) if pending.token == token => guard.pending_deletion.take(),
                _ => None,
            }
        };
        if let Some(pending) = expired {
            hide_toast_unless_replaced(&deps);
            commit(&deps, pending).await;
        }
    });
}

fn undo<R>(deps: &Deps<R>) {
    let Some(window) = deps.weak.upgrade() else {
        return;
    };
    let app_state = window.global::<AppState>();
    app_state.set_undo_delete_visible(false);

    let task_id = {
        let mut guard = deps.state.lock().expect("shared state poisoned");
        let Some(pending) = guard.pending_deletion.take() else {
            // The timer got there first; the task is already on its way out.
            return;
        };
        set_hidden(&mut guard.trees, &pending.task_id, false);
        SharedString::from(pending.task_id.as_str())
    };

    refresh_projects(&window, &deps.state);
    refresh_tasks(&window, &deps.state, deps.timezone);
    // The query may have changed since; the task is back either way.
    if indexed_task_row(&window, &task_id).is_some() {
        select_task(&window, &task_id);
        app_state.set_active_pane(Pane::List);
    }
}

/// Moves the selection to the row that took the deleted one's place, so the
/// keyboard can carry on deleting or navigating.
fn select_neighbour(window: &AppWindow, removed_index: Option<usize>) {
    let app_state = window.global::<AppState>();
    let tasks = app_state.get_tasks();
    let next = removed_index
        .filter(|_| tasks.row_count() > 0)
        .and_then(|index| tasks.row_data(index.min(tasks.row_count() - 1)));
    match next {
        Some(task) => {
            select_task(window, &task.id);
            app_state.set_active_pane(Pane::List);
        }
        None => clear_selection(window),
    }
}

fn hide_toast_unless_replaced<R>(deps: &Deps<R>) {
    let state = Arc::clone(&deps.state);
    let posted = deps.weak.upgrade_in_event_loop(move |window| {
        if state
            .lock()
            .expect("shared state poisoned")
            .pending_deletion
            .is_none()
        {
            window.global::<AppState>().set_undo_delete_visible(false);
        }
    });
    if let Err(error) = posted {
        tracing::error!(%error, "could not close the undo toast");
    }
}

fn spawn_commit<R>(deps: &Deps<R>, pending: PendingDeletion)
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    let deps_for_task = deps.clone();
    deps.runtime
        .spawn(async move { commit(&deps_for_task, pending).await });
}

/// Deletes the task from storage, then reloads so the list matches storage
/// again. On failure the reload is what brings the task back into view.
async fn commit<R>(deps: &Deps<R>, pending: PendingDeletion)
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    if let Err(error) = delete_from_storage(&pending, &deps.repositories, &deps.platform).await {
        tracing::error!(%error, task_id = %pending.task_id, "failed to delete task");
        report_error(&deps.weak, error.code());
    }
    reload_projects(&deps.weak, &deps.state, &deps.repositories, deps.timezone).await;
}

async fn delete_from_storage<R>(
    pending: &PendingDeletion,
    repositories: &Arc<R>,
    platform: &Arc<dyn Platform>,
) -> Result<(), UiError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync + 'static,
{
    task_facades::delete_task(
        repositories.as_ref(),
        &pending.project_id,
        &pending.task_id,
        &pending.user_id,
        &Utc::now(),
    )
    .await
    .map_err(UiError::from)?;

    for (task_id, reminder) in &pending.reminders {
        let task_id = task_id.as_str();
        let id = NotificationId::scheduled(&task_id, reminder);
        if let Err(error) = platform.cancel_notification(&id).await {
            tracing::warn!(%error, %task_id, "failed to cancel deleted task reminder");
        }
    }
    Ok(())
}

/// Marks the task deleted in the cached tree only, which is all the views read.
/// Its subtasks go out of view with it.
fn set_hidden(trees: &mut [ProjectTree], task_id: &TaskId, hidden: bool) {
    if let Some(task) = trees
        .iter_mut()
        .find_map(|tree| tree.find_task_mut(task_id))
    {
        task.deleted = hidden;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{list_named, project_named, task_named};
    use super::*;
    use flequit_model::models::task_projects::task::TaskTree;

    fn state_with(titles: &[&str]) -> SharedState {
        let tasks = titles.iter().map(|title| task_named(title)).collect();
        SharedState {
            trees: vec![project_named("Work", vec![list_named("Inbox", tasks)])],
            ..SharedState::default()
        }
    }

    fn live_titles(state: &SharedState) -> Vec<String> {
        state
            .live_tasks()
            .map(|(_, _, task)| task.title.clone())
            .collect()
    }

    fn pending_for(state: &SharedState, index: usize) -> PendingDeletion {
        let tree = &state.trees[0];
        PendingDeletion {
            token: 1,
            task_id: tree.task_lists[0].tasks[index].id,
            project_id: tree.id,
            user_id: UserId::new(),
            reminders: Vec::new(),
        }
    }

    #[test]
    fn a_hidden_task_leaves_the_list_and_comes_back_on_undo() {
        let mut state = state_with(&["Buy milk", "Wash car"]);
        let task_id = state.trees[0].task_lists[0].tasks[0].id;

        set_hidden(&mut state.trees, &task_id, true);
        assert_eq!(live_titles(&state), ["Wash car"]);

        set_hidden(&mut state.trees, &task_id, false);
        assert_eq!(live_titles(&state), ["Buy milk", "Wash car"]);
    }

    #[test]
    fn a_reload_does_not_bring_back_a_pending_deletion() {
        let mut state = state_with(&["Buy milk", "Wash car"]);
        state.pending_deletion = Some(pending_for(&state, 1));

        // A reload replaces the trees with what storage still holds.
        let reloaded = state.trees.clone();
        state.trees = reloaded;
        hide_pending(&mut state);

        assert_eq!(live_titles(&state), ["Buy milk"]);
    }

    #[test]
    fn nothing_is_hidden_without_a_pending_deletion() {
        let mut state = state_with(&["Buy milk"]);
        hide_pending(&mut state);
        assert_eq!(live_titles(&state), ["Buy milk"]);
    }

    #[test]
    fn a_hidden_subtask_takes_its_own_subtasks_out_of_view() {
        let mut state = state_with(&["Buy milk"]);
        let root = &mut state.trees[0].task_lists[0].tasks[0];
        let mut child = TaskTree {
            id: TaskId::new(),
            list_id: None,
            parent_task_id: Some(root.id),
            title: "Check the fridge".to_string(),
            ..root.clone()
        };
        child.sub_tasks.push(TaskTree {
            id: TaskId::new(),
            parent_task_id: Some(child.id),
            title: "Wipe the shelf".to_string(),
            ..child.clone()
        });
        let child_id = child.id;
        root.sub_tasks.push(child);

        set_hidden(&mut state.trees, &child_id, true);

        let root = &state.trees[0].task_lists[0].tasks[0];
        assert_eq!(live_titles(&state), ["Buy milk"]);
        assert_eq!(crate::adapters::task::live_children(root).count(), 0);
    }
}
