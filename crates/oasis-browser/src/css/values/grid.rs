//! CSS Grid value parsing: track lists and placement lines.
//!
//! Grid properties are kept as raw CSS text by the declaration parser
//! (the generic value-list parser cannot represent `repeat()`,
//! `minmax()` or the `/` separator), so everything here works on text
//! with a small paren/bracket-aware tokenizer.

use super::types::{
    GridAutoRepeat, GridLine, GridTemplate, GridTrackSize, TrackBreadth, current_root_font_size,
};
use crate::css::parser::{CssValue, LengthUnit};

/// Upper bound on integer `repeat()` counts, to keep hostile CSS such as
/// `repeat(100000000, 1px)` from allocating unbounded track lists.
const MAX_REPEAT: usize = 1000;

// -----------------------------------------------------------------------
// CssValue -> text
// -----------------------------------------------------------------------

fn unit_suffix(unit: LengthUnit) -> &'static str {
    match unit {
        LengthUnit::Px => "px",
        LengthUnit::Em => "em",
        LengthUnit::Rem => "rem",
        LengthUnit::Pt => "pt",
        LengthUnit::Ex => "ex",
        LengthUnit::Ch => "ch",
    }
}

/// Render a parsed value back to CSS-ish text so programmatic callers
/// (tests, JS `style.gridColumn = ...`) that pass structured values go
/// through the same text parser as stylesheet declarations.
fn value_text(value: &CssValue) -> Option<String> {
    Some(match value {
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        CssValue::Number(n) => format!("{n}"),
        CssValue::Length(n, unit) => format!("{n}{}", unit_suffix(*unit)),
        CssValue::Percentage(n) => format!("{n}%"),
        CssValue::Multiple(parts) => {
            let texts: Vec<String> = parts.iter().filter_map(value_text).collect();
            texts.join(" ")
        },
        _ => return None,
    })
}

// -----------------------------------------------------------------------
// Tokenizer
// -----------------------------------------------------------------------

/// Split `s` at top-level separators (outside `()` and `[]`), dropping
/// empty pieces. `sep` decides which characters separate.
fn split_top_level(s: &str, sep: impl Fn(char) -> bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 && sep(c) => {
                let piece = s[start..i].trim();
                if !piece.is_empty() {
                    out.push(piece);
                }
                start = i + c.len_utf8();
            },
            _ => {},
        }
    }
    let piece = s[start..].trim();
    if !piece.is_empty() {
        out.push(piece);
    }
    out
}

/// If `s` is `name(...)` (case-insensitive name), return the inner text.
fn function_args<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = s.find('(')?;
    if !s[..open].trim().eq_ignore_ascii_case(name) {
        return None;
    }
    s[open + 1..].strip_suffix(')')
}

// -----------------------------------------------------------------------
// Track lists
// -----------------------------------------------------------------------

/// Parse a length-ish token to px. Supports px/em/rem/pt/ex/ch and
/// bare numbers (treated as px, matching the legacy behaviour for `0`).
fn parse_length_px(s: &str, font_size: f32) -> Option<f32> {
    let s = s.trim();
    let (num, factor) = if let Some(n) = s.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix("rem") {
        (n, current_root_font_size())
    } else if let Some(n) = s.strip_suffix("em") {
        (n, font_size)
    } else if let Some(n) = s.strip_suffix("pt") {
        (n, 1.333)
    } else if let Some(n) = s.strip_suffix("ex") {
        (n, font_size * 0.5)
    } else if let Some(n) = s.strip_suffix("ch") {
        (n, font_size * 0.5)
    } else {
        (s, 1.0)
    };
    let v = num.trim().parse::<f32>().ok()?;
    v.is_finite().then_some(v * factor)
}

/// Parse a single track breadth (`200px`, `25%`, `1fr`, `auto`, ...).
fn parse_breadth(s: &str, font_size: f32) -> Option<TrackBreadth> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    match lower.as_str() {
        "auto" => return Some(TrackBreadth::Auto),
        "min-content" => return Some(TrackBreadth::MinContent),
        "max-content" => return Some(TrackBreadth::MaxContent),
        _ => {},
    }
    if let Some(fr) = lower.strip_suffix("fr") {
        let v = fr.trim().parse::<f32>().ok()?;
        return (v.is_finite() && v >= 0.0).then_some(TrackBreadth::Fr(v));
    }
    if let Some(pct) = lower.strip_suffix('%') {
        let v = pct.trim().parse::<f32>().ok()?;
        return (v.is_finite() && v >= 0.0).then_some(TrackBreadth::Percent(v));
    }
    let px = parse_length_px(&lower, font_size)?;
    (px >= 0.0).then_some(TrackBreadth::Px(px))
}

