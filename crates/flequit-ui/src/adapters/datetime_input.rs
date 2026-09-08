//! Parsing of hand-typed dates and times.
//!
//! The pickers accept free text so a date can be entered without paging a
//! calendar. Users type what their locale taught them — `2026/9/15`,
//! `2026-09-15 13:30`, `9月15日 13時30分`, `20260915`, or just `15` — so the
//! parser reads separators rather than one fixed pattern.
//!
//! Everything here is pure: the caller supplies the reference date that fills
//! in the parts the input leaves out.

use chrono::NaiveDate;

use super::datetime::DateTimeParts;

/// A successfully parsed input.
///
/// `has_time` distinguishes "the user typed a date only" from "the user typed
/// midnight", which decides whether the picker keeps the time it already had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeInput {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub has_time: bool,
}

/// Reads a date, and optionally a time, out of what the user typed.
///
/// Returns `None` when the text does not describe a real date, so the caller
/// can leave the picker on its previous value instead of jumping somewhere
/// arbitrary.
///
/// # Examples
///
/// ```
/// use flequit_ui::adapters::datetime::DateTimeParts;
/// use flequit_ui::adapters::datetime_input::parse_datetime_input;
///
/// let reference = DateTimeParts { year: 2026, month: 9, day: 8, hour: 0, minute: 0 };
/// let parsed = parse_datetime_input("2026/9/15 13:30", reference).unwrap();
///
/// assert_eq!((parsed.year, parsed.month, parsed.day), (2026, 9, 15));
/// assert_eq!((parsed.hour, parsed.minute, parsed.has_time), (13, 30, true));
/// ```
pub fn parse_datetime_input(input: &str, reference: DateTimeParts) -> Option<DateTimeInput> {
    let normalized = normalize(input);
    let (date_text, time_text) = split_date_and_time(&normalized);

    let date = parse_date(&date_text, reference)?;
    let time = match time_text {
        Some(text) => Some(parse_time(&text)?),
        None => None,
    };

    Some(DateTimeInput {
        year: date.0,
        month: date.1,
        day: date.2,
        hour: time.map_or(reference.hour, |(hour, _)| hour),
        minute: time.map_or(reference.minute, |(_, minute)| minute),
        has_time: time.is_some(),
    })
}

/// Folds the many ways of writing a separator into `/` for dates and `:` for
/// times, so the split below only has to know two of them.
///
/// Full-width digits and colons are included because a Japanese IME produces
/// them whenever the user forgets to leave conversion mode.
fn normalize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());

    for ch in input.chars() {
        match ch {
            '０'..='９' => out.push(char::from(b'0' + (ch as u32 - '０' as u32) as u8)),
            '年' | '月' | '-' | '.' | '／' => out.push('/'),
            '時' | '：' => out.push(':'),
            '日' | '分' | 'T' | 't' | '\u{3000}' => out.push(' '),
            '秒' => out.push(' '),
            _ => out.push(ch),
        }
    }

    out
}

/// Splits the normalized text into its date and time halves.
///
/// The time is whichever whitespace-separated chunk carries a `:`; anything
/// else belongs to the date. A trailing separator (`2026/9/15/`) is dropped so
/// `9月15日` and `9/15` parse the same way.
fn split_date_and_time(normalized: &str) -> (String, Option<String>) {
    let mut date = Vec::new();
    let mut time = None;

    for chunk in normalized.split_whitespace() {
        if chunk.contains(':') {
            time = Some(chunk.to_string());
        } else {
            date.push(chunk);
        }
    }

    (
        date.join("/").trim_matches('/').to_string(),
        time.filter(|value| !value.is_empty()),
    )
}

fn parse_date(text: &str, reference: DateTimeParts) -> Option<(i32, i32, i32)> {
    let numbers = split_numbers(text)?;

    let (year, month, day) = match numbers.as_slice() {
        [year, month, day] => (expand_year(*year), *month, *day),
        [month, day] => (reference.year, *month, *day),
        [single] => return from_digits(text, *single, reference),
        _ => return None,
    };

    valid_date(year, month, day)
}

/// Reads a run of digits that carries no separator at all.
///
/// The length says what it is: `20260915` is a full date, `260915` a two-digit
/// year, `0915` a month and a day inside the reference year, and one or two
/// digits a day inside the reference month.
fn from_digits(text: &str, value: i32, reference: DateTimeParts) -> Option<(i32, i32, i32)> {
    match text.len() {
        8 => valid_date(value / 10_000, value / 100 % 100, value % 100),
        6 => valid_date(expand_year(value / 10_000), value / 100 % 100, value % 100),
        4 => valid_date(reference.year, value / 100, value % 100),
        1 | 2 => valid_date(reference.year, reference.month, value),
        _ => None,
    }
}

