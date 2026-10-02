//! プロジェクトのタスクを木に組み立てる
//!
//! タスクはプロジェクト単位で平らに保存され、親子は `parent_task_id`、
//! リストへの所属は最上位のタスクの `list_id` だけが表す。ここでそれを
//! 「タスクリストごとの最上位のタスク」と「プロジェクト直下の最上位のタスク」の
//! 木にまとめる。
//!
//! 保存されたデータが壊れていても木は必ず作る。親が見つからないタスクと、
//! 親をたどると自分に戻る（循環している）タスクは最上位として扱う。データは書き換えない。

use std::collections::{HashMap, HashSet};

use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::task::{Task, TaskTree};
use flequit_model::models::task_projects::task_list::{TaskList, TaskListTree};
use flequit_model::types::id_types::{ProjectId, RecurrenceRuleId, TaskId, TaskListId};
use flequit_repository::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::service_error::ServiceError;

use crate::InfrastructureRepositoriesTrait;
use crate::services::task_list_service;

/// プロジェクトのタスクの木
#[derive(Debug, Clone, Default)]
pub struct ProjectTasks {
    /// タスクリスト（表示順序で並ぶ）と、それぞれに属する最上位のタスク
    pub task_lists: Vec<TaskListTree>,
    /// リストに属さない最上位のタスク（プロジェクト直下）
    pub tasks: Vec<TaskTree>,
}

/// プロジェクトのタスクリストとタスクを読み、木に組み立てる
pub async fn load_project_tasks<R>(
    repositories: &R,
    project_id: &ProjectId,
) -> Result<ProjectTasks, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let task_lists = task_list_service::list_task_lists(repositories, project_id).await?;
    let tasks = repositories.tasks().find_all(project_id).await?;

    let rules: HashMap<RecurrenceRuleId, RecurrenceRule> = repositories
        .recurrence_rules()
        .find_all(project_id)
        .await?
        .into_iter()
        .map(|rule| (rule.id, rule))
        .collect();
    let mut relations = repositories.task_recurrences().find_all(project_id).await?;
    // 旧データに重複がある場合も、最新の関連付けが残るよう古い順に入れる
    relations.sort_by_key(|relation| relation.updated_at);
    let rule_of: HashMap<TaskId, RecurrenceRuleId> = relations
        .into_iter()
        .map(|relation| (relation.task_id, relation.recurrence_rule_id))
        .collect();

    let trees = tasks
        .into_iter()
        .map(|task| {
            let rule = rule_of.get(&task.id).and_then(|id| rules.get(id)).cloned();
            to_tree(task, rule)
        })
        .collect();
    Ok(assemble(task_lists, trees))
}

/// 子を持たない `TaskTree` を作る（繰り返しルールは関連テーブルから引いたもの）
pub fn to_tree(task: Task, recurrence_rule: Option<RecurrenceRule>) -> TaskTree {
    TaskTree {
        id: task.id,
        project_id: task.project_id,
        list_id: task.list_id,
        parent_task_id: task.parent_task_id,
        title: task.title,
        description: task.description,
        status: task.status,
        priority: task.priority,
        plan_start_date: task.plan_start_date,
        plan_end_date: task.plan_end_date,
        do_start_date: task.do_start_date,
        do_end_date: task.do_end_date,
        is_range_date: task.is_range_date,
        recurrence_rule: recurrence_rule.or(task.recurrence_rule),
        reminders: task.reminders,
        assigned_user_ids: task.assigned_user_ids,
        order_index: task.order_index,
        is_archived: task.is_archived,
        created_at: task.created_at,
        updated_at: task.updated_at,
        deleted: task.deleted,
        updated_by: task.updated_by,
        sub_tasks: Vec::new(),
        tag_ids: task.tag_ids,
    }
}