/// Parse one track size (breadth, `minmax()` or `fit-content()`).
pub(super) fn parse_track(s: &str, font_size: f32) -> Option<GridTrackSize> {
    let s = s.trim();
    if let Some(args) = function_args(s, "minmax") {
        let parts = split_top_level(args, |c| c == ',');
        if parts.len() != 2 {
            return None;
        }
        let min = parse_breadth(parts[0], font_size)?;
        let max = parse_breadth(parts[1], font_size)?;
        // A flexible minimum is invalid per spec.
        if matches!(min, TrackBreadth::Fr(_)) {
            return None;
        }
        return Some(GridTrackSize::Minmax(min, max));
    }
    if let Some(arg) = function_args(s, "fit-content") {
        return parse_length_px(arg, font_size).map(|px| GridTrackSize::FitContent(px.max(0.0)));
    }
    Some(match parse_breadth(s, font_size)? {
        TrackBreadth::Px(v) => GridTrackSize::Px(v),
        TrackBreadth::Percent(v) => GridTrackSize::Percent(v),
        TrackBreadth::Fr(v) => GridTrackSize::Fr(v),
        TrackBreadth::Auto => GridTrackSize::Auto,
        TrackBreadth::MinContent => GridTrackSize::MinContent,
        TrackBreadth::MaxContent => GridTrackSize::MaxContent,
    })
}

/// Parse the tracks inside a `repeat()` (or a whole list), skipping
/// `[line-name]` groups.
fn parse_plain_tracks(s: &str, font_size: f32) -> Vec<GridTrackSize> {
    split_top_level(s, char::is_whitespace)
        .into_iter()
        .filter(|tok| !tok.starts_with('['))
        .filter_map(|tok| parse_track(tok, font_size))
        .collect()
}

/// Parse a full `grid-template-columns` / `-rows` track list string.
///
/// Handles nested functions (`repeat(3, minmax(0, 1fr))`), multi-track
/// repeats (`repeat(2, 1fr 2fr)`), `repeat(auto-fill | auto-fit, ...)`,
/// line-name brackets (ignored) and all track breadth forms.
pub(super) fn parse_track_list(s: &str, font_size: f32) -> GridTemplate {
    let s = s.trim();
    let mut out = GridTemplate::default();
    if s.eq_ignore_ascii_case("none") {
        return out;
    }
    for tok in split_top_level(s, char::is_whitespace) {
        if tok.starts_with('[') {
            continue;
        }
        let Some(args) = function_args(tok, "repeat") else {
            if let Some(t) = parse_track(tok, font_size) {
                out.tracks.push(t);
            }
            continue;
        };
        let Some((count, list)) = args.split_once(',') else {
            continue;
        };
        let tracks = parse_plain_tracks(list, font_size);
        if tracks.is_empty() {
            continue;
        }
        let count = count.trim().to_ascii_lowercase();
        match count.as_str() {
            "auto-fill" | "auto-fit" => {
                // Only one auto-repeat is allowed per track list.
                if out.auto_repeat.is_none() {
                    out.auto_repeat = Some(GridAutoRepeat {
                        insert_at: out.tracks.len(),
                        tracks,
                        fit: count == "auto-fit",
                    });
                }
            },
            n => {
                if let Ok(n) = n.parse::<usize>() {
                    let n = n.min(MAX_REPEAT / tracks.len().max(1));
                    for _ in 0..n {
                        out.tracks.extend_from_slice(&tracks);
                    }
                }
            },
        }
    }
    out
}

/// Parse a `grid-template-*` value into a [`GridTemplate`].
pub(super) fn parse_grid_template(value: &CssValue, font_size: f32) -> GridTemplate {
    match value_text(value) {
        Some(text) => parse_track_list(&text, font_size),
        None => GridTemplate::default(),
    }
}

/// Parse a `grid-auto-rows` / `grid-auto-columns` value (a plain track
/// list; auto-repeat is not allowed there and is dropped).
pub(super) fn parse_grid_auto_tracks(value: &CssValue, font_size: f32) -> Vec<GridTrackSize> {
    parse_grid_template(value, font_size).tracks
}

// -----------------------------------------------------------------------
// Placement
// -----------------------------------------------------------------------

