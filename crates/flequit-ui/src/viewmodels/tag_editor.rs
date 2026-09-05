//! The writes behind the tag manager and the detail pane's tag area.
//!
//! Pure: the UI sends plain text, and these functions turn it into the domain
//! values a facade expects. Validation lives here so that the manager, the
//! "create and assign" field and any future entry point all agree on what a
//! usable tag name is.

use chrono::Utc;
use flequit_model::models::task_projects::tag::{PartialTag, Tag};
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use flequit_model::types::id_types::{ProjectId, TagBookmarkId, TagId, UserId};

use crate::adapters::color::parse_hex;

/// Trims a name and rejects it when it is empty or contains whitespace.
///
/// Whitespace is refused rather than replaced: a tag is searched for as
/// `#name`, and the search splits its input on whitespace, so a two-word tag
/// could never be typed back into the search box.
///
/// # Examples
///
/// ```
/// use flequit_ui::viewmodels::tag_editor::validated_name;
///
/// assert_eq!(validated_name("  home "), Some("home".to_string()));
/// assert_eq!(validated_name("two words"), None);
/// ```
pub fn validated_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty() && !trimmed.chars().any(char::is_whitespace)).then(|| trimmed.to_string())
}

/// Normalises a colour coming from the editor.
///
/// Anything that is not a hex colour becomes "no colour": the value is chosen
/// from a palette, so a different one means the UI and the domain disagree and
/// storing it would only spread the confusion.
pub fn validated_color(raw: &str) -> Option<String> {
    parse_hex(raw).map(|_| raw.trim().to_string())
}

/// A new tag, ready to be created in the project the manager is showing.
///
/// The project is not recorded on the tag itself: the association is written
/// separately by the facade, which is why no project id is taken here.
pub fn new_tag(name: String, color: Option<String>, order_index: i32, user_id: UserId) -> Tag {
    let now = Utc::now();
    Tag {
        id: TagId::new(),
        name,
        color,
        order_index: Some(order_index),
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: user_id,
    }
}

/// The patch for renaming and recolouring a tag.
///
/// The colour is always written, including when it was cleared, so that
/// choosing "no colour" is a change rather than a no-op.
pub fn tag_patch(name: String, color: Option<String>) -> PartialTag {
    PartialTag {
        name: Some(name),
        color: Some(color),
        ..Default::default()
    }
}

/// A new sidebar pin for `tag_id`.
///
/// The pin is per user and records its project, because the sidebar lists pins
/// from every project and selecting one has to move to the owning project.
pub fn new_bookmark(
    user_id: UserId,
    project_id: ProjectId,
    tag_id: TagId,
    order_index: i32,
) -> TagBookmark {
    let now = Utc::now();
    TagBookmark {
        id: TagBookmarkId::new(),
        user_id,
        project_id,
        tag_id,
        order_index,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_name_is_rejected() {
        for raw in ["", " ", "\t\n"] {
            assert_eq!(validated_name(raw), None);
        }
    }

    #[test]
    fn a_name_that_would_not_survive_the_search_box_is_rejected() {
        // The search splits on whitespace, so "#two words" could never be typed
        // back in. A full-width space counts as whitespace, so a name made only
        // of one is empty once trimmed.
        for raw in ["two words", "a\tb", "\u{3000}"] {
            assert_eq!(validated_name(raw), None, "{raw:?} should be rejected");
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_rather_than_rejected() {
        assert_eq!(validated_name("  home\n"), Some("home".to_string()));
    }

    #[test]
    fn a_palette_colour_survives_validation() {
        assert_eq!(validated_color("#4c6ef5"), Some("#4c6ef5".to_string()));
        assert_eq!(validated_color("  #4c6ef5  "), Some("#4c6ef5".to_string()));
    }

    #[test]
    fn anything_that_is_not_a_colour_becomes_none() {
        for raw in ["", "blue", "#12345"] {
            assert_eq!(validated_color(raw), None);
        }
    }

    #[test]
    fn a_tag_patch_can_clear_its_colour() {
        let patch = tag_patch("Home".to_string(), None);

        assert_eq!(patch.name, Some("Home".to_string()));
        assert_eq!(patch.color, Some(None));
    }

    #[test]
    fn a_new_tag_keeps_its_requested_order() {
        let user_id = UserId::new();
        let tag = new_tag("Home".to_string(), None, 4, user_id);

        assert_eq!(tag.name, "Home");
        assert_eq!(tag.order_index, Some(4));
        assert_eq!(tag.updated_by, user_id);
        assert!(!tag.deleted);
    }

    #[test]
    fn a_new_bookmark_records_the_project_that_owns_the_tag() {
        let user_id = UserId::new();
        let project_id = ProjectId::new();
        let tag_id = TagId::new();

        let bookmark = new_bookmark(user_id, project_id, tag_id, 2);

        assert_eq!(bookmark.user_id, user_id);
        assert_eq!(bookmark.project_id, project_id);
        assert_eq!(bookmark.tag_id, tag_id);
        assert_eq!(bookmark.order_index, 2);
    }
}
