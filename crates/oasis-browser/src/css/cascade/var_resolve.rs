//! CSS custom property (`var()`) resolution.
//!
//! Recursively resolves `CssValue::Var` references using the element's
//! custom property map, with cycle detection via a depth limit.

use rustc_hash::FxHashMap;

use super::super::parser::{CssValue, parse_value_list};
use super::super::tokenizer::CssTokenizer;

/// Recursively resolve `CssValue::Var` references using the element's
/// custom property map.
///
/// If the custom property exists, its raw CSS text is re-tokenized and
/// re-parsed. If not, the fallback value is used. If neither exists,
/// an empty keyword is returned (property will be silently ignored).
pub(super) fn resolve_css_var(value: &CssValue, props: &FxHashMap<String, String>) -> CssValue {
    resolve_css_var_depth(value, props, 0)
}

/// Maximum recursion depth for `var()` resolution. Prevents stack overflow
/// on cyclic custom properties like `--a: var(--a)`.
const MAX_VAR_DEPTH: u32 = 16;

fn resolve_css_var_depth(
    value: &CssValue,
    props: &FxHashMap<String, String>,
    depth: u32,
) -> CssValue {
    if depth >= MAX_VAR_DEPTH {
        return CssValue::Keyword(String::new());
    }
    match value {
        CssValue::Var(name, fallback) => {
            let raw = props
                .get(name.as_str())
                .map(|s| s.as_str())
                .or(fallback.as_deref());
            if let Some(css_text) = raw {
                let tokens = CssTokenizer::new(css_text).tokenize();
                let parsed = parse_value_list(&tokens);
                match parsed.len() {
                    0 => CssValue::Keyword(String::new()),
                    1 => {
                        if let Some(v) = parsed.into_iter().next() {
                            // Handle chained var() references.
                            resolve_css_var_depth(&v, props, depth + 1)
                        } else {
                            CssValue::Keyword(String::new())
                        }
                    },
                    _ => {
                        let resolved: Vec<CssValue> = parsed
                            .into_iter()
                            .map(|v| resolve_css_var_depth(&v, props, depth + 1))
                            .collect();
                        CssValue::Multiple(resolved)
                    },
                }
            } else {
                CssValue::Keyword(String::new())
            }
        },
        CssValue::Multiple(parts) => {
            let resolved: Vec<CssValue> = parts
                .iter()
                .map(|p| resolve_css_var_depth(p, props, depth + 1))
                .collect();
            CssValue::Multiple(resolved)
        },
        other => other.clone(),
    }
}