/// Parse a single `<grid-line>` value: `auto`, `<integer>`, `span <n>`,
/// `<custom-ident>`. Unsupported combos (`span name`, `2 name`) fall
/// back to their numeric part or `auto`.
pub(super) fn parse_grid_line(s: &str) -> GridLine {
    let words: Vec<&str> = s.split_whitespace().collect();
    let mut span = false;
    let mut number = None;
    let mut name = None;
    for w in &words {
        if w.eq_ignore_ascii_case("span") {
            span = true;
        } else if w.eq_ignore_ascii_case("auto") {
            return GridLine::Auto;
        } else if let Ok(n) = w.parse::<f32>() {
            number = Some(n as i32);
        } else {
            name = Some(*w);
        }
    }
    match (span, number, name) {
        (true, Some(n), _) if n > 0 => GridLine::Span(n as u32),
        (true, None, _) => GridLine::Span(1),
        (true, _, _) => GridLine::Auto,
        (false, Some(0), _) => GridLine::Auto,
        (false, Some(n), _) => GridLine::Line(n),
        (false, None, Some(id)) => GridLine::Named(id.to_string()),
        (false, None, None) => GridLine::Auto,
    }
}

/// Parse a single-line value (`grid-column-start` etc.).
pub(super) fn parse_grid_line_value(value: &CssValue) -> GridLine {
    value_text(value)
        .map(|t| parse_grid_line(&t))
        .unwrap_or_default()
}

/// The end line implied when a `grid-row` / `grid-column` / `grid-area`
/// shorthand omits it: the same ident for a named start, else `auto`.
fn implied_end(start: &GridLine) -> GridLine {
    match start {
        GridLine::Named(n) => GridLine::Named(n.clone()),
        _ => GridLine::Auto,
    }
}

/// Parse `grid-column` / `grid-row`: `<start> [ / <end> ]?`.
pub(super) fn parse_grid_line_pair(value: &CssValue) -> (GridLine, GridLine) {
    let Some(text) = value_text(value) else {
        return (GridLine::Auto, GridLine::Auto);
    };
    let mut parts = text.split('/');
    let start = parse_grid_line(parts.next().unwrap_or(""));
    let end = match parts.next() {
        Some(e) => parse_grid_line(e),
        None => implied_end(&start),
    };
    (start, end)
}

/// Parse `grid-auto-flow: [row | column] || dense` into
/// `(column_flow, dense)`. Returns `None` for unrecognised values.
pub(super) fn parse_grid_auto_flow(value: &CssValue) -> Option<(bool, bool)> {
    let text = value_text(value)?.to_ascii_lowercase();
    let mut column = false;
    let mut dense = false;
    for w in text.split_whitespace() {
        match w {
            "row" => {},
            "column" => column = true,
            "dense" => dense = true,
            _ => return None,
        }
    }
    Some((column, dense))
}

/// Parsed `grid-area` shorthand.
pub(super) struct GridAreaValue {
    /// The area name, when the value is a single custom ident.
    pub name: Option<String>,
    pub row_start: GridLine,
    pub column_start: GridLine,
    pub row_end: GridLine,
    pub column_end: GridLine,
}

