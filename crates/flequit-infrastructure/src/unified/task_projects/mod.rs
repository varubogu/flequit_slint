//! タスク・プロジェクト管理統合リポジトリ
//!
//! プロジェクト、タスク（サブタスクを含む）、タグ、メンバー、アサイン、繰り返し設定など
//! タスク管理の核心機能を提供する統合リポジトリ群。

// 基本エンティティ
pub mod member;
pub mod project;
pub mod tag;
pub mod task;
pub mod task_list;

// 関連テーブル
pub mod recurrence_rule;
pub mod task_assignments;
pub mod task_recurrence;
pub mod task_tag;

// 公開エクスポート
pub use project::ProjectUnifiedRepository;
pub use recurrence_rule::RecurrenceRuleUnifiedRepository;
pub use tag::TagUnifiedRepository;
pub use task::TaskUnifiedRepository;
pub use task_assignments::TaskAssignmentUnifiedRepository;
pub use task_list::TaskListUnifiedRepository;
pub use task_recurrence::TaskRecurrenceUnifiedRepository;
pub use task_tag::TaskTagUnifiedRepository;
