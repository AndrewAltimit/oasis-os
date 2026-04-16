//! List marker painting: disc, circle, square, ordered (decimal,
//! alphabetic, roman, custom @counter-style).

use crate::css::parser::CounterStyleRule;
use crate::layout::box_model::{LayoutBox, ListMarker, ListMarkerStyle};
use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

use super::PaintContext;

pub(super) fn paint_list_marker(
    marker: &ListMarker,
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let content = &layout_box.dimensions.content;
    let x = (content.x - ctx.scroll_x + offset_x as f32 - 20.0) as i32;
    let y = (content.y - ctx.scroll_y + offset_y as f32) as i32;
    let color = layout_box.style.color;
    let font_size = layout_box.style.font_size as u16;

    match marker {
        ListMarker::Disc => {
            backend.draw_text("\u{2022}", x, y, font_size, color)?;
        },
        ListMarker::Circle => {
            backend.draw_text("\u{25E6}", x, y, font_size, color)?;
        },
        ListMarker::Square => {
            backend.draw_text("\u{25AA}", x, y, font_size, color)?;
        },
        ListMarker::Ordered(style, n) => {
            let text = format_ordered_marker(style, *n, &ctx.counter_styles);
            backend.draw_text(&text, x - 10, y, font_size, color)?;
        },
        ListMarker::None => {},
    }

    Ok(())
}

/// Format an ordered list marker using the given style and counter value.
pub(super) fn format_ordered_marker_public(
    style: &ListMarkerStyle,
    n: usize,
    counter_styles: &[CounterStyleRule],
) -> String {
    format_ordered_marker(style, n, counter_styles)
}

fn format_ordered_marker(
    style: &ListMarkerStyle,
    n: usize,
    counter_styles: &[CounterStyleRule],
) -> String {
    match style {
        ListMarkerStyle::Decimal => format!("{}.", n),
        ListMarkerStyle::DecimalLeadingZero => format!("{:02}.", n),
        ListMarkerStyle::LowerAlpha => format!("{}.", to_alpha(n, false)),
        ListMarkerStyle::UpperAlpha => format!("{}.", to_alpha(n, true)),
        ListMarkerStyle::LowerRoman => format!("{}.", to_roman(n, false)),
        ListMarkerStyle::UpperRoman => format!("{}.", to_roman(n, true)),
        ListMarkerStyle::Custom(name) => {
            if let Some(rule) = counter_styles.iter().find(|r| r.name == *name) {
                format_counter_style(rule, n)
            } else {
                // Fallback to decimal if @counter-style not found.
                format!("{}.", n)
            }
        },
    }
}

/// Convert a 1-based counter to alphabetic representation (a..z, aa..az, ..).
fn to_alpha(n: usize, upper: bool) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut result = String::new();
    let mut val = n;
    while val > 0 {
        val -= 1;
        let c = if upper {
            (b'A' + (val % 26) as u8) as char
        } else {
            (b'a' + (val % 26) as u8) as char
        };
        result.insert(0, c);
        val /= 26;
    }
    result
}

/// Convert a 1-based counter to roman numeral representation.
fn to_roman(n: usize, upper: bool) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    let table: &[(usize, &str)] = &[
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut result = String::new();
    let mut val = n;
    for &(value, symbol) in table {
        while val >= value {
            result.push_str(symbol);
            val -= value;
        }
    }
    if upper {
        result.make_ascii_uppercase();
    }
    result
}

/// Format a counter value using a `@counter-style` rule.
fn format_counter_style(rule: &CounterStyleRule, n: usize) -> String {
    let prefix = rule.prefix.as_deref().unwrap_or("");
    let suffix = rule.suffix.as_deref().unwrap_or(". ");
    let system = rule.system.as_deref().unwrap_or("symbolic");

    let repr = match system {
        "cyclic" => {
            if rule.symbols.is_empty() {
                n.to_string()
            } else {
                let idx = (n - 1) % rule.symbols.len();
                rule.symbols[idx].clone()
            }
        },
        "numeric" => {
            if rule.symbols.len() < 2 {
                n.to_string()
            } else {
                format_numeric(&rule.symbols, n)
            }
        },
        "alphabetic" => {
            if rule.symbols.is_empty() {
                n.to_string()
            } else {
                format_alphabetic(&rule.symbols, n)
            }
        },
        "symbolic" => {
            if rule.symbols.is_empty() {
                n.to_string()
            } else {
                format_symbolic(&rule.symbols, n)
            }
        },
        "additive" => format_additive(&rule.additive_symbols, n),
        "fixed" => {
            if rule.symbols.is_empty() || n == 0 || n > rule.symbols.len() {
                // Out of range — fall back to decimal.
                n.to_string()
            } else {
                rule.symbols[n - 1].clone()
            }
        },
        _ => n.to_string(),
    };

    format!("{prefix}{repr}{suffix}")
}

