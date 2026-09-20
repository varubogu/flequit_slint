//! Automerge 同期キューの SQLite エンティティ
//!
//! 1 行が「Automerge へ反映する 1 件の変更」。`payload` の中身は
//! `flequit-infrastructure` が決める JSON で、このクレートは解釈しない。

use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// 行の状態（`status` 列の値）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncQueueStatus {
    /// Automerge へ未反映
    Pending,
    /// Automerge へ反映済み。保持期間を過ぎると削除される
    Processed,
    /// 再試行しても反映できなかった。削除せず調査用に残す
    Failed,
}

impl SyncQueueStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processed => "processed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "automerge_sync_queue")]
pub struct Model {
    /// 適用順。AUTOINCREMENT のため再利用されない
    #[sea_orm(primary_key)]
    pub id: i64,

    /// 反映先の Automerge ドキュメント（例: `project:{uuid}`）。
    /// 同じキーの行は id 順に 1 件ずつ適用する
    pub document_key: String,

    /// 変更の種類（例: `task.save`）。ログと調査用
    pub change_kind: String,

    /// 変更内容の JSON
    pub payload: String,

    /// `pending` / `processed` / `failed`
    pub status: String,

    /// 反映を試みた回数
    pub attempts: i32,

    /// 最後に失敗したときのエラー
    pub last_error: Option<String>,

    /// この時刻まで再試行しない（失敗後のバックオフ）
    pub next_attempt_at: Option<DateTime<Utc>>,

    /// キューへ入れた時刻（SQLite のコミットと同じトランザクション）
    pub created_at: DateTime<Utc>,

    /// Automerge へ反映した時刻
    pub processed_at: Option<DateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