/// Parse `grid-area: <row-start> [ / <col-start> [ / <row-end> [ / <col-end> ]]]`.
pub(super) fn parse_grid_area(value: &CssValue) -> GridAreaValue {
    let text = value_text(value).unwrap_or_default();
    let parts: Vec<GridLine> = text.split('/').map(parse_grid_line).collect();
    let get = |i: usize| parts.get(i).cloned();
    let row_start = get(0).unwrap_or_default();
    let column_start = get(1).unwrap_or_else(|| implied_end(&row_start));
    let row_end = get(2).unwrap_or_else(|| implied_end(&row_start));
    let column_end = get(3).unwrap_or_else(|| implied_end(&column_start));
    let name = match (&row_start, parts.len()) {
        (GridLine::Named(n), 1) => Some(n.clone()),
        _ => None,
    };
    GridAreaValue {
        name,
        row_start,
        column_start,
        row_end,
        column_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw(s: &str) -> CssValue {
        CssValue::Keyword(s.into())
    }

    #[test]
    fn compound_repeat() {
        let t = parse_track_list("repeat(3, 1fr) 20px", 16.0);
        assert_eq!(
            t.tracks,
            vec![
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(1.0),
                GridTrackSize::Px(20.0),
            ]
        );
        let t = parse_track_list("100px repeat(2, auto)", 16.0);
        assert_eq!(
            t.tracks,
            vec![
                GridTrackSize::Px(100.0),
                GridTrackSize::Auto,
                GridTrackSize::Auto
            ]
        );
        assert!(parse_track_list("none", 16.0).is_empty());
    }

    #[test]
    fn repeat_with_nested_minmax() {
        let t = parse_track_list("repeat(3, minmax(0, 1fr))", 16.0);
        assert_eq!(
            t.tracks,
            vec![GridTrackSize::Minmax(TrackBreadth::Px(0.0), TrackBreadth::Fr(1.0)); 3]
        );
        assert!(t.auto_repeat.is_none());
    }

    #[test]
    fn repeat_multi_track() {
        let t = parse_track_list("repeat(2, 1fr 2fr) 50px", 16.0);
        assert_eq!(
            t.tracks,
            vec![
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(2.0),
                GridTrackSize::Fr(1.0),
                GridTrackSize::Fr(2.0),
                GridTrackSize::Px(50.0),
            ]
        );
    }

    #[test]
    fn repeat_auto_fill_recorded_for_layout() {
        let t = parse_track_list("100px repeat(auto-fill, minmax(200px, 1fr))", 16.0);
        assert_eq!(t.tracks, vec![GridTrackSize::Px(100.0)]);
        let rep = t.auto_repeat.expect("auto repeat");
        assert_eq!(rep.insert_at, 1);
        assert!(!rep.fit);
        assert_eq!(
            rep.tracks,
            vec![GridTrackSize::Minmax(
                TrackBreadth::Px(200.0),
                TrackBreadth::Fr(1.0)
            )]
        );
    }

    #[test]
    fn repeat_auto_fit_flag() {
        let t = parse_track_list("repeat(auto-fit, 100px)", 16.0);
        assert!(t.auto_repeat.expect("auto repeat").fit);
    }

    #[test]
    fn track_units_percent_em_rem_content() {
        let t = parse_track_list(
            "25% 2em 1rem min-content max-content fit-content(120px)",
            10.0,
        );
        assert_eq!(
            t.tracks,
            vec![
                GridTrackSize::Percent(25.0),
                GridTrackSize::Px(20.0),
                GridTrackSize::Px(current_root_font_size()),
                GridTrackSize::MinContent,
                GridTrackSize::MaxContent,
                GridTrackSize::FitContent(120.0),
            ]
        );
    }

    #[test]
    fn line_names_are_skipped() {
        let t = parse_track_list(
            "[full-start] 1fr [main-start] 2fr [main-end full-end]",
            16.0,
        );
        assert_eq!(
            t.tracks,
            vec![GridTrackSize::Fr(1.0), GridTrackSize::Fr(2.0)]
        );
    }

    #[test]
    fn invalid_minmax_flexible_min_rejected() {
        assert_eq!(parse_track("minmax(1fr, 100px)", 16.0), None);
    }

    #[test]
    fn huge_repeat_is_capped() {
        let t = parse_track_list("repeat(100000000, 1px)", 16.0);
        assert_eq!(t.tracks.len(), MAX_REPEAT);
    }

    #[test]
    fn line_pair_slash_syntax() {
        assert_eq!(
            parse_grid_line_pair(&kw("1 / 3")),
            (GridLine::Line(1), GridLine::Line(3))
        );
        assert_eq!(
            parse_grid_line_pair(&kw("1 / -1")),
            (GridLine::Line(1), GridLine::Line(-1))
        );
        assert_eq!(
            parse_grid_line_pair(&kw("span 2")),
            (GridLine::Span(2), GridLine::Auto)
        );
        assert_eq!(
            parse_grid_line_pair(&kw("2 / span 3")),
            (GridLine::Line(2), GridLine::Span(3))
        );
        assert_eq!(
            parse_grid_line_pair(&kw("header")),
            (
                GridLine::Named("header".into()),
                GridLine::Named("header".into())
            )
        );
        assert_eq!(
            parse_grid_line_pair(&CssValue::Number(4.0)),
            (GridLine::Line(4), GridLine::Auto)
        );
    }

    #[test]
    fn grid_area_numeric_and_named() {
        let a = parse_grid_area(&kw("1 / 2 / 3 / 4"));
        assert!(a.name.is_none());
        assert_eq!(a.row_start, GridLine::Line(1));
        assert_eq!(a.column_start, GridLine::Line(2));
        assert_eq!(a.row_end, GridLine::Line(3));
        assert_eq!(a.column_end, GridLine::Line(4));

        let a = parse_grid_area(&kw("main"));
        assert_eq!(a.name.as_deref(), Some("main"));
        assert_eq!(a.column_end, GridLine::Named("main".into()));
    }
}
