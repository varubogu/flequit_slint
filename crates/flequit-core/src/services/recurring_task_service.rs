//! 繰り返しタスクの次タスク管理
//!
//! 繰り返しルールを持つタスクが完了したとき、次の期限を持つ別タスク（次タスク）を
//! 生成する。タスクは生成元である「前タスクID」だけを持ち、次タスクIDは持たない。
//! 次タスクは「前タスクIDが自分であるタスク」を探して求める。
//!
//! - 完了: 次タスクがまだ無く、次回の期限があれば生成する。次タスクが既にあれば
//!   （前回の完了で生成済み、またはキャンセル時に変更済みで残したもの）生成しない
//! - キャンセル: 次タスクが生成時点から変更されていなければ削除する。
//!   変更されていれば残す
//!
//! 次回の期限の計算は表示タイムゾーンのカレンダーに依存するため、呼び出し側が
//! `next_due` として渡す。

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::date_condition::DateCondition;
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::task::Task;
use flequit_model::types::id_types::{
    DateConditionId, ProjectId, RecurrenceAdjustmentId, TagId, TaskId, TaskListId, UserId,
    WeekdayConditionId,
};
use flequit_model::types::task_types::TaskStatus;
use flequit_repository::repositories::project_relation_repository_trait::ProjectRelationRepository;
use flequit_repository::repositories::project_repository_trait::ProjectRepository;
use flequit_types::errors::service_error::ServiceError;
use tokio::sync::Mutex;

use crate::InfrastructureRepositoriesTrait;
use crate::services::recurrence_service;

/// 完了とキャンセルの後処理を直列化する。
///
/// 完了 → 未完了 → 完了のような連続操作で後処理が並走すると、どちらも
/// 「次タスクが無い」と判断して二重に生成してしまうため。
static SUCCESSOR_LOCK: Mutex<()> = Mutex::const_new(());

/// 後処理で次タスクに起きたこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessorChange {
    /// 何もしなかった（繰り返しでない、次回が無い、対象外のステータス）
    None,
    /// 次タスクを生成した
    Created(TaskId),
    /// 次タスクが既にあるため、そのまま残した
    Kept(TaskId),
    /// 変更されていない次タスクを削除した
    Removed(TaskId),
}

/// タスクの現在のステータスに合わせて次タスクを生成・削除する。
///
/// ステータスは引数で受け取らず保存済みの値を読む。後処理が遅れて走っても、
/// その時点の状態に対して正しく振る舞うため。
///
/// `next_due` は `(ルール, 起点, 起点が系列の何回目か)` から次回の期限を返す。
/// 起点はタスクの期限で、期限が無ければ現在時刻。
pub async fn sync_successor<R, F>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
    user_id: &UserId,
    next_due: F,
) -> Result<SuccessorChange, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
    F: FnOnce(&RecurrenceRule, &DateTime<Utc>, u32) -> Option<DateTime<Utc>> + Send,
{
    let _guard = SUCCESSOR_LOCK.lock().await;

    let tasks = repositories.tasks().find_all(project_id).await?;
    let Some(task) = tasks
        .iter()
        .find(|task| task.id == *task_id && !task.deleted)
    else {
        return Ok(SuccessorChange::None);
    };
    let successor = find_successor(&tasks, task_id);

    match task.status {
        TaskStatus::Completed => {
            if let Some(successor) = successor {
                return Ok(SuccessorChange::Kept(successor.id));
            }
            let Some(rule) = task_rule(repositories, project_id, task_id).await? else {
                return Ok(SuccessorChange::None);
            };

            let now = Utc::now();
            let anchor = task.plan_end_date.unwrap_or(now);
            let Some(due) = next_due(&rule, &anchor, occurrence_position(&tasks, task)) else {
                return Ok(SuccessorChange::None);
            };

            let next = build_successor(
                task,
                due,
                anchor,
                next_order_index(&tasks, &task.list_id),
                *user_id,
                now,
            );
            let next_rule = copy_rule(&rule, *user_id, now);
            let tag_ids = tag_ids_of(repositories, project_id, task_id).await?;

            repositories
                .tasks()
                .save(project_id, &next, user_id, &now)
                .await?;
            recurrence_service::create_recurrence_rule(
                repositories,
                project_id,
                next_rule.clone(),
                user_id,
            )
            .await?;
            recurrence_service::create_task_recurrence(
                repositories,
                project_id,
                &next.id,
                &next_rule.id,
            )
            .await?;
            for tag_id in &tag_ids {
                repositories
                    .task_tags()
                    .add(project_id, &next.id, tag_id, user_id, &now)
                    .await?;
            }

            Ok(SuccessorChange::Created(next.id))
        }
        TaskStatus::Cancelled => {
            let Some(successor) = successor else {
                return Ok(SuccessorChange::None);
            };

            let subtask_count = repositories
                .sub_tasks()
                .find_all(project_id)
                .await?
                .iter()
                .filter(|subtask| subtask.task_id == successor.id && !subtask.deleted)
                .count();
            let successor_tags = tag_ids_of(repositories, project_id, &successor.id).await?;
            let predecessor_tags = tag_ids_of(repositories, project_id, task_id).await?;
            let successor_rule = task_rule(repositories, project_id, &successor.id).await?;

            if !is_untouched(
                successor,
                subtask_count,
                &successor_tags,
                &predecessor_tags,
                successor_rule.as_ref(),
            ) {
                return Ok(SuccessorChange::Kept(successor.id));
            }

            let now = Utc::now();
            repositories
                .delete_task_transactionally(project_id, &successor.id, user_id, &now)
                .await?;
            // 次タスクのルールは生成時に複製した専用のものなので、一緒に消す。
            if let Some(rule) = successor_rule {
                recurrence_service::delete_recurrence_rule(
                    repositories,
                    project_id,
                    &rule.id.to_string(),
                )
                .await?;
            }

            Ok(SuccessorChange::Removed(successor.id))
        }
        _ => Ok(SuccessorChange::None),
    }
}

