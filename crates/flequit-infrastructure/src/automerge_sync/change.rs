//! Automerge へ反映する変更の表現
//!
//! 1 つの [`AutomergeChange`] がキューの 1 行になる。JSON にしてキューの `payload` 列へ
//! 入れ、ワーカーが読み戻して [`super::apply::AutomergeSyncTargets::apply`] で適用する。
//!
//! 形式を変えるときは、キューに残っている古い行を読めるように互換性を保つこと
//! （フィールドの追加は `#[serde(default)]` で、それ以外は新しい種類を足す）。
//!
//! サブタスクは親を持つタスクになった。`sub_task` 系の種類はもう作らないが、
//! アプリの更新前にキューへ入った行を読むために残し、タスクの操作として適用する。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use flequit_model::models::accounts::account::Account;
use flequit_model::models::task_projects::project::Project;
use flequit_model::models::task_projects::recurrence_rule::RecurrenceRule;
use flequit_model::models::task_projects::tag::Tag;
use flequit_model::models::task_projects::task::Task;
use flequit_model::models::task_projects::task_list::TaskList;
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::models::users::user::User;
use flequit_model::types::id_types::{
    AccountId, ProjectId, RecurrenceRuleId, TagId, TaskId, TaskListId, UserId,
};

pub use flequit_infrastructure_automerge::infrastructure::task_projects::legacy_subtasks::LegacySubTask;

/// Automerge へ反映する 1 件の変更
///
/// 各バリアントは、同期キュー導入前に統合リポジトリが Automerge リポジトリへ
/// 直接行っていた呼び出しと 1 対 1 に対応する。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "target", content = "change", rename_all = "snake_case")]
pub enum AutomergeChange {
    Account(RootChange<Account, AccountId>),
    User(RootChange<User, UserId>),
    Project(RootChange<Project, ProjectId>),
    TaskList(ProjectChange<TaskList, TaskListId>),
    /// 箱に入れても JSON は変わらない（他の種類との大きさの差を詰めるため）
    Task(Box<ProjectChange<Task, TaskId>>),
    /// 旧形式（読み取り専用）。サブタスクの ID はそのままタスクの ID になる
    SubTask(ProjectChange<LegacySubTask, TaskId>),
    Tag(ProjectChange<Tag, TagId>),
    RecurrenceRule(ProjectChange<RecurrenceRule, RecurrenceRuleId>),
    TaskTag(RelationChange<TaskId, TagId>),
    /// 旧形式（読み取り専用）。タスクのタグとして適用する
    SubTaskTag(RelationChange<TaskId, TagId>),
    TaskAssignment(RelationChange<TaskId, UserId>),
    /// 旧形式（読み取り専用）。タスクの担当者として適用する
    SubTaskAssignment(RelationChange<TaskId, UserId>),
    TaskRecurrence(RelationChange<TaskId, RecurrenceRuleId>),
    /// 旧形式（読み取り専用）。サブタスクは繰り返しを持たないので何もしない
    SubTaskRecurrence(RelationChange<TaskId, RecurrenceRuleId>),
    TagBookmark(TagBookmarkChange),
    /// 論理削除（ゴミ箱へ移す）と復元
    Trash(TrashChange),
}

/// プロジェクトに属さないエンティティ（アカウント・ユーザー・プロジェクト本体）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RootChange<T, Id> {
    Save {
        entity: T,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    Delete {
        id: Id,
    },
}

