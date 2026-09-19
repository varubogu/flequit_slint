//! The search box: its query language, and how the query drives the task list.
//!
//! The specification is `docs/ja/develop/design/ui/page/main/search.md`. In
//! short: `@` names a due date, a state, a project, a list or a user; `#` names
//! a tag; anything else is free text; conditions combine with AND (space), OR
//! (`|`), NOT (`-`) and parentheses. The box shows short names while the
//! internal query keeps what each confirmed `@` token points at.
//!
//! Everything here is pure so the rules can be tested without a window.

mod document;
pub mod due;
mod eval;
mod lexer;
mod normalize;
mod parser;
mod resolve;
mod session;
mod sidebar;
mod suggestion;
mod vocabulary;

pub use document::Segment;
pub use due::{DueKeyword, DueName, DueSpec};
pub use eval::{EvalContext, Prepared, SearchUnit, UnitText};
pub use normalize::fold;
pub use resolve::{ListEntry, NameIndex, ProjectEntry, Reference, UserEntry};
pub use session::{SearchSession, SearchView};
pub use sidebar::{Highlight, ItemKey, QueryEdit, tag_key};
pub use suggestion::{Suggestion, SuggestionKind};
pub use vocabulary::{Lang, StatusKey, UnitStatus};
