//! 旧形式のサブタスクをタスクへ移す
//!
//! 以前はサブタスクを独立したエンティティとして、プロジェクトドキュメントの
//! `subtasks` / `subtask_tags` / `subtask_assignments` / `subtask_recurrences` に置いていた。
//! 今のサブタスクは親を持つタスクで、`tasks` / `task_tags` / `task_assignments` に置く。
//!
//! 移すのはドキュメントへ書き込む直前（同期キューの適用とゴミ箱からの復元の前）。
//! 移す先にすでに同じキーがあれば書かないので、途中で止まってもやり直せる。
//! 旧サブタスクの繰り返しの関連は UI から書いていなかったため移さずに消す。

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use flequit_model::models::task_projects::task::Task;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, UserId};
use flequit_model::types::task_types::TaskStatus;
use flequit_types::errors::repository_error::RepositoryError;

use super::task::TASKS;
use super::task_assignments::{TASK_ASSIGNMENTS, TaskAssignmentRelation};
use super::task_tag::{TASK_TAGS, TaskTagRelation};
use crate::infrastructure::collection::{Collection, relation_key};
use crate::infrastructure::document::Document;

/// 旧サブタスク
///
/// 同期キューに残った旧形式の行（`sub_task.save`）の読み取りにも使う。
/// 欠けていてよい項目は既定値で読む。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySubTask {
    pub id: TaskId,
    /// 親タスク
    pub task_id: TaskId,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub plan_start_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub plan_end_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub do_start_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub do_end_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub is_range_date: Option<bool>,
    #[serde(default)]
    pub assigned_user_ids: Vec<UserId>,
    #[serde(default)]
    pub tag_ids: Vec<TagId>,
    #[serde(default)]
    pub order_index: i32,
    /// 状態より前からある完了フラグ。立っていれば状態にかかわらず完了
    #[serde(default)]
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub deleted: bool,
    pub updated_by: UserId,
}

