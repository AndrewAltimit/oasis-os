//! Container queries and `will-change` shorthand parsing.
//!
//! Free helpers used by `apply_declaration` to interpret the
//! `container`, `container-name`, `container-type`, and `will-change`
//! properties. Kept separate from the main dispatch so the dispatch
//! file stays focused on the per-property `match` arm.

use super::super::types::ContainerType;
use crate::css::parser::CssValue;

/// True if a `will-change` value names any property that benefits
/// from layer promotion in our pipeline.
///
/// Recognised hints are `transform`, `opacity`, `filter`,
/// `scroll-position`, and `contents`. All other identifiers (and
/// `auto`) leave the flag false.
pub(super) fn will_change_promotes(value: &CssValue) -> bool {
    fn kw_promotes(kw: &str) -> bool {
        matches!(
            kw.trim(),
            "transform" | "opacity" | "filter" | "scroll-position" | "contents"
        )
    }
    match value {
        CssValue::Keyword(s) => kw_promotes(s),
        CssValue::String(s) => s.split([',', ' ']).any(kw_promotes),
        CssValue::Multiple(parts) => parts.iter().any(will_change_promotes),
        _ => false,
    }
}

/// Parse a `container-name` value: a list of identifiers, the
/// keyword `none`, or empty. Returns the list of names; `none`
/// produces an empty list.
pub(super) fn parse_container_name_list(value: &CssValue) -> Vec<String> {
    fn push_ident(out: &mut Vec<String>, kw: &str) {
        let kw = kw.trim();
        if kw.is_empty() || kw.eq_ignore_ascii_case("none") {
            return;
        }
        out.push(kw.to_string());
    }
    let mut out = Vec::new();
    match value {
        CssValue::Keyword(kw) => {
            for tok in kw.split_whitespace() {
                push_ident(&mut out, tok);
            }
        },
        CssValue::String(s) => {
            for tok in s.split_whitespace() {
                push_ident(&mut out, tok);
            }
        },
        CssValue::Multiple(parts) => {
            for p in parts {
                out.extend(parse_container_name_list(p));
            }
        },
        _ => {},
    }
    out
}

/// Parse a `container` shorthand: `<name> [/ <type>]`.
///
/// Examples:
/// - `container: card` → name = ["card"], type = None
/// - `container: card / inline-size` → name = ["card"], type = InlineSize
/// - `container: none / size` → name = [], type = Size
pub(super) fn parse_container_shorthand(value: &CssValue) -> (Vec<String>, Option<ContainerType>) {
    // Flatten into a single string and split on `/`.
    fn flatten(v: &CssValue, out: &mut String) {
        match v {
            CssValue::Keyword(kw) => {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(kw);
            },
            CssValue::String(s) => {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(s);
            },
            CssValue::Multiple(parts) => {
                for p in parts {
                    flatten(p, out);
                }
            },
            _ => {},
        }
    }
    let mut raw = String::new();
    flatten(value, &mut raw);
    let mut split = raw.splitn(2, '/');
    let name_part = split.next().unwrap_or("").trim();
    let type_part = split.next().map(|s| s.trim());

    let names = if name_part.eq_ignore_ascii_case("none") || name_part.is_empty() {
        Vec::new()
    } else {
        name_part
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    };

    let ty = type_part.and_then(|t| match t.to_ascii_lowercase().as_str() {
        "normal" => Some(ContainerType::Normal),
        "inline-size" => Some(ContainerType::InlineSize),
        "size" => Some(ContainerType::Size),
        _ => None,
    });

    (names, ty)
}
