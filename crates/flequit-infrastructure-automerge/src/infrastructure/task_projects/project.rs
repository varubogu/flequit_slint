use crate::infrastructure::collection::Collection;
use crate::infrastructure::document::Document;

use super::super::document_manager::{DocumentManager, DocumentType};
use super::member::MEMBERS;
use super::subtask::SUBTASKS;
use super::tag::TAGS;
use super::task::TASKS;
use super::task_list::TASK_LISTS;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flequit_model::models::task_projects::member::Member;
use flequit_model::models::task_projects::{
    project::Project, subtask::SubTask, tag::Tag, task::Task, task_list::TaskList,
};
use flequit_model::traits::Trackable;
use flequit_model::types::id_types::{ProjectId, TagId, TaskId, TaskListId, UserId};
use flequit_model::types::project_types::ProjectStatus;
use flequit_repository::base_repository_trait::Repository;
use flequit_repository::repositories::task_projects::project_repository_trait::ProjectRepositoryTrait;
use flequit_types::errors::repository_error::RepositoryError;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Project Document構造（data-structure.md仕様準拠）
///
/// models/project::Projectの全プロパティ + プロジェクト内のすべてのエンティティを含む
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDocument {
    // models/project::Projectのプロパティ
    /// プロジェクトの一意識別子
    pub id: String,
    /// プロジェクト名（必須）
    pub name: String,
    /// プロジェクトの説明文
    pub description: Option<String>,
    /// UI表示用のカラーコード（Svelteフロントエンド対応）
    pub color: Option<String>,
    /// 表示順序（昇順ソート用）
    pub order_index: i32,
    /// アーカイブ状態フラグ
    pub is_archived: bool,
    /// プロジェクトステータス（進行中、完了等）
    pub status: Option<ProjectStatus>,
    /// プロジェクトオーナーのユーザーID
    pub owner_id: Option<UserId>,
    /// プロジェクト作成日時
    pub created_at: DateTime<Utc>,
    /// 最終更新日時
    pub updated_at: DateTime<Utc>,
    /// 最終更新者のユーザーID
    pub updated_by: UserId,
    /// 論理削除フラグ
    pub deleted: bool,

    // プロジェクト内のエンティティ
    /// タスクリスト配列
    pub task_lists: Vec<TaskList>,
    /// タスク配列
    pub tasks: Vec<Task>,
    /// サブタスク配列
    pub subtasks: Vec<SubTask>,
    /// タグ配列
    pub tags: Vec<Tag>,
    /// メンバー配列
    pub members: Vec<Member>,
}

/// Automerge実装のプロジェクトリポジトリ
///
/// `Repository<Project>`と`ProjectRepositoryTrait`を実装し、
/// Automerge-Repoを使用したプロジェクト管理を提供する。
///
/// # アーキテクチャ
///
/// ```text
/// LocalAutomergeProjectRepository (このクラス)
///   | 委譲
/// DocumentManager (プロジェクトごとのドキュメント管理)
///   | データアクセス
/// Automerge Documents
/// ```
///
/// # 特徴
///
/// - **分散同期**: CRDTによる競合解決機能
/// - **履歴管理**: すべての変更履歴を保持
/// - **オフライン対応**: ローカル優先で同期可能
/// - **JSON互換**: 構造化データの効率的な管理
/// - **ステートレス**: プロジェクトIDは必要時に引数で受け取る
#[derive(Debug)]
pub struct ProjectLocalAutomergeRepository {
    document_manager: Arc<Mutex<DocumentManager>>,
}

impl ProjectLocalAutomergeRepository {
    /// 新しいProjectRepositoryを作成
    pub async fn new(base_path: PathBuf) -> Result<Self, RepositoryError> {
        let document_manager = DocumentManager::new(base_path)?;
        Ok(Self {
            document_manager: Arc::new(Mutex::new(document_manager)),
        })
    }

    /// 共有DocumentManagerを使用して新しいインスタンスを作成
    pub async fn new_with_manager(
        document_manager: Arc<Mutex<DocumentManager>>,
    ) -> Result<Self, RepositoryError> {
        Ok(Self { document_manager })
    }

    /// 指定されたプロジェクトのDocumentを取得または作成
    async fn get_or_create_document(
        &self,
        project_id: &ProjectId,
    ) -> Result<Document, RepositoryError> {
        let doc_type = DocumentType::Project(*project_id);
        let mut manager = self.document_manager.lock().await;
        manager
            .get_or_create(&doc_type)
            .await
            .map_err(|e| RepositoryError::AutomergeError(e.to_string()))
    }