/// `task_id` を前タスクに持つ、削除されていないタスク。
pub fn find_successor<'a>(tasks: &'a [Task], task_id: &TaskId) -> Option<&'a Task> {
    tasks
        .iter()
        .find(|task| !task.deleted && task.previous_task_id == Some(*task_id))
}

/// `task` が系列の何回目か（最初のタスクが 1）。
///
/// 前タスクIDを辿った数に 1 を足す。既に消えた前タスクより先は辿れないため、
/// そこで数え終える。循環したデータでも止まるよう、訪問済みで打ち切る。
pub fn occurrence_position(tasks: &[Task], task: &Task) -> u32 {
    let mut visited = HashSet::from([task.id]);
    let mut position = 1;
    let mut previous = task.previous_task_id;

    while let Some(previous_id) = previous {
        if !visited.insert(previous_id) {
            break;
        }
        let Some(ancestor) = tasks.iter().find(|candidate| candidate.id == previous_id) else {
            break;
        };
        position += 1;
        previous = ancestor.previous_task_id;
    }

    position
}

/// 完了したタスクから次タスクを組み立てる。
///
/// 期限は `due` に置き換え、開始日時とリマインダーは期限と同じだけずらす。
/// 実績日時とステータスは引き継がない。タグと繰り返しルールは関連テーブルで
/// 管理するため、ここでは持たせない。
pub fn build_successor(
    current: &Task,
    due: DateTime<Utc>,
    anchor: DateTime<Utc>,
    order_index: i32,
    user_id: UserId,
    now: DateTime<Utc>,
) -> Task {
    let shift = due - anchor;

    Task {
        id: TaskId::new(),
        project_id: current.project_id,
        list_id: current.list_id,
        previous_task_id: Some(current.id),
        title: current.title.clone(),
        description: current.description.clone(),
        status: TaskStatus::NotStarted,
        priority: current.priority,
        plan_start_date: current.plan_start_date.map(|start| start + shift),
        plan_end_date: Some(due),
        do_start_date: None,
        do_end_date: None,
        is_range_date: current.is_range_date,
        recurrence_rule: None,
        reminders: current
            .reminders
            .iter()
            .map(|reminder| *reminder + shift)
            .collect(),
        order_index,
        is_archived: false,
        assigned_user_ids: current.assigned_user_ids.clone(),
        tag_ids: Vec::new(),
        // 作成日時と更新日時を揃えておき、キャンセル時の「未変更」判定に使う。
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    }
}

/// 次タスクが生成された時点から変更されていないか。
///
/// タスク本体の編集は `updated_at` に残るが、サブタスク・タグ・繰り返しルールは
/// 別テーブルのためタスクの `updated_at` を動かさない。それぞれを個別に確かめる。
pub fn is_untouched(
    successor: &Task,
    subtask_count: usize,
    successor_tags: &[TagId],
    predecessor_tags: &[TagId],
    successor_rule: Option<&RecurrenceRule>,
) -> bool {
    let same_tags = successor_tags.iter().collect::<HashSet<_>>()
        == predecessor_tags.iter().collect::<HashSet<_>>();

    successor.updated_at == successor.created_at
        && subtask_count == 0
        && same_tags
        && successor_rule.is_some_and(|rule| rule.updated_at == rule.created_at)
}