fn parse_time(text: &str) -> Option<(i32, i32)> {
    let numbers = split_numbers(text)?;

    let (hour, minute) = match numbers.as_slice() {
        [hour, minute] | [hour, minute, _] => (*hour, *minute),
        [hour] => (*hour, 0),
        _ => return None,
    };

    (0..24).contains(&hour).then_some(())?;
    (0..60).contains(&minute).then_some(())?;
    Some((hour, minute))
}

/// Splits on every non-digit and rejects anything that is not a plain number.
///
/// Rejecting rather than skipping keeps `2026/9/x5` from silently becoming
/// `2026/9/5`.
fn split_numbers(text: &str) -> Option<Vec<i32>> {
    if text.is_empty()
        || !text
            .chars()
            .all(|ch| ch.is_ascii_digit() || is_separator(ch))
    {
        return None;
    }

    text.split(is_separator)
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<i32>().ok())
        .collect()
}

fn is_separator(ch: char) -> bool {
    matches!(ch, '/' | ':' | ',')
}

/// Reads a one- or two-digit year as this century, the way a form would.
fn expand_year(year: i32) -> i32 {
    if year < 100 { 2000 + year } else { year }
}

fn valid_date(year: i32, month: i32, day: i32) -> Option<(i32, i32, i32)> {
    NaiveDate::from_ymd_opt(year, u32::try_from(month).ok()?, u32::try_from(day).ok()?)
        .map(|_| (year, month, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> DateTimeParts {
        DateTimeParts {
            year: 2026,
            month: 9,
            day: 8,
            hour: 10,
            minute: 30,
        }
    }

    fn parse(input: &str) -> Option<DateTimeInput> {
        parse_datetime_input(input, reference())
    }

    #[test]
    fn every_common_date_separator_reads_the_same() {
        for input in ["2026/9/15", "2026-9-15", "2026.09.15", "2026年9月15日"] {
            let parsed = parse(input).unwrap_or_else(|| panic!("{input} should parse"));
            assert_eq!((parsed.year, parsed.month, parsed.day), (2026, 9, 15));
            assert!(!parsed.has_time, "{input} carries no time");
        }
    }

    #[test]
    fn a_time_is_recognised_next_to_the_date() {
        for input in [
            "2026/9/15 13:30",
            "2026-09-15T13:30",
            "2026年9月15日 13時30分",
        ] {
            let parsed = parse(input).unwrap_or_else(|| panic!("{input} should parse"));
            assert_eq!((parsed.year, parsed.month, parsed.day), (2026, 9, 15));
            assert_eq!((parsed.hour, parsed.minute), (13, 30));
            assert!(parsed.has_time);
        }
    }

    #[test]
    fn a_date_without_a_time_keeps_the_time_it_was_given() {
        let parsed = parse("2026/9/15").expect("a bare date parses");

        assert_eq!((parsed.hour, parsed.minute), (10, 30));
        assert!(!parsed.has_time);
    }

    #[test]
    fn the_reference_fills_in_what_was_left_out() {
        let month_and_day = parse("9/15").expect("a month and a day parse");
        assert_eq!(
            (month_and_day.year, month_and_day.month, month_and_day.day),
            (2026, 9, 15)
        );

        let day_only = parse("15").expect("a bare day parses");
        assert_eq!((day_only.year, day_only.month, day_only.day), (2026, 9, 15));
    }

    #[test]
    fn digits_without_separators_are_read_by_length() {
        assert_eq!(
            parse("20260915").map(|parsed| (parsed.year, parsed.month, parsed.day)),
            Some((2026, 9, 15))
        );
        assert_eq!(
            parse("260915").map(|parsed| (parsed.year, parsed.month, parsed.day)),
            Some((2026, 9, 15))
        );
        assert_eq!(
            parse("0915").map(|parsed| (parsed.year, parsed.month, parsed.day)),
            Some((2026, 9, 15))
        );
    }

    #[test]
    fn full_width_digits_from_an_ime_are_accepted() {
        let parsed = parse("２０２６／９／１５　１３：３０").expect("full-width input parses");

        assert_eq!((parsed.year, parsed.month, parsed.day), (2026, 9, 15));
        assert_eq!((parsed.hour, parsed.minute), (13, 30));
    }

    #[test]
    fn a_date_that_does_not_exist_is_rejected() {
        assert_eq!(parse("2026/2/30"), None);
        assert_eq!(parse("2026/13/1"), None);
    }

    #[test]
    fn an_impossible_time_is_rejected() {
        assert_eq!(parse("2026/9/15 25:00"), None);
        assert_eq!(parse("2026/9/15 12:70"), None);
    }

    #[test]
    fn text_that_is_not_a_date_is_rejected() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   "), None);
        assert_eq!(parse("tomorrow"), None);
        assert_eq!(parse("2026/9/x5"), None);
    }
}