    /// プロジェクトドキュメント全体を取得
    pub async fn get_project_document(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<ProjectDocument>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        let Some(project) = load_project(&document).await? else {
            return Ok(None);
        };

        Ok(Some(ProjectDocument {
            id: project.id.to_string(),
            name: project.name,
            description: project.description,
            color: project.color,
            order_index: project.order_index,
            is_archived: project.is_archived,
            status: project.status,
            owner_id: project.owner_id,
            created_at: project.created_at,
            updated_at: project.updated_at,
            updated_by: project.updated_by,
            deleted: project.deleted,
            task_lists: document.load_collection(&TASK_LISTS).await?,
            tasks: document.load_collection(&TASKS).await?,
            subtasks: document.load_collection(&SUBTASKS).await?,
            tags: document.load_collection(&TAGS).await?,
            members: document.load_collection(&MEMBERS).await?,
        }))
    }

    /// プロジェクトドキュメント全体を `project_document` の内容にする
    ///
    /// 変わった値だけを書き、`project_document` に無いエンティティは消す。スナップショットからの
    /// 復元用。個々のエンティティの変更には、そのエンティティだけを書くメソッドを使うこと。
    pub async fn save_project_document(
        &self,
        project_id: &ProjectId,
        project_document: &ProjectDocument,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;

        let project = Project {
            id: ProjectId::from(project_document.id.clone()),
            name: project_document.name.clone(),
            description: project_document.description.clone(),
            color: project_document.color.clone(),
            order_index: project_document.order_index,
            is_archived: project_document.is_archived,
            status: project_document.status.clone(),
            owner_id: project_document.owner_id,
            created_at: project_document.created_at,
            updated_at: project_document.updated_at,
            updated_by: project_document.updated_by,
            deleted: project_document.deleted,
        };
        save_project(&document, &project).await?;

        document
            .replace_collection(&TASK_LISTS, &project_document.task_lists)
            .await?;
        document
            .replace_collection(&TASKS, &project_document.tasks)
            .await?;
        document
            .replace_collection(&SUBTASKS, &project_document.subtasks)
            .await?;
        document
            .replace_collection(&TAGS, &project_document.tags)
            .await?;
        document
            .replace_collection(&MEMBERS, &project_document.members)
            .await?;

        Ok(())
    }

    /// 空のプロジェクトドキュメントを作成
    ///
    /// 基本情報だけを書く。エンティティの集合は最初のエンティティを保存したときに作られる。
    pub async fn create_empty_project_document(
        &self,
        project: &Project,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(&project.id).await?;
        save_project(&document, project).await
    }

    /// IDでプロジェクトを取得（プロジェクトドキュメントから基本情報のみ）
    pub async fn get_project(&self, project_id: &str) -> Result<Option<Project>, RepositoryError> {
        let document = self
            .get_or_create_document(&ProjectId::from(project_id))
            .await?;
        load_project(&document).await
    }

