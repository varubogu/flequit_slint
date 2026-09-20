//! Automerge 同期キュー
//!
//! 書き込みは SQLite だけを同期的に行い、Automerge への反映はキューを介して
//! バックグラウンドで行う。Automerge の保存（ドキュメント全体の読み書き）を
//! 待たずに操作を確定させ、応答を速くするため。
//!
//! ```text
//! 統合リポジトリ ──(1 トランザクション)──▶ キュー行 INSERT + SQLite CRUD → COMMIT
//!                                                   │ 通知
//!                                                   ▼
//!                                 ワーカー: id 順に Automerge へ適用 → processed
//!                                           processed は 30 日後に削除
//! ```
//!
//! 設計: `docs/ja/develop/design/data/automerge-sync-queue.md`

pub mod apply;
pub mod change;
pub mod queue;
pub mod worker;

pub use apply::AutomergeSyncTargets;
pub use change::{
    AutomergeChange, ProjectChange, RelationChange, RootChange, TagBookmarkChange, TrashChange,
};
pub use queue::{AutomergeSyncQueue, QueuedSqlite, QueuedTransaction};
pub use worker::{AutomergeSyncHandle, AutomergeSyncProcessor, SyncPassReport, spawn_worker};