impl LegacySubTask {
    /// 親を持つタスクにする。リストには属さず、繰り返しは持たない
    pub fn into_task(self, project_id: ProjectId) -> Task {
        Task {
            id: self.id,
            project_id,
            list_id: None,
            parent_task_id: Some(self.task_id),
            title: self.title,
            description: self.description,
            status: if self.completed {
                TaskStatus::Completed
            } else {
                self.status
            },
            priority: self.priority.unwrap_or(0),
            plan_start_date: self.plan_start_date,
            plan_end_date: self.plan_end_date,
            do_start_date: self.do_start_date,
            do_end_date: self.do_end_date,
            is_range_date: self.is_range_date,
            recurrence_rule: None,
            reminders: Vec::new(),
            order_index: self.order_index,
            is_archived: false,
            assigned_user_ids: self.assigned_user_ids,
            tag_ids: self.tag_ids,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted: self.deleted,
            updated_by: self.updated_by,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacySubTaskTag {
    subtask_id: String,
    tag_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    deleted: bool,
    updated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacySubTaskAssignment {
    subtask_id: String,
    user_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    deleted: bool,
    updated_by: String,
}

const SUBTASKS: Collection<LegacySubTask> =
    Collection::new("subtasks", |subtask| subtask.id.to_string());
const SUBTASK_TAGS: Collection<LegacySubTaskTag> = Collection::new("subtask_tags", |relation| {
    relation_key(&relation.subtask_id, &relation.tag_id)
});
const SUBTASK_ASSIGNMENTS: Collection<LegacySubTaskAssignment> =
    Collection::new("subtask_assignments", |relation| {
        relation_key(&relation.subtask_id, &relation.user_id)
    });

/// 旧形式の集合が置かれていたキー
const LEGACY_KEYS: [&str; 4] = [
    "subtasks",
    "subtask_tags",
    "subtask_assignments",
    "subtask_recurrences",
];

/// 移した結果
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacySubtaskMigration {
    /// タスクとして書いた旧サブタスクの数
    pub tasks: usize,
    /// 関連（タグ・担当者）として書いた数
    pub relations: usize,
}

/// `document` の旧サブタスクをタスクへ移し、旧形式のキーを消す
///
/// 旧形式のキーが無ければ何も書かない。
pub(crate) async fn migrate(
    document: &Document,
    project_id: &ProjectId,
) -> Result<LegacySubtaskMigration, RepositoryError> {
    let mut present = Vec::new();
    for key in LEGACY_KEYS {
        if document.load_data::<Json>(key).await?.is_some() {
            present.push(key);
        }
    }
    if present.is_empty() {
        return Ok(LegacySubtaskMigration::default());
    }

    let subtasks = document.load_collection(&SUBTASKS).await?;
    let tags = document.load_collection(&SUBTASK_TAGS).await?;
    let assignments = document.load_collection(&SUBTASK_ASSIGNMENTS).await?;

    // 移す先にすでにあるものは、後から書かれた新しい内容なので上書きしない
    let existing_tasks: HashSet<String> = document
        .load_collection(&TASKS)
        .await?
        .iter()
        .map(|task| TASKS.key_of(task))
        .collect();
    let tasks: Vec<Task> = subtasks
        .into_iter()
        .filter(|subtask| !existing_tasks.contains(&subtask.id.to_string()))
        .map(|subtask| subtask.into_task(*project_id))
        .collect();

    let existing_tags: HashSet<String> = document
        .load_collection(&TASK_TAGS)
        .await?
        .iter()
        .map(|relation| TASK_TAGS.key_of(relation))
        .collect();
    let task_tags: Vec<TaskTagRelation> = tags
        .into_iter()
        .map(|relation| TaskTagRelation {
            task_id: relation.subtask_id,
            tag_id: relation.tag_id,
            created_at: relation.created_at,
            updated_at: relation.updated_at,
            deleted: relation.deleted,
            updated_by: relation.updated_by,
        })
        .filter(|relation| !existing_tags.contains(&TASK_TAGS.key_of(relation)))
        .collect();

    let existing_assignments: HashSet<String> = document
        .load_collection(&TASK_ASSIGNMENTS)
        .await?
        .iter()
        .map(|relation| TASK_ASSIGNMENTS.key_of(relation))
        .collect();
    let task_assignments: Vec<TaskAssignmentRelation> = assignments
        .into_iter()
        .map(|relation| TaskAssignmentRelation {
            task_id: relation.subtask_id,
            user_id: relation.user_id,
            created_at: relation.created_at,
            updated_at: relation.updated_at,
            updated_by: relation.updated_by,
            deleted: relation.deleted,
        })
        .filter(|relation| !existing_assignments.contains(&TASK_ASSIGNMENTS.key_of(relation)))
        .collect();

    if !tasks.is_empty() {
        document.put_entries(&TASKS, &tasks).await?;
    }
    if !task_tags.is_empty() {
        document.put_entries(&TASK_TAGS, &task_tags).await?;
    }
    if !task_assignments.is_empty() {
        document
            .put_entries(&TASK_ASSIGNMENTS, &task_assignments)
            .await?;
    }
    // 書き終えてから消す。消す前に止まっても、次は書いた分を飛ばして消すだけになる
    for key in present {
        document.delete_data_at_path(&[key]).await?;
    }

    let migrated = LegacySubtaskMigration {
        tasks: tasks.len(),
        relations: task_tags.len() + task_assignments.len(),
    };
    tracing::info!(
        %project_id,
        tasks = migrated.tasks,
        relations = migrated.relations,
        "migrated legacy subtasks into tasks"
    );
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::document_manager::{DocumentManager, DocumentType};
    use serde_json::json;
    use tempfile::TempDir;

    const NOW: &str = "2026-09-30T00:00:00Z";

    async fn project_document(dir: &TempDir, project_id: ProjectId) -> Document {
        let mut manager = DocumentManager::new(dir.path()).unwrap();
        manager
            .get_or_create(&DocumentType::Project(project_id))
            .await
            .unwrap()
    }

    fn legacy_subtask(id: TaskId, parent: TaskId, completed: bool) -> Json {
        json!({
            "id": id,
            "task_id": parent,
            "title": "Legacy",
            "description": null,
            "status": "in_progress",
            "priority": null,
            "plan_start_date": null,
            "plan_end_date": NOW,
            "do_start_date": null,
            "do_end_date": null,
            "is_range_date": null,
            "recurrence_rule": null,
            "assigned_user_ids": [],
            "tag_ids": [],
            "order_index": 3,
            "completed": completed,
            "created_at": NOW,
            "updated_at": NOW,
            "deleted": false,
            "updated_by": UserId::new(),
        })
    }

    #[tokio::test]
    async fn legacy_subtasks_and_their_relations_become_child_tasks() {
        let dir = TempDir::new().unwrap();
        let project_id = ProjectId::new();
        let document = project_document(&dir, project_id).await;
        let (parent, open, done) = (TaskId::new(), TaskId::new(), TaskId::new());
        let (tag, user) = (TagId::new(), UserId::new());
        document
            .save_data_at_path(
                &["subtasks", &open.to_string()],
                &legacy_subtask(open, parent, false),
            )
            .await
            .unwrap();
        // 以前の形式（配列）でも読める
        document
            .save_data(
                "subtask_tags",
                &json!([{
                    "subtask_id": open, "tag_id": tag, "created_at": NOW, "updated_at": NOW,
                    "deleted": false, "updated_by": user,
                }]),
            )
            .await
            .unwrap();
        document
            .save_data_at_path(
                &["subtask_assignments", &format!("{open}:{user}")],
                &json!({
                    "subtask_id": open, "user_id": user, "created_at": NOW, "updated_at": NOW,
                    "updated_by": user, "deleted": false,
                }),
            )
            .await
            .unwrap();
        document
            .save_data_at_path(
                &["subtasks", &done.to_string()],
                &legacy_subtask(done, parent, true),
            )
            .await
            .unwrap();
        document
            .save_data("subtask_recurrences", &json!({}))
            .await
            .unwrap();

        let migrated = migrate(&document, &project_id).await.unwrap();

        assert_eq!(
            migrated,
            LegacySubtaskMigration {
                tasks: 2,
                relations: 2
            }
        );
        let tasks = document.load_collection(&TASKS).await.unwrap();
        let open_task = tasks.iter().find(|task| task.id == open).unwrap();
        assert_eq!(open_task.parent_task_id, Some(parent));
        assert_eq!(open_task.list_id, None);
        assert_eq!(open_task.project_id, project_id);
        assert_eq!(open_task.status, TaskStatus::InProgress);
        assert_eq!(open_task.priority, 0);
        assert_eq!(open_task.order_index, 3);
        let done_task = tasks.iter().find(|task| task.id == done).unwrap();
        assert_eq!(done_task.status, TaskStatus::Completed);
        assert_eq!(document.load_collection(&TASK_TAGS).await.unwrap().len(), 1);
        assert_eq!(
            document
                .load_collection(&TASK_ASSIGNMENTS)
                .await
                .unwrap()
                .len(),
            1
        );
        for key in LEGACY_KEYS {
            assert!(document.load_data::<Json>(key).await.unwrap().is_none());
        }

        // 2 回目は何もしない
        let again = migrate(&document, &project_id).await.unwrap();
        assert_eq!(again, LegacySubtaskMigration::default());
    }

    #[tokio::test]
    async fn a_task_written_after_the_legacy_one_is_not_overwritten() {
        let dir = TempDir::new().unwrap();
        let project_id = ProjectId::new();
        let document = project_document(&dir, project_id).await;
        let (parent, id) = (TaskId::new(), TaskId::new());
        document
            .save_data_at_path(
                &["subtasks", &id.to_string()],
                &legacy_subtask(id, parent, false),
            )
            .await
            .unwrap();
        let mut newer = serde_json::from_value::<LegacySubTask>(legacy_subtask(id, parent, false))
            .unwrap()
            .into_task(project_id);
        newer.title = "Renamed later".to_string();
        document.put_entry(&TASKS, &newer).await.unwrap();

        let migrated = migrate(&document, &project_id).await.unwrap();

        assert_eq!(migrated.tasks, 0);
        let kept = document
            .load_entry(&TASKS, &id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kept.title, "Renamed later");
        assert!(
            document
                .load_data::<Json>("subtasks")
                .await
                .unwrap()
                .is_none()
        );
    }
}