/// 平らなタスクを木にまとめる
///
/// `tasks` の `sub_tasks` は無視する（空であること）。兄弟は表示順序で並べる。
pub fn assemble(task_lists: Vec<TaskList>, tasks: Vec<TaskTree>) -> ProjectTasks {
    let parent_of = usable_parents(&tasks);
    let promoted = cycle_breakers(&parent_of);

    let mut children: HashMap<TaskId, Vec<TaskTree>> = HashMap::new();
    let mut roots = Vec::new();
    for task in tasks {
        let parent = parent_of
            .get(&task.id)
            .copied()
            .filter(|_| !promoted.contains(&task.id));
        match parent {
            Some(parent) => children.entry(parent).or_default().push(task),
            None => roots.push(task),
        }
    }

    let mut roots: Vec<TaskTree> = roots
        .into_iter()
        .map(|root| attach_children(root, &mut children))
        .collect();
    sort_siblings(&mut roots);

    let list_ids: HashSet<TaskListId> = task_lists.iter().map(|list| list.id).collect();
    let mut by_list: HashMap<TaskListId, Vec<TaskTree>> = HashMap::new();
    let mut unlisted = Vec::new();
    for root in roots {
        // 子として保存されていたタスクが最上位に繰り上がったときは、
        // `list_id` を持たないのでプロジェクト直下に置かれる
        match root.list_id.filter(|id| list_ids.contains(id)) {
            Some(list_id) => by_list.entry(list_id).or_default().push(root),
            None => unlisted.push(root),
        }
    }

    let task_lists = task_lists
        .into_iter()
        .map(|list| TaskListTree {
            tasks: by_list.remove(&list.id).unwrap_or_default(),
            id: list.id,
            project_id: list.project_id,
            name: list.name,
            description: list.description,
            color: list.color,
            order_index: list.order_index,
            is_archived: list.is_archived,
            created_at: list.created_at,
            updated_at: list.updated_at,
            deleted: list.deleted,
            updated_by: list.updated_by,
        })
        .collect();

    ProjectTasks {
        task_lists,
        tasks: unlisted,
    }
}

/// 親として使える参照（同じプロジェクトに存在し、自分自身ではない親）
fn usable_parents(tasks: &[TaskTree]) -> HashMap<TaskId, TaskId> {
    let ids: HashSet<TaskId> = tasks.iter().map(|task| task.id).collect();
    tasks
        .iter()
        .filter_map(|task| {
            task.parent_task_id
                .filter(|parent| *parent != task.id && ids.contains(parent))
                .map(|parent| (task.id, parent))
        })
        .collect()
}

/// 親をたどると戻ってくる輪ごとに、最上位へ繰り上げるタスクを 1 つ選ぶ
///
/// 選ぶのは輪の中で ID が最小のタスク。どの順で調べても同じタスクが選ばれる。
fn cycle_breakers(parent_of: &HashMap<TaskId, TaskId>) -> HashSet<TaskId> {
    #[derive(Clone, Copy, PartialEq)]
    enum Visit {
        OnPath,
        Done,
    }

    let mut visits: HashMap<TaskId, Visit> = HashMap::new();
    let mut breakers = HashSet::new();
    for &start in parent_of.keys() {
        let mut path = Vec::new();
        let mut current = Some(start);
        while let Some(id) = current {
            match visits.get(&id) {
                Some(Visit::Done) => break,
                Some(Visit::OnPath) => {
                    if let Some(at) = path.iter().position(|seen| *seen == id)
                        && let Some(lowest) = path[at..].iter().min()
                    {
                        breakers.insert(*lowest);
                    }
                    break;
                }
                None => {}
            }
            visits.insert(id, Visit::OnPath);
            path.push(id);
            current = parent_of.get(&id).copied();
        }
        for id in path {
            visits.insert(id, Visit::Done);
        }
    }
    breakers
}

fn attach_children(mut task: TaskTree, children: &mut HashMap<TaskId, Vec<TaskTree>>) -> TaskTree {
    let mut own = children.remove(&task.id).unwrap_or_default();
    sort_siblings(&mut own);
    task.sub_tasks = own
        .into_iter()
        .map(|child| attach_children(child, children))
        .collect();
    task
}

