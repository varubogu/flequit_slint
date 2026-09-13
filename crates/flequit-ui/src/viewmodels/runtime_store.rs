use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use flequit_model::models::task_projects::project::ProjectTree;
use flequit_model::models::task_projects::task::{PartialTask, TaskTree};
use flequit_model::types::id_types::TaskId;
use uuid::Uuid;

mod task_patch;
#[cfg(test)]
mod tests;

use task_patch::apply_task_patch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TaskMutationId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationResolution {
    Committed,
    Failed,
}

#[derive(Debug, Clone)]
struct PendingTaskMutation {
    id: TaskMutationId,
    base_revision: u64,
    revision: u64,
    patch: PartialTask,
    resolution: Option<MutationResolution>,
}

#[derive(Debug, Clone)]
struct TaskMutationQueue {
    base: TaskTree,
    pending: Vec<PendingTaskMutation>,
}

/// Runtime source of truth for the loaded project hierarchy.
///
/// Slint models are projections of this store. Task mutations are retained in
/// order until every older mutation has resolved, so a failed mutation can be
/// removed and newer changes can be replayed over the last committed value.
#[derive(Debug, Default)]
pub(super) struct RuntimeStore {
    trees: Vec<ProjectTree>,
    task_revisions: HashMap<TaskId, u64>,
    task_mutations: HashMap<TaskId, TaskMutationQueue>,
}

impl RuntimeStore {
    pub(super) fn replace_trees(&mut self, trees: Vec<ProjectTree>) {
        self.trees = trees;
        let task_ids: Vec<TaskId> = self.task_mutations.keys().copied().collect();
        for task_id in task_ids {
            if let Some(loaded) = find_task(&self.trees, &task_id).cloned()
                && let Some(queue) = self.task_mutations.get_mut(&task_id)
            {
                queue.base = loaded;
            }
            self.rebuild_task(&task_id);
        }
    }

    pub(super) fn begin_task_mutation(
        &mut self,
        task_id: &TaskId,
        patch: PartialTask,
    ) -> Option<TaskMutationId> {
        let current = find_task(&self.trees, task_id)?.clone();
        let base_revision = self.task_revisions.get(task_id).copied().unwrap_or(0);
        let revision = base_revision.wrapping_add(1);
        self.task_revisions.insert(*task_id, revision);

        let id = TaskMutationId(Uuid::new_v4());
        let queue = self
            .task_mutations
            .entry(*task_id)
            .or_insert_with(|| TaskMutationQueue {
                base: current,
                pending: Vec::new(),
            });
        queue.pending.push(PendingTaskMutation {
            id,
            base_revision,
            revision,
            patch,
            resolution: None,
        });
        self.rebuild_task(task_id);
        Some(id)
    }

    pub(super) fn resolve_task_mutation(
        &mut self,
        task_id: &TaskId,
        mutation_id: TaskMutationId,
        succeeded: bool,
    ) -> bool {
        let Some(queue) = self.task_mutations.get_mut(task_id) else {
            return false;
        };
        let Some(mutation) = queue
            .pending
            .iter_mut()
            .find(|mutation| mutation.id == mutation_id)
        else {
            return false;
        };
        mutation.resolution = Some(if succeeded {
            MutationResolution::Committed
        } else {
            MutationResolution::Failed
        });

        while queue
            .pending
            .first()
            .is_some_and(|mutation| mutation.resolution.is_some())
        {
            let mutation = queue.pending.remove(0);
            debug_assert_eq!(
                mutation.revision,
                mutation.base_revision.wrapping_add(1),
                "task mutations must advance one runtime revision"
            );
            if mutation.resolution == Some(MutationResolution::Committed) {
                apply_task_patch(&mut queue.base, &mutation.patch);
            }
        }

        self.rebuild_task(task_id);
        if self
            .task_mutations
            .get(task_id)
            .is_some_and(|queue| queue.pending.is_empty())
        {
            self.task_mutations.remove(task_id);
        }
        true
    }

    fn rebuild_task(&mut self, task_id: &TaskId) {
        let Some(queue) = self.task_mutations.get(task_id) else {
            return;
        };
        let mut rebuilt = queue.base.clone();
        for mutation in &queue.pending {
            if mutation.resolution != Some(MutationResolution::Failed) {
                apply_task_patch(&mut rebuilt, &mutation.patch);
            }
        }
        if let Some(task) = find_task_mut(&mut self.trees, task_id) {
            *task = rebuilt;
        }
    }
}

impl Deref for RuntimeStore {
    type Target = Vec<ProjectTree>;

    fn deref(&self) -> &Self::Target {
        &self.trees
    }
}

impl DerefMut for RuntimeStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.trees
    }
}

impl From<Vec<ProjectTree>> for RuntimeStore {
    fn from(trees: Vec<ProjectTree>) -> Self {
        Self {
            trees,
            ..Self::default()
        }
    }
}

fn find_task<'a>(trees: &'a [ProjectTree], task_id: &TaskId) -> Option<&'a TaskTree> {
    trees
        .iter()
        .flat_map(|project| project.task_lists.iter())
        .flat_map(|list| list.tasks.iter())
        .find(|task| task.id == *task_id)
}

fn find_task_mut<'a>(trees: &'a mut [ProjectTree], task_id: &TaskId) -> Option<&'a mut TaskTree> {
    trees
        .iter_mut()
        .flat_map(|project| project.task_lists.iter_mut())
        .flat_map(|list| list.tasks.iter_mut())
        .find(|task| task.id == *task_id)
}
