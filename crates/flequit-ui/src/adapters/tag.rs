//! Domain tags and bookmarks converted into display-ready Slint rows.

use flequit_model::models::task_projects::tag::Tag;
use flequit_model::models::user_preferences::tag_bookmark::TagBookmark;
use slint::SharedString;

use super::color::parse_hex;
use crate::bindings::{BookmarkedTagItem, FilterHighlight, TagItem};

/// Converts a tag into the row shown by the tag manager and the detail pane.
///
/// The owning project is passed in rather than read from the tag: a tag knows
/// nothing about its project, but every write needs the project id, and the
/// caller already knows which project it is listing.
///
/// `bookmarked` and `assigned` are relationships held elsewhere — in the user's
/// pins and in the selected task — so they are supplied by the caller too.
///
/// An unparseable colour is treated as unset rather than as black: the value
/// comes from storage and may predate the current palette. The original text is
/// kept alongside the brush so the editor can send the same value back.
pub fn to_tag_item(tag: &Tag, project_id: &str, bookmarked: bool, assigned: bool) -> TagItem {
    let parsed = tag.color.as_deref().and_then(parse_hex);

    TagItem {
        id: SharedString::from(tag.id.as_str()),
        project_id: SharedString::from(project_id),
        name: SharedString::from(tag.name.as_str()),
        color: SharedString::from(parsed.and(tag.color.as_deref()).unwrap_or_default()),
        color_brush: parsed.unwrap_or_default().into(),
        has_color: parsed.is_some(),
        bookmarked,
        assigned,
    }
}

/// Converts a pinned tag into the row shown in the sidebar.
///
/// The project comes from the bookmark, not from the current selection: the
/// sidebar lists pins from every project, and selecting one has to move to the
/// project the tag belongs to.
///
/// The colour text is dropped: a pin is not editable from the sidebar, so only
/// the rendered brush is needed.
pub fn to_bookmarked_tag_item(tag: &Tag, bookmark: &TagBookmark) -> BookmarkedTagItem {
    let parsed = tag.color.as_deref().and_then(parse_hex);

    BookmarkedTagItem {
        id: SharedString::from(tag.id.as_str()),
        project_id: SharedString::from(bookmark.project_id.as_str()),
        name: SharedString::from(tag.name.as_str()),
        color_brush: parsed.unwrap_or_default().into(),
        has_color: parsed.is_some(),
        highlight: FilterHighlight::None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use flequit_model::types::id_types::{ProjectId, TagBookmarkId, TagId, UserId};

    use super::*;

    fn tag(name: &str, color: Option<&str>) -> Tag {
        let now = Utc::now();
        Tag {
            id: TagId::new(),
            name: name.to_string(),
            color: color.map(str::to_string),
            order_index: Some(0),
            created_at: now,
            updated_at: now,
            deleted: false,
            updated_by: UserId::new(),
        }
    }

    fn bookmark(project_id: ProjectId, tag_id: TagId) -> TagBookmark {
        let now = Utc::now();
        TagBookmark {
            id: TagBookmarkId::new(),
            user_id: UserId::new(),
            project_id,
            tag_id,
            order_index: 0,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn a_tag_row_carries_the_project_it_was_listed_under() {
        let item = to_tag_item(&tag("home", None), "p1", false, true);

        assert_eq!(item.project_id, "p1");
        assert_eq!(item.name, "home");
        assert!(!item.bookmarked);
        assert!(item.assigned);
    }

    #[test]
    fn a_palette_colour_reaches_the_row_as_both_text_and_brush() {
        let item = to_tag_item(&tag("home", Some("#4c6ef5")), "p1", false, false);

        assert!(item.has_color);
        assert_eq!(item.color, "#4c6ef5");
        assert_eq!(item.color_brush.color().red(), 0x4c);
    }

    #[test]
    fn an_unusable_colour_is_reported_as_no_colour() {
        for stored in [None, Some("teal"), Some("#12345")] {
            let item = to_tag_item(&tag("home", stored), "p1", false, false);

            assert!(!item.has_color, "{stored:?} should not count as a colour");
            assert_eq!(item.color, "", "{stored:?} should not reach the editor");
        }
    }

    #[test]
    fn a_pinned_row_points_at_the_project_that_owns_the_tag() {
        let tag = tag("home", Some("#4c6ef5"));
        let project_id = ProjectId::new();
        let bookmark = bookmark(project_id, tag.id);

        let item = to_bookmarked_tag_item(&tag, &bookmark);

        assert_eq!(item.id, tag.id.as_str());
        assert_eq!(item.project_id, project_id.as_str());
        assert!(item.has_color);
    }
}
