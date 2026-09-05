//! Colour values exchanged with the UI.
//!
//! The domain stores a colour as a hex string, which `.slint` cannot render:
//! there is no text-to-colour conversion in the language. Parsing therefore
//! happens here, and both the parsed colour and the original text are handed to
//! the UI — the text so the editor can send the same value back.

use slint::Color;

/// The colours a project can be tagged with.
///
/// A fixed palette rather than a free colour picker: these have to stay legible
/// against both the light and the dark surface, which an arbitrary colour does
/// not.
pub const PROJECT_COLORS: &[&str] = &[
    "#4c6ef5", // blue
    "#15aabf", // cyan
    "#40c057", // green
    "#fd7e14", // orange
    "#fa5252", // red
    "#be4bdb", // violet
    "#868e96", // grey
];

/// Parses `#rrggbb` into a colour, ignoring case and surrounding whitespace.
///
/// Returns `None` for anything else, including the shorthand `#rgb` form: the
/// palette only ever produces the long form, and a half-understood colour is
/// worse than none.
///
/// # Examples
///
/// ```
/// use flequit_ui::adapters::color::parse_hex;
///
/// assert!(parse_hex("#4C6EF5").is_some());
/// assert!(parse_hex("#fff").is_none());
/// assert!(parse_hex("blue").is_none());
/// ```
pub fn parse_hex(value: &str) -> Option<Color> {
    let digits = value.trim().strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
    Some(Color::from_rgb_u8(
        channel(0..2)?,
        channel(2..4)?,
        channel(4..6)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_entry_parses() {
        for value in PROJECT_COLORS {
            assert!(parse_hex(value).is_some(), "{value} is not a valid colour");
        }
    }

    #[test]
    fn the_channels_are_read_in_order() {
        let color = parse_hex("#0a1422").expect("a valid colour");

        assert_eq!(
            (color.red(), color.green(), color.blue()),
            (0x0a, 0x14, 0x22)
        );
    }

    #[test]
    fn malformed_values_are_rejected() {
        for value in ["", "#", "#12345", "#1234567", "#gggggg", "4c6ef5"] {
            assert!(parse_hex(value).is_none(), "{value} should not parse");
        }
    }
}