    /// プロジェクトを作成または更新（基本情報のみ）
    ///
    /// ドキュメント内のエンティティには触れない。
    pub async fn set_project(&self, project: &Project) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(&project.id).await?;
        let project = match load_project(&document).await? {
            // 作成時の値（ID・作成日時）は残す
            Some(existing) => Project {
                id: existing.id,
                created_at: existing.created_at,
                ..project.clone()
            },
            None => project.clone(),
        };
        save_project(&document, &project).await
    }

    /// 基本情報のあるプロジェクトドキュメントを返す。無ければ `NotFound`
    async fn existing_document(&self, project_id: &ProjectId) -> Result<Document, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        if load_project(&document).await?.is_none() {
            return Err(RepositoryError::NotFound(format!(
                "Project not found: {}",
                project_id
            )));
        }
        Ok(document)
    }

    /// エンティティを追加し、プロジェクトの更新日時を進める
    async fn add_entity<T: Serialize + DeserializeOwned>(
        &self,
        project_id: &ProjectId,
        collection: &Collection<T>,
        entity: &T,
    ) -> Result<(), RepositoryError> {
        let document = self.existing_document(project_id).await?;
        document.put_entry(collection, entity).await?;
        document.save_data("updated_at", &Utc::now()).await?;
        Ok(())
    }

    /// 集合の全エンティティに `update` を適用し、変えたもの（`update` が真を返したもの）だけを書く
    async fn update_all<T: Serialize + DeserializeOwned>(
        &self,
        project_id: &ProjectId,
        collection: &Collection<T>,
        mut update: impl FnMut(&mut T) -> bool,
    ) -> Result<(), RepositoryError> {
        let document = self.existing_document(project_id).await?;
        let mut entities = document.load_collection(collection).await?;
        entities.retain_mut(|entity| update(entity));
        document.put_entries(collection, &entities).await?;
        Ok(())
    }

    /// `key` のエンティティに `update` を適用して書く。エンティティが無ければ `not_found()`
    async fn update_one<T: Serialize + DeserializeOwned>(
        &self,
        project_id: &ProjectId,
        collection: &Collection<T>,
        key: &str,
        not_found: impl FnOnce() -> RepositoryError,
        update: impl FnOnce(&mut T) -> Result<(), RepositoryError>,
    ) -> Result<(), RepositoryError> {
        let document = self.existing_document(project_id).await?;
        let mut entity = document
            .load_entry(collection, key)
            .await?
            .ok_or_else(not_found)?;
        update(&mut entity)?;
        document.put_entry(collection, &entity).await?;
        Ok(())
    }

    /// 削除済みの `key` のエンティティを返す。プロジェクトやエンティティが無い、または削除済みでなければ `None`
    async fn find_deleted<T: DeserializeOwned + Trackable>(
        &self,
        project_id: &ProjectId,
        collection: &Collection<T>,
        key: &str,
    ) -> Result<Option<T>, RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        if load_project(&document).await?.is_none() {
            return Ok(None);
        }
        let entity: Option<T> = document.load_entry(collection, key).await?;
        Ok(entity.filter(|entity| entity.is_deleted()))
    }

    /// タスクリストを追加
    pub async fn add_task_list(
        &self,
        project_id: &ProjectId,
        task_list: &TaskList,
    ) -> Result<(), RepositoryError> {
        self.add_entity(project_id, &TASK_LISTS, task_list).await
    }

    /// タスクを追加
    pub async fn add_task(
        &self,
        project_id: &ProjectId,
        task: &Task,
    ) -> Result<(), RepositoryError> {
        self.add_entity(project_id, &TASKS, task).await
    }

    /// サブタスクを追加
    pub async fn add_subtask(
        &self,
        project_id: &ProjectId,
        subtask: &SubTask,
    ) -> Result<(), RepositoryError> {
        self.add_entity(project_id, &SUBTASKS, subtask).await
    }

    /// タグを追加
    pub async fn add_tag(&self, project_id: &ProjectId, tag: &Tag) -> Result<(), RepositoryError> {
        self.add_entity(project_id, &TAGS, tag).await
    }

    /// メンバーを追加
    pub async fn add_member(
        &self,
        project_id: &ProjectId,
        member: &Member,
    ) -> Result<(), RepositoryError> {
        self.add_entity(project_id, &MEMBERS, member).await
    }

    /// プロジェクト内の全タスクを取得
    pub async fn get_tasks(&self, project_id: &ProjectId) -> Result<Vec<Task>, RepositoryError> {
        if let Some(document) = self.get_project_document(project_id).await? {
            Ok(document.tasks)
        } else {
            Ok(Vec::new())
        }
    }

    /// プロジェクト内の全タスクリストを取得
    pub async fn get_task_lists(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskList>, RepositoryError> {
        if let Some(document) = self.get_project_document(project_id).await? {
            Ok(document.task_lists)
        } else {
            Ok(Vec::new())
        }
    }

    /// プロジェクト内の全サブタスクを取得
    pub async fn get_subtasks(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<SubTask>, RepositoryError> {
        if let Some(document) = self.get_project_document(project_id).await? {
            Ok(document.subtasks)
        } else {
            Ok(Vec::new())
        }
    }

    /// プロジェクト内の全タグを取得
    pub async fn get_tags(&self, project_id: &ProjectId) -> Result<Vec<Tag>, RepositoryError> {
        if let Some(document) = self.get_project_document(project_id).await? {
            Ok(document.tags)
        } else {
            Ok(Vec::new())
        }
    }

    /// プロジェクト内の全メンバーを取得
    pub async fn get_members(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<Member>, RepositoryError> {
        if let Some(document) = self.get_project_document(project_id).await? {
            Ok(document.members)
        } else {
            Ok(Vec::new())
        }
    }

    /// プロジェクトドキュメント自体を削除
    pub async fn delete_project_document(
        &self,
        project_id: &ProjectId,
    ) -> Result<(), RepositoryError> {
        let mut manager = self.document_manager.lock().await;
        let doc_type = DocumentType::Project(*project_id);
        manager
            .delete(doc_type)
            .map_err(|e| RepositoryError::AutomergeError(e.to_string()))
    }

    /// JSON出力機能：プロジェクト変更履歴をエクスポート
    pub async fn export_project_changes_history<P: AsRef<Path>>(
        &self,
        project_id: &ProjectId,
        output_dir: P,
        description: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        document
            .export_document_changes_history(&output_dir, description)
            .await
            .map_err(|e| RepositoryError::Export(e.to_string()))
    }

    /// JSON出力機能：現在のプロジェクト状態をファイルにエクスポート
    pub async fn export_project_state<P: AsRef<Path>>(
        &self,
        project_id: &ProjectId,
        file_path: P,
        description: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let document = self.get_or_create_document(project_id).await?;
        document
            .export_json(&file_path, description)
            .await
            .map_err(|e| RepositoryError::Export(e.to_string()))
    }

    // ========== 論理削除メソッド（Phase 1） ==========

    /// プロジェクトを論理削除（deleted=trueに設定）
    ///
    /// # 引数
    /// * `project_id` - 削除するプロジェクトのID
    /// * `user_id` - 削除操作を実行するユーザーのID
    /// * `timestamp` - 削除操作の日時
    ///
    /// # 戻り値
    /// 成功時は`Ok(())`、プロジェクトが見つからない場合は`Err(RepositoryError::NotFound)`
    pub async fn mark_project_deleted(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        if let Some(mut project) = self.get_project(&project_id.to_string()).await? {
            // Trackableトレイトを使用して論理削除
            project.mark_deleted(*user_id, *timestamp);
            self.set_project(&project).await
        } else {
            Err(RepositoryError::NotFound(format!(
                "Project not found: {}",
                project_id
            )))
        }
    }

    /// プロジェクト内のすべてのタスクを論理削除
    pub async fn mark_all_tasks_deleted(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_all(project_id, &TASKS, |task| {
            task.mark_deleted(*user_id, *timestamp);
            true
        })
        .await
    }

    /// プロジェクト内のすべてのタグを論理削除
    pub async fn mark_all_tags_deleted(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_all(project_id, &TAGS, |tag| {
            tag.mark_deleted(*user_id, *timestamp);
            true
        })
        .await
    }

    /// プロジェクト内のすべてのタスクリストを論理削除
    pub async fn mark_all_task_lists_deleted(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_all(project_id, &TASK_LISTS, |task_list| {
            task_list.mark_deleted(*user_id, *timestamp);
            true
        })
        .await
    }

    // ========== スナップショット機能（Phase 2） ==========

    /// スナップショットを作成（更新前の状態を保存）
    ///
    /// トランザクション中のロールバック用に、プロジェクトドキュメント全体をメモリに保持します。
    ///
    /// # 引数
    /// * `project_id` - スナップショットを作成するプロジェクトのID
    ///
    /// # 戻り値
    /// プロジェクトドキュメントのクローン。プロジェクトが存在しない場合はエラー。
    pub async fn create_snapshot(
        &self,
        project_id: &ProjectId,
    ) -> Result<ProjectDocument, RepositoryError> {
        let document = self
            .get_project_document(project_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("Project not found: {}", project_id))
            })?;
        Ok(document.clone())
    }

    /// スナップショットから復元
    ///
    /// トランザクション失敗時に、保存されたスナップショットからドキュメントを復元します。
    ///
    /// # 引数
    /// * `project_id` - 復元するプロジェクトのID
    /// * `snapshot` - 復元元のプロジェクトドキュメント
    pub async fn restore_from_snapshot(
        &self,
        project_id: &ProjectId,
        snapshot: &ProjectDocument,
    ) -> Result<(), RepositoryError> {
        self.save_project_document(project_id, snapshot).await
    }

    // ========== クエリフィルタ（Phase 3） ==========

    /// 削除済みプロジェクトを取得
    ///
    /// # 引数
    /// * `project_id` - プロジェクトのID
    ///
    /// # 戻り値
    /// 削除済み（deleted=true）のプロジェクト、またはNone
    pub async fn get_deleted_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Project>, RepositoryError> {
        if let Some(project) = self.get_project(&project_id.to_string()).await? {
            if project.is_deleted() {
                Ok(Some(project))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// アクティブなタスクのみ取得（deleted=falseのみ）
    pub async fn get_active_tasks(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<Task>, RepositoryError> {
        let document = self
            .get_project_document(project_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("Project not found: {}", project_id))
            })?;
        Ok(document
            .tasks
            .into_iter()
            .filter(|t| !t.is_deleted())
            .collect())
    }

    /// 削除済みタスクのみ取得（deleted=trueのみ）
    pub async fn get_deleted_tasks(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<Task>, RepositoryError> {
        let document = self
            .get_project_document(project_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("Project not found: {}", project_id))
            })?;
        Ok(document
            .tasks
            .into_iter()
            .filter(|t| t.is_deleted())
            .collect())
    }

    /// アクティブなタグのみ取得（deleted=falseのみ）
    pub async fn get_active_tags(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<Tag>, RepositoryError> {
        let document = self
            .get_project_document(project_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("Project not found: {}", project_id))
            })?;
        Ok(document
            .tags
            .into_iter()
            .filter(|t| !t.is_deleted())
            .collect())
    }

    /// 削除済みタグのみ取得（deleted=trueのみ）
    pub async fn get_deleted_tags(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<Tag>, RepositoryError> {
        let document = self
            .get_project_document(project_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("Project not found: {}", project_id))
            })?;
        Ok(document
            .tags
            .into_iter()
            .filter(|t| t.is_deleted())
            .collect())
    }

    /// アクティブなタスクリストのみ取得（deleted=falseのみ）
    pub async fn get_active_task_lists(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskList>, RepositoryError> {
        let document = self
            .get_project_document(project_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("Project not found: {}", project_id))
            })?;
        Ok(document
            .task_lists
            .into_iter()
            .filter(|tl| !tl.is_deleted())
            .collect())
    }

    /// 削除済みタスクリストのみ取得（deleted=trueのみ）
    pub async fn get_deleted_task_lists(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<TaskList>, RepositoryError> {
        let document = self
            .get_project_document(project_id)
            .await?
            .ok_or_else(|| {
                RepositoryError::NotFound(format!("Project not found: {}", project_id))
            })?;
        Ok(document
            .task_lists
            .into_iter()
            .filter(|tl| tl.is_deleted())
            .collect())
    }

    /// プロジェクトを復元（deleted=falseに設定）
    pub async fn restore_project(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        if let Some(mut project) = self.get_project(&project_id.to_string()).await? {
            if !project.is_deleted() {
                return Err(RepositoryError::InvalidOperation(format!(
                    "Project is not deleted: {}",
                    project_id
                )));
            }

            // Trackableトレイトを使用して復元
            project.mark_restored(*user_id, *timestamp);
            self.set_project(&project).await
        } else {
            Err(RepositoryError::NotFound(format!(
                "Project not found: {}",
                project_id
            )))
        }
    }

    /// プロジェクト内のすべてのタスクを復元
    pub async fn restore_all_tasks(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_all(project_id, &TASKS, |task| {
            restore_if_deleted(task, user_id, timestamp)
        })
        .await
    }

    /// プロジェクト内のすべてのタグを復元
    pub async fn restore_all_tags(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_all(project_id, &TAGS, |tag| {
            restore_if_deleted(tag, user_id, timestamp)
        })
        .await
    }

    /// プロジェクト内のすべてのタスクリストを復元
    pub async fn restore_all_task_lists(
        &self,
        project_id: &ProjectId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_all(project_id, &TASK_LISTS, |task_list| {
            restore_if_deleted(task_list, user_id, timestamp)
        })
        .await
    }

    /// 個別タスクの論理削除
    pub async fn mark_task_deleted(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_one(
            project_id,
            &TASKS,
            &task_id.to_string(),
            || RepositoryError::NotFound(format!("Task not found: {}", task_id)),
            |task: &mut Task| {
                task.mark_deleted(*user_id, *timestamp);
                Ok(())
            },
        )
        .await
    }

    /// 個別タグの論理削除
    pub async fn mark_tag_deleted(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_one(
            project_id,
            &TAGS,
            &tag_id.to_string(),
            || RepositoryError::NotFound(format!("Tag not found: {}", tag_id)),
            |tag: &mut Tag| {
                tag.mark_deleted(*user_id, *timestamp);
                Ok(())
            },
        )
        .await
    }

    /// 個別タスクリストの論理削除
    pub async fn mark_task_list_deleted(
        &self,
        project_id: &ProjectId,
        task_list_id: &TaskListId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_one(
            project_id,
            &TASK_LISTS,
            &task_list_id.to_string(),
            || RepositoryError::NotFound(format!("TaskList not found: {}", task_list_id)),
            |task_list: &mut TaskList| {
                task_list.mark_deleted(*user_id, *timestamp);
                Ok(())
            },
        )
        .await
    }

    /// 個別タスクの復元
    pub async fn restore_task(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_one(
            project_id,
            &TASKS,
            &task_id.to_string(),
            || RepositoryError::NotFound(format!("Task not found: {}", task_id)),
            |task: &mut Task| {
                restore_deleted(task, "Task", &task_id.to_string(), user_id, timestamp)
            },
        )
        .await
    }

    /// 個別タグの復元
    pub async fn restore_tag(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_one(
            project_id,
            &TAGS,
            &tag_id.to_string(),
            || RepositoryError::NotFound(format!("Tag not found: {}", tag_id)),
            |tag: &mut Tag| restore_deleted(tag, "Tag", &tag_id.to_string(), user_id, timestamp),
        )
        .await
    }

    /// 個別タスクリストの復元
    pub async fn restore_task_list(
        &self,
        project_id: &ProjectId,
        task_list_id: &TaskListId,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        self.update_one(
            project_id,
            &TASK_LISTS,
            &task_list_id.to_string(),
            || RepositoryError::NotFound(format!("TaskList not found: {}", task_list_id)),
            |task_list: &mut TaskList| {
                restore_deleted(
                    task_list,
                    "TaskList",
                    &task_list_id.to_string(),
                    user_id,
                    timestamp,
                )
            },
        )
        .await
    }

    /// 削除済み個別タスクの取得
    pub async fn get_deleted_task_by_id(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Result<Option<Task>, RepositoryError> {
        self.find_deleted(project_id, &TASKS, &task_id.to_string())
            .await
    }

    /// 削除済み個別タグの取得
    pub async fn get_deleted_tag_by_id(
        &self,
        project_id: &ProjectId,
        tag_id: &TagId,
    ) -> Result<Option<Tag>, RepositoryError> {
        self.find_deleted(project_id, &TAGS, &tag_id.to_string())
            .await
    }

    /// 削除済み個別タスクリストの取得
    pub async fn get_deleted_task_list_by_id(
        &self,
        project_id: &ProjectId,
        task_list_id: &TaskListId,
    ) -> Result<Option<TaskList>, RepositoryError> {
        self.find_deleted(project_id, &TASK_LISTS, &task_list_id.to_string())
            .await
    }
}

/// ドキュメント直下のプロジェクト基本情報を読む。必須の値（ID・名前・作成日時・更新日時）が無ければ `None`
async fn load_project(document: &Document) -> Result<Option<Project>, RepositoryError> {
    let id: Option<String> = document.load_data("id").await?;
    let name: Option<String> = document.load_data("name").await?;
    let description: Option<Option<String>> = document.load_data("description").await?;
    let color: Option<Option<String>> = document.load_data("color").await?;
    let order_index: Option<i32> = document.load_data("order_index").await?;
    let is_archived: Option<bool> = document.load_data("is_archived").await?;
    let status: Option<Option<ProjectStatus>> = document.load_data("status").await?;
    let owner_id: Option<Option<UserId>> = document.load_data("owner_id").await?;
    let created_at: Option<DateTime<Utc>> = document.load_data("created_at").await?;
    let updated_at: Option<DateTime<Utc>> = document.load_data("updated_at").await?;
    let updated_by: Option<UserId> = document.load_data("updated_by").await?;
    let deleted: Option<bool> = document.load_data("deleted").await?;

    let (Some(id), Some(name), Some(created_at), Some(updated_at)) =
        (id, name, created_at, updated_at)
    else {
        return Ok(None);
    };
    Ok(Some(Project {
        id: ProjectId::from(id.clone()),
        name,
        description: description.unwrap_or(None),
        color: color.unwrap_or(None),
        order_index: order_index.unwrap_or(0),
        is_archived: is_archived.unwrap_or(false),
        status: status.unwrap_or(None),
        owner_id: owner_id.unwrap_or(None),
        created_at,
        updated_at,
        updated_by: updated_by.unwrap_or_else(|| UserId::from(id)),
        deleted: deleted.unwrap_or(false),
    }))
}

/// ドキュメント直下にプロジェクト基本情報を書く。変わった値だけが書かれる
async fn save_project(document: &Document, project: &Project) -> Result<(), RepositoryError> {
    document.save_data("id", &project.id.to_string()).await?;
    document.save_data("name", &project.name).await?;
    document
        .save_data("description", &project.description)
        .await?;
    document.save_data("color", &project.color).await?;
    document
        .save_data("order_index", &project.order_index)
        .await?;
    document
        .save_data("is_archived", &project.is_archived)
        .await?;
    document.save_data("status", &project.status).await?;
    document.save_data("owner_id", &project.owner_id).await?;
    document
        .save_data("created_at", &project.created_at)
        .await?;
    document
        .save_data("updated_at", &project.updated_at)
        .await?;
    document
        .save_data("updated_by", &project.updated_by)
        .await?;
    document.save_data("deleted", &project.deleted).await?;
    Ok(())
}

/// 削除済みなら復元して真を返す
fn restore_if_deleted<T: Trackable>(
    entity: &mut T,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> bool {
    if !entity.is_deleted() {
        return false;
    }
    entity.mark_restored(*user_id, *timestamp);
    true
}

/// 削除済みのエンティティを復元する。削除済みでなければ `InvalidOperation`
fn restore_deleted<T: Trackable>(
    entity: &mut T,
    kind: &str,
    id: &str,
    user_id: &UserId,
    timestamp: &DateTime<Utc>,
) -> Result<(), RepositoryError> {
    if !entity.is_deleted() {
        return Err(RepositoryError::InvalidOperation(format!(
            "{kind} is not deleted: {id}"
        )));
    }
    entity.mark_restored(*user_id, *timestamp);
    Ok(())
}

#[async_trait]
impl Repository<Project, ProjectId> for ProjectLocalAutomergeRepository {
    async fn save(
        &self,
        entity: &Project,
        user_id: &UserId,
        timestamp: &DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        tracing::info!(
            "ProjectLocalAutomergeRepository::save - 開始: {:?}",
            entity.id
        );

        // updated_by と updated_at を更新
        let mut updated_entity = entity.clone();
        updated_entity.updated_by = *user_id;
        updated_entity.updated_at = *timestamp;

        let result = self.set_project(&updated_entity).await;
        if result.is_ok() {
            tracing::info!(
                "ProjectLocalAutomergeRepository::save - 完了: {:?}",
                entity.id
            );
        } else {
            tracing::error!(
                "ProjectLocalAutomergeRepository::save - エラー: {:?}",
                result
            );
        }
        result
    }

    async fn find_by_id(&self, id: &ProjectId) -> Result<Option<Project>, RepositoryError> {
        self.get_project(&id.to_string()).await
    }

    async fn find_all(&self) -> Result<Vec<Project>, RepositoryError> {
        // 注意: この実装は個別プロジェクト管理の範囲外
        // 一覧取得はProjectListLocalAutomergeRepositoryを使用してください
        Err(RepositoryError::NotFound(
            "Use ProjectListLocalAutomergeRepository for project listing".to_string(),
        ))
    }

    async fn delete(&self, id: &ProjectId) -> Result<(), RepositoryError> {
        // プロジェクトドキュメント自体を削除
        // 注意: プロジェクト一覧からの削除は別途ProjectListLocalAutomergeRepositoryで行ってください
        self.delete_project_document(id).await
    }

    async fn exists(&self, id: &ProjectId) -> Result<bool, RepositoryError> {
        let found = self.find_by_id(id).await?;
        Ok(found.is_some())
    }

    async fn count(&self) -> Result<u64, RepositoryError> {
        // 注意: この実装は個別プロジェクト管理の範囲外
        // カウント取得はProjectListLocalAutomergeRepositoryを使用してください
        Err(RepositoryError::NotFound(
            "Use ProjectListLocalAutomergeRepository for project counting".to_string(),
        ))
    }
}

#[async_trait]
impl ProjectRepositoryTrait for ProjectLocalAutomergeRepository {}