/// 次タスク専用に繰り返しルールを複製する。
///
/// ルールを共有すると、次タスクで繰り返しを解除したときに前タスクのルールまで
/// 消えてしまう。子要素も主キーを持つため、すべて新しい ID を振る。
pub fn copy_rule(rule: &RecurrenceRule, user_id: UserId, now: DateTime<Utc>) -> RecurrenceRule {
    let mut copy = rule.clone();
    copy.id = Default::default();
    copy.created_at = now;
    copy.updated_at = now;
    copy.deleted = false;
    copy.updated_by = user_id;

    if let Some(details) = copy.details.as_mut()
        && let Some(conditions) = details.date_conditions.as_mut()
    {
        conditions.iter_mut().for_each(renew_date_condition);
    }
    if let Some(adjustment) = copy.adjustment.as_mut() {
        adjustment.id = RecurrenceAdjustmentId::new();
        adjustment.recurrence_rule_id = copy.id;
        adjustment
            .date_conditions
            .iter_mut()
            .for_each(renew_date_condition);
        for condition in &mut adjustment.weekday_conditions {
            condition.id = WeekdayConditionId::new();
        }
    }

    copy
}

fn renew_date_condition(condition: &mut DateCondition) {
    condition.id = DateConditionId::new();
}

fn next_order_index(tasks: &[Task], list_id: &TaskListId) -> i32 {
    tasks
        .iter()
        .filter(|task| !task.deleted && task.list_id == *list_id)
        .map(|task| task.order_index)
        .max()
        .map_or(0, |order| order + 1)
}

async fn task_rule<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<Option<RecurrenceRule>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    let Some(relation) =
        recurrence_service::get_task_recurrence_by_task_id(repositories, project_id, task_id)
            .await?
    else {
        return Ok(None);
    };
    Ok(repositories
        .recurrence_rules()
        .find_by_id(project_id, &relation.recurrence_rule_id)
        .await?)
}