/// 兄弟を表示順序で並べる。同じ順序なら作成順、最後に ID で決める
fn sort_siblings(tasks: &mut [TaskTree]) {
    tasks.sort_by(|a, b| {
        a.order_index
            .cmp(&b.order_index)
            .then(a.created_at.cmp(&b.created_at))
            .then(a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use flequit_model::types::id_types::UserId;
    use flequit_model::types::task_types::TaskStatus;

    fn list(project_id: ProjectId, order_index: i32) -> TaskList {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        TaskList {
            id: TaskListId::new(),
            project_id,
            name: format!("list {order_index}"),
            description: None,
            color: None,
            order_index,
            is_archived: false,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    fn task(
        project_id: ProjectId,
        list_id: Option<TaskListId>,
        parent_task_id: Option<TaskId>,
        order_index: i32,
    ) -> TaskTree {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        TaskTree {
            id: TaskId::new(),
            project_id,
            list_id,
            parent_task_id,
            title: format!("task {order_index}"),
            description: None,
            status: TaskStatus::NotStarted,
            priority: 0,
            plan_start_date: None,
            plan_end_date: None,
            do_start_date: None,
            do_end_date: None,
            is_range_date: None,
            recurrence_rule: None,
            reminders: Vec::new(),
            assigned_user_ids: Vec::new(),
            order_index,
            is_archived: false,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
            sub_tasks: Vec::new(),
            tag_ids: Vec::new(),
        }
    }

    #[test]
    fn roots_go_to_their_list_or_to_the_project() {
        let project = ProjectId::new();
        let inbox = list(project, 0);
        let in_list = task(project, Some(inbox.id), None, 0);
        let unlisted = task(project, None, None, 0);
        let (in_list_id, unlisted_id) = (in_list.id, unlisted.id);

        let assembled = assemble(vec![inbox], vec![in_list, unlisted]);

        assert_eq!(assembled.task_lists[0].tasks.len(), 1);
        assert_eq!(assembled.task_lists[0].tasks[0].id, in_list_id);
        assert_eq!(assembled.tasks.len(), 1);
        assert_eq!(assembled.tasks[0].id, unlisted_id);
    }

    #[test]
    fn children_nest_to_any_depth_in_stored_order() {
        let project = ProjectId::new();
        let root = task(project, None, None, 0);
        let child = task(project, None, Some(root.id), 0);
        let grandchild_late = task(project, None, Some(child.id), 2);
        let grandchild_early = task(project, None, Some(child.id), 1);
        let (child_id, early, late) = (child.id, grandchild_early.id, grandchild_late.id);

        let assembled = assemble(
            Vec::new(),
            vec![grandchild_late, child, root, grandchild_early],
        );

        assert_eq!(assembled.tasks.len(), 1);
        let child = &assembled.tasks[0].sub_tasks[0];
        assert_eq!(child.id, child_id);
        let order: Vec<TaskId> = child.sub_tasks.iter().map(|task| task.id).collect();
        assert_eq!(order, vec![early, late]);
    }

    #[test]
    fn a_child_whose_parent_is_missing_becomes_a_root_of_the_project() {
        let project = ProjectId::new();
        let orphan = task(project, None, Some(TaskId::new()), 0);
        let orphan_id = orphan.id;

        let assembled = assemble(Vec::new(), vec![orphan]);

        assert_eq!(assembled.tasks.len(), 1);
        assert_eq!(assembled.tasks[0].id, orphan_id);
    }

    #[test]
    fn a_root_naming_an_unknown_list_stays_in_the_project() {
        let project = ProjectId::new();
        let lost = task(project, Some(TaskListId::new()), None, 0);

        let assembled = assemble(Vec::new(), vec![lost]);

        assert_eq!(assembled.tasks.len(), 1);
    }

    #[test]
    fn a_cycle_is_broken_at_its_lowest_id_and_nothing_is_lost() {
        let project = ProjectId::new();
        let mut a = task(project, None, None, 0);
        let mut b = task(project, None, None, 1);
        let mut c = task(project, None, None, 2);
        a.parent_task_id = Some(c.id);
        b.parent_task_id = Some(a.id);
        c.parent_task_id = Some(b.id);
        let lowest = [a.id, b.id, c.id].into_iter().min().unwrap();

        let assembled = assemble(Vec::new(), vec![a, b, c]);

        assert_eq!(assembled.tasks.len(), 1);
        let root = &assembled.tasks[0];
        assert_eq!(root.id, lowest);
        assert_eq!(root.walk().count(), 3);
    }

    #[test]
    fn a_task_naming_itself_as_parent_is_a_root() {
        let project = ProjectId::new();
        let mut selfish = task(project, None, None, 0);
        selfish.parent_task_id = Some(selfish.id);

        let assembled = assemble(Vec::new(), vec![selfish]);

        assert_eq!(assembled.tasks.len(), 1);
    }
}