/// Numeric system: positional base-N using the symbol list.
fn format_numeric(symbols: &[String], n: usize) -> String {
    if n == 0 {
        return symbols[0].clone();
    }
    let base = symbols.len();
    let mut result = Vec::new();
    let mut val = n;
    while val > 0 {
        result.push(symbols[val % base].clone());
        val /= base;
    }
    result.reverse();
    result.join("")
}

/// Alphabetic system: bijective base-N (like spreadsheet columns).
fn format_alphabetic(symbols: &[String], n: usize) -> String {
    if n == 0 {
        return "0".into();
    }
    let base = symbols.len();
    let mut result = Vec::new();
    let mut val = n;
    while val > 0 {
        val -= 1;
        result.push(symbols[val % base].clone());
        val /= base;
    }
    result.reverse();
    result.join("")
}

/// Symbolic system: repeat each symbol N times where N = ceil(counter/len).
fn format_symbolic(symbols: &[String], n: usize) -> String {
    if n == 0 {
        return "".into();
    }
    let idx = (n - 1) % symbols.len();
    let repeats = (n - 1) / symbols.len() + 1;
    symbols[idx].repeat(repeats)
}

/// Additive system: greedy decomposition with (weight, symbol) pairs.
fn format_additive(symbols: &[(i32, String)], n: usize) -> String {
    if symbols.is_empty() {
        return n.to_string();
    }
    let mut val = n as i32;
    let mut result = String::new();
    for (weight, symbol) in symbols {
        if *weight <= 0 {
            continue;
        }
        while val >= *weight {
            result.push_str(symbol);
            val -= weight;
        }
    }
    if val > 0 {
        // Can't fully represent — fall back.
        return n.to_string();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_lower() {
        assert_eq!(to_alpha(1, false), "a");
        assert_eq!(to_alpha(26, false), "z");
        assert_eq!(to_alpha(27, false), "aa");
    }

    #[test]
    fn alpha_upper() {
        assert_eq!(to_alpha(1, true), "A");
        assert_eq!(to_alpha(3, true), "C");
    }

    #[test]
    fn roman_lower() {
        assert_eq!(to_roman(1, false), "i");
        assert_eq!(to_roman(4, false), "iv");
        assert_eq!(to_roman(9, false), "ix");
        assert_eq!(to_roman(42, false), "xlii");
    }

    #[test]
    fn roman_upper() {
        assert_eq!(to_roman(14, true), "XIV");
        assert_eq!(to_roman(2024, true), "MMXXIV");
    }

    #[test]
    fn counter_style_cyclic() {
        let rule = CounterStyleRule {
            name: "thumbs".into(),
            system: Some("cyclic".into()),
            symbols: vec!["\u{1F44D}".into()],
            additive_symbols: vec![],
            range: None,
            prefix: None,
            suffix: Some(" ".into()),
            pad: None,
            negative: None,
            fallback: None,
            speak_as: None,
        };
        assert_eq!(format_counter_style(&rule, 1), "\u{1F44D} ");
        assert_eq!(format_counter_style(&rule, 5), "\u{1F44D} ");
    }

    #[test]
    fn counter_style_alphabetic() {
        let rule = CounterStyleRule {
            name: "abc".into(),
            system: Some("alphabetic".into()),
            symbols: vec!["a".into(), "b".into(), "c".into()],
            additive_symbols: vec![],
            range: None,
            prefix: None,
            suffix: Some(". ".into()),
            pad: None,
            negative: None,
            fallback: None,
            speak_as: None,
        };
        assert_eq!(format_counter_style(&rule, 1), "a. ");
        assert_eq!(format_counter_style(&rule, 3), "c. ");
        assert_eq!(format_counter_style(&rule, 4), "aa. ");
    }

    #[test]
    fn counter_style_additive() {
        let rule = CounterStyleRule {
            name: "dice".into(),
            system: Some("additive".into()),
            symbols: vec![],
            additive_symbols: vec![
                (6, "\u{2685}".into()),
                (5, "\u{2684}".into()),
                (4, "\u{2683}".into()),
                (3, "\u{2682}".into()),
                (2, "\u{2681}".into()),
                (1, "\u{2680}".into()),
            ],
            range: None,
            prefix: None,
            suffix: Some(" ".into()),
            pad: None,
            negative: None,
            fallback: None,
            speak_as: None,
        };
        assert_eq!(format_counter_style(&rule, 1), "\u{2680} ");
        assert_eq!(format_counter_style(&rule, 7), "\u{2685}\u{2680} ");
    }

    #[test]
    fn ordered_marker_decimal_leading_zero() {
        let s = format_ordered_marker(&ListMarkerStyle::DecimalLeadingZero, 3, &[]);
        assert_eq!(s, "03.");
    }

    #[test]
    fn ordered_marker_lower_roman() {
        let s = format_ordered_marker(&ListMarkerStyle::LowerRoman, 4, &[]);
        assert_eq!(s, "iv.");
    }
}