async fn tag_ids_of<R>(
    repositories: &R,
    project_id: &ProjectId,
    task_id: &TaskId,
) -> Result<Vec<TagId>, ServiceError>
where
    R: InfrastructureRepositoriesTrait + Send + Sync,
{
    Ok(repositories
        .task_tags()
        .find_relations(project_id, task_id)
        .await?
        .into_iter()
        .map(|relation| relation.tag_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, TimeZone};
    use flequit_model::models::task_projects::recurrence_adjustment::RecurrenceAdjustment;
    use flequit_model::models::task_projects::weekday_condition::WeekdayCondition;
    use flequit_model::types::datetime_calendar_types::{
        AdjustmentDirection, AdjustmentTarget, DayOfWeek, RecurrenceUnit,
    };

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour, 0, 0).unwrap()
    }

    fn task(previous: Option<TaskId>) -> Task {
        let created = at(1, 0);
        Task {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            list_id: TaskListId::new(),
            previous_task_id: previous,
            title: "Water the plants".to_string(),
            description: Some("both balconies".to_string()),
            status: TaskStatus::Completed,
            priority: 3,
            plan_start_date: Some(at(6, 8)),
            plan_end_date: Some(at(6, 9)),
            do_start_date: Some(at(6, 8)),
            do_end_date: Some(at(6, 9)),
            is_range_date: Some(true),
            recurrence_rule: None,
            reminders: vec![at(6, 7)],
            order_index: 4,
            is_archived: false,
            assigned_user_ids: Vec::new(),
            tag_ids: Vec::new(),
            created_at: created,
            updated_at: created,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    fn rule(now: DateTime<Utc>) -> RecurrenceRule {
        RecurrenceRule {
            id: Default::default(),
            unit: RecurrenceUnit::Day,
            interval: 1,
            days_of_week: None,
            details: None,
            adjustment: None,
            end_date: None,
            max_occurrences: None,
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    #[test]
    fn the_successor_is_found_through_its_previous_task_id() {
        let first = task(None);
        let second = task(Some(first.id));
        let tasks = vec![first.clone(), second.clone()];

        assert_eq!(
            find_successor(&tasks, &first.id).map(|t| t.id),
            Some(second.id)
        );
        assert!(find_successor(&tasks, &second.id).is_none());
    }

    #[test]
    fn a_deleted_successor_does_not_count() {
        let first = task(None);
        let mut second = task(Some(first.id));
        second.deleted = true;

        assert!(find_successor(&[first.clone(), second], &first.id).is_none());
    }

    #[test]
    fn the_position_counts_the_chain_of_previous_tasks() {
        let first = task(None);
        let second = task(Some(first.id));
        let third = task(Some(second.id));
        let tasks = vec![first.clone(), second.clone(), third.clone()];

        assert_eq!(occurrence_position(&tasks, &first), 1);
        assert_eq!(occurrence_position(&tasks, &third), 3);
    }

    #[test]
    fn the_position_stops_where_the_chain_is_gone_or_loops() {
        let orphan = task(Some(TaskId::new()));
        assert_eq!(
            occurrence_position(std::slice::from_ref(&orphan), &orphan),
            1
        );

        let mut a = task(None);
        let b = task(Some(a.id));
        a.previous_task_id = Some(b.id);
        assert_eq!(occurrence_position(&[a.clone(), b], &a), 2);
    }

    #[test]
    fn the_successor_moves_to_the_next_due_date_and_starts_over() {
        let current = task(None);
        let now = at(6, 10);
        let due = at(7, 9);

        let next = build_successor(&current, due, at(6, 9), 5, UserId::new(), now);

        assert_ne!(next.id, current.id);
        assert_eq!(next.previous_task_id, Some(current.id));
        assert_eq!(next.status, TaskStatus::NotStarted);
        assert_eq!(next.plan_end_date, Some(due));
        assert_eq!(next.plan_start_date, Some(at(7, 8)));
        assert_eq!(next.reminders, vec![at(7, 7)]);
        assert_eq!((next.do_start_date, next.do_end_date), (None, None));
        assert_eq!(next.title, current.title);
        assert_eq!(next.priority, current.priority);
        assert_eq!(next.order_index, 5);
        assert_eq!(next.created_at, next.updated_at);
    }

    #[test]
    fn a_fresh_successor_is_untouched() {
        let now = at(6, 10);
        let next = build_successor(&task(None), at(7, 9), at(6, 9), 0, UserId::new(), now);
        let tags = vec![TagId::new(), TagId::new()];
        let reordered = vec![tags[1], tags[0]];

        assert!(is_untouched(&next, 0, &tags, &reordered, Some(&rule(now))));
    }

    #[test]
    fn any_edit_to_the_successor_keeps_it() {
        let now = at(6, 10);
        let next = build_successor(&task(None), at(7, 9), at(6, 9), 0, UserId::new(), now);
        let tags = vec![TagId::new()];
        let fresh_rule = rule(now);

        let mut edited = next.clone();
        edited.updated_at = now + TimeDelta::seconds(1);
        assert!(!is_untouched(&edited, 0, &tags, &tags, Some(&fresh_rule)));

        assert!(!is_untouched(&next, 1, &tags, &tags, Some(&fresh_rule)));
        assert!(!is_untouched(&next, 0, &[], &tags, Some(&fresh_rule)));
        assert!(!is_untouched(&next, 0, &tags, &tags, None));

        let mut edited_rule = fresh_rule.clone();
        edited_rule.updated_at = now + TimeDelta::seconds(1);
        assert!(!is_untouched(&next, 0, &tags, &tags, Some(&edited_rule)));
    }

    #[test]
    fn a_copied_rule_shares_no_identity_with_the_original() {
        let original_time = at(1, 0);
        let mut original = rule(original_time);
        original.updated_at = at(2, 0);
        original.adjustment = Some(RecurrenceAdjustment {
            id: RecurrenceAdjustmentId::new(),
            recurrence_rule_id: original.id,
            date_conditions: Vec::new(),
            weekday_conditions: vec![WeekdayCondition {
                id: WeekdayConditionId::new(),
                if_weekday: DayOfWeek::Saturday,
                then_direction: AdjustmentDirection::Previous,
                then_target: AdjustmentTarget::Weekday,
                then_weekday: Some(DayOfWeek::Friday),
                then_days: None,
                created_at: original_time,
                updated_at: original_time,
                deleted: false,
                updated_by: UserId::new(),
            }],
            created_at: original_time,
            updated_at: original_time,
            deleted: false,
            updated_by: UserId::new(),
        });
        let now = at(6, 10);

        let copy = copy_rule(&original, UserId::new(), now);

        assert_ne!(copy.id, original.id);
        assert_eq!((copy.created_at, copy.updated_at), (now, now));
        assert_eq!(copy.interval, original.interval);
        let (copied, source) = (
            copy.adjustment.as_ref().unwrap(),
            original.adjustment.as_ref().unwrap(),
        );
        assert_ne!(copied.id, source.id);
        assert_eq!(copied.recurrence_rule_id, copy.id);
        assert_ne!(
            copied.weekday_conditions[0].id,
            source.weekday_conditions[0].id
        );
    }

    #[test]
    fn a_new_successor_goes_after_the_last_task_in_its_list() {
        let first = task(None);
        let mut other_list = task(None);
        other_list.order_index = 99;
        let mut deleted = task(None);
        deleted.list_id = first.list_id;
        deleted.order_index = 50;
        deleted.deleted = true;

        let tasks = vec![first.clone(), other_list, deleted];

        assert_eq!(next_order_index(&tasks, &first.list_id), 5);
        assert_eq!(next_order_index(&tasks, &TaskListId::new()), 0);
    }
}
