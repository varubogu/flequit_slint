//! ViewModel layer.
//!
//! ViewModels are the only place that registers Slint callbacks and the only
//! place that calls `flequit-core` facades. They own the `VecModel`s backing the
//! UI lists, apply optimistic updates, and roll them back when persistence fails.
//!
//! See `docs/ja/develop/design/ui/viewmodel-architecture.md`.

pub mod app;
pub mod task_list_ui;

pub use app::AppViewModel;
pub use task_list_ui::TaskListUiViewModel;