/// プロジェクト内のエンティティ
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ProjectChange<T, Id> {
    Save {
        project_id: ProjectId,
        entity: T,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    Delete {
        project_id: ProjectId,
        id: Id,
    },
}

/// プロジェクト内の関連（親 → 子）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RelationChange<P, C> {
    Add {
        project_id: ProjectId,
        parent_id: P,
        child_id: C,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    Remove {
        project_id: ProjectId,
        parent_id: P,
        child_id: C,
    },
    /// 親に付いた関連をすべて外す
    RemoveAll { project_id: ProjectId, parent_id: P },
    /// 子を指す関連をすべて外す（タグの削除時）
    RemoveAllByChild { project_id: ProjectId, child_id: C },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TagBookmarkChange {
    Create {
        bookmark: TagBookmark,
    },
    Update {
        bookmark: TagBookmark,
    },
    Delete {
        user_id: UserId,
        project_id: ProjectId,
        tag_id: TagId,
    },
}

/// 論理削除と復元。Automerge では履歴のためにエンティティを消さず `deleted` にする
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TrashChange {
    /// プロジェクトと、その中のタスク・タグ・タスクリストをすべて削除済みにする
    DeleteProject {
        project_id: ProjectId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    DeleteTaskList {
        project_id: ProjectId,
        task_list_id: TaskListId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    DeleteTask {
        project_id: ProjectId,
        task_id: TaskId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    DeleteTag {
        project_id: ProjectId,
        tag_id: TagId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    /// プロジェクトと、その中の削除済みのタスク・タグ・タスクリストをすべて戻す
    RestoreProject {
        project_id: ProjectId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    RestoreTaskList {
        project_id: ProjectId,
        task_list_id: TaskListId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    RestoreTask {
        project_id: ProjectId,
        task_id: TaskId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
    RestoreTag {
        project_id: ProjectId,
        tag_id: TagId,
        user_id: UserId,
        timestamp: DateTime<Utc>,
    },
}

/// 反映先の Automerge ドキュメントを表すキー
///
/// 同じキーの変更は必ずキューに入った順に適用する。
/// ファイル名（`DocumentType::filename`）と対応させている。
pub fn project_document_key(project_id: &ProjectId) -> String {
    format!("project:{project_id}")
}

pub const ACCOUNT_DOCUMENT_KEY: &str = "account";
pub const USER_DOCUMENT_KEY: &str = "user";

impl AutomergeChange {
    /// 反映先ドキュメントのキー
    pub fn document_key(&self) -> String {
        match self {
            Self::Account(_) => ACCOUNT_DOCUMENT_KEY.to_string(),
            // タグのブックマークはユーザードキュメントに入る
            Self::User(_) | Self::TagBookmark(_) => USER_DOCUMENT_KEY.to_string(),
            Self::Project(RootChange::Save { entity, .. }) => project_document_key(&entity.id),
            Self::Project(RootChange::Delete { id }) => project_document_key(id),
            Self::TaskList(change) => project_document_key(change.project_id()),
            Self::Task(change) => project_document_key(change.project_id()),
            Self::SubTask(change) => project_document_key(change.project_id()),
            Self::Tag(change) => project_document_key(change.project_id()),
            Self::RecurrenceRule(change) => project_document_key(change.project_id()),
            Self::TaskTag(change) => project_document_key(change.project_id()),
            Self::SubTaskTag(change) => project_document_key(change.project_id()),
            Self::TaskAssignment(change) => project_document_key(change.project_id()),
            Self::SubTaskAssignment(change) => project_document_key(change.project_id()),
            Self::TaskRecurrence(change) => project_document_key(change.project_id()),
            Self::SubTaskRecurrence(change) => project_document_key(change.project_id()),
            Self::Trash(change) => project_document_key(change.project_id()),
        }
    }

    /// 反映先がプロジェクトのドキュメントなら、そのプロジェクト
    pub fn project_id(&self) -> Option<ProjectId> {
        match self {
            Self::Account(_) | Self::User(_) | Self::TagBookmark(_) => None,
            Self::Project(RootChange::Save { entity, .. }) => Some(entity.id),
            Self::Project(RootChange::Delete { id }) => Some(*id),
            Self::TaskList(change) => Some(*change.project_id()),
            Self::Task(change) => Some(*change.project_id()),
            Self::SubTask(change) => Some(*change.project_id()),
            Self::Tag(change) => Some(*change.project_id()),
            Self::RecurrenceRule(change) => Some(*change.project_id()),
            Self::TaskTag(change) | Self::SubTaskTag(change) => Some(*change.project_id()),
            Self::TaskAssignment(change) | Self::SubTaskAssignment(change) => {
                Some(*change.project_id())
            }
            Self::TaskRecurrence(change) | Self::SubTaskRecurrence(change) => {
                Some(*change.project_id())
            }
            Self::Trash(change) => Some(*change.project_id()),
        }
    }

    /// ログと調査用の種類名（例: `task.save`）
    pub fn kind(&self) -> String {
        let (target, op) = match self {
            Self::Account(change) => ("account", change.op()),
            Self::User(change) => ("user", change.op()),
            Self::Project(change) => ("project", change.op()),
            Self::TaskList(change) => ("task_list", change.op()),
            Self::Task(change) => ("task", change.op()),
            Self::SubTask(change) => ("sub_task", change.op()),
            Self::Tag(change) => ("tag", change.op()),
            Self::RecurrenceRule(change) => ("recurrence_rule", change.op()),
            Self::TaskTag(change) => ("task_tag", change.op()),
            Self::SubTaskTag(change) => ("sub_task_tag", change.op()),
            Self::TaskAssignment(change) => ("task_assignment", change.op()),
            Self::SubTaskAssignment(change) => ("sub_task_assignment", change.op()),
            Self::TaskRecurrence(change) => ("task_recurrence", change.op()),
            Self::SubTaskRecurrence(change) => ("sub_task_recurrence", change.op()),
            Self::TagBookmark(change) => ("tag_bookmark", change.op()),
            Self::Trash(change) => ("trash", change.op()),
        };
        format!("{target}.{op}")
    }
}

impl<T, Id> RootChange<T, Id> {
    fn op(&self) -> &'static str {
        match self {
            Self::Save { .. } => "save",
            Self::Delete { .. } => "delete",
        }
    }
}

impl<T, Id> ProjectChange<T, Id> {
    pub fn project_id(&self) -> &ProjectId {
        match self {
            Self::Save { project_id, .. } | Self::Delete { project_id, .. } => project_id,
        }
    }

    fn op(&self) -> &'static str {
        match self {
            Self::Save { .. } => "save",
            Self::Delete { .. } => "delete",
        }
    }
}

impl<P, C> RelationChange<P, C> {
    pub fn project_id(&self) -> &ProjectId {
        match self {
            Self::Add { project_id, .. }
            | Self::Remove { project_id, .. }
            | Self::RemoveAll { project_id, .. }
            | Self::RemoveAllByChild { project_id, .. } => project_id,
        }
    }

    fn op(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Remove { .. } => "remove",
            Self::RemoveAll { .. } => "remove_all",
            Self::RemoveAllByChild { .. } => "remove_all_by_child",
        }
    }
}

impl TagBookmarkChange {
    fn op(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Update { .. } => "update",
            Self::Delete { .. } => "delete",
        }
    }
}

impl TrashChange {
    pub fn project_id(&self) -> &ProjectId {
        match self {
            Self::DeleteProject { project_id, .. }
            | Self::DeleteTaskList { project_id, .. }
            | Self::DeleteTask { project_id, .. }
            | Self::DeleteTag { project_id, .. }
            | Self::RestoreProject { project_id, .. }
            | Self::RestoreTaskList { project_id, .. }
            | Self::RestoreTask { project_id, .. }
            | Self::RestoreTag { project_id, .. } => project_id,
        }
    }

    fn op(&self) -> &'static str {
        match self {
            Self::DeleteProject { .. } => "delete_project",
            Self::DeleteTaskList { .. } => "delete_task_list",
            Self::DeleteTask { .. } => "delete_task",
            Self::DeleteTag { .. } => "delete_tag",
            Self::RestoreProject { .. } => "restore_project",
            Self::RestoreTaskList { .. } => "restore_task_list",
            Self::RestoreTask { .. } => "restore_task",
            Self::RestoreTag { .. } => "restore_tag",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_survives_the_json_round_trip_through_the_queue() {
        let project_id = ProjectId::new();
        let change = AutomergeChange::TaskTag(RelationChange::Add {
            project_id,
            parent_id: TaskId::new(),
            child_id: TagId::new(),
            user_id: UserId::new(),
            timestamp: Utc::now(),
        });

        let json = serde_json::to_string(&change).expect("serialize");
        let restored: AutomergeChange = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.kind(), "task_tag.add");
        assert_eq!(restored.document_key(), format!("project:{project_id}"));
    }

    /// 更新前のアプリがキューへ入れたサブタスクの行も読める
    #[test]
    fn a_legacy_subtask_row_is_still_read() {
        let project_id = ProjectId::new();
        let (subtask_id, task_id, user_id) = (TaskId::new(), TaskId::new(), UserId::new());
        let payload = serde_json::json!({
            "target": "sub_task",
            "change": {
                "op": "save",
                "project_id": project_id,
                "entity": {
                    "id": subtask_id,
                    "task_id": task_id,
                    "title": "Legacy",
                    "description": null,
                    "status": "completed",
                    "priority": 2,
                    "plan_start_date": null,
                    "plan_end_date": null,
                    "do_start_date": null,
                    "do_end_date": null,
                    "is_range_date": null,
                    "recurrence_rule": null,
                    "assigned_user_ids": [],
                    "tag_ids": [],
                    "order_index": 1,
                    "completed": false,
                    "created_at": "2026-09-30T00:00:00Z",
                    "updated_at": "2026-09-30T00:00:00Z",
                    "deleted": false,
                    "updated_by": user_id,
                },
                "user_id": user_id,
                "timestamp": "2026-09-30T00:00:00Z",
            }
        });

        let change: AutomergeChange = serde_json::from_value(payload).expect("deserialize");

        assert_eq!(change.kind(), "sub_task.save");
        assert_eq!(change.document_key(), project_document_key(&project_id));
        let AutomergeChange::SubTask(ProjectChange::Save { entity, .. }) = change else {
            panic!("expected a legacy subtask save");
        };
        let task = entity.into_task(project_id);
        assert_eq!(task.id, subtask_id);
        assert_eq!(task.parent_task_id, Some(task_id));
        assert_eq!(task.list_id, None);
    }

    #[test]
    fn bookmarks_share_the_user_document_with_the_user() {
        let change = AutomergeChange::TagBookmark(TagBookmarkChange::Delete {
            user_id: UserId::new(),
            project_id: ProjectId::new(),
            tag_id: TagId::new(),
        });
        assert_eq!(change.document_key(), USER_DOCUMENT_KEY);
        assert_eq!(change.kind(), "tag_bookmark.delete");
    }

    #[test]
    fn a_project_is_keyed_by_its_own_document() {
        let id = ProjectId::new();
        let change: AutomergeChange = AutomergeChange::Project(RootChange::Delete { id });
        assert_eq!(change.document_key(), project_document_key(&id));
        assert_eq!(change.kind(), "project.delete");
    }
}
