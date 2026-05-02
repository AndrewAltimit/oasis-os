//! `@supports` condition evaluation.
//!
//! Decides at parse time whether the property name in an `@supports
//! (prop: value)` condition is recognised by `apply_declaration`.
//! Conditions for unknown properties evaluate to `false` and the
//! enclosed rules are dropped from the stylesheet.

/// List of CSS property names recognized by `apply_declaration`.
pub(super) const SUPPORTED_PROPERTIES: &[&str] = &[
    "display",
    "visibility",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border-width",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-style",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "width",
    "height",
    "max-width",
    "min-width",
    "max-height",
    "min-height",
    "color",
    "font-size",
    "font-weight",
    "font-style",
    "font-family",
    "text-align",
    "text-decoration",
    "text-indent",
    "text-transform",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "background-color",
    "background",
    "list-style-type",
    "list-style-position",
    "border-collapse",
    "border-spacing",
    "float",
    "clear",
    "overflow",
    "position",
    "top",
    "right",
    "bottom",
    "left",
    "z-index",
    "flex-direction",
    "flex-wrap",
    "justify-content",
    "align-items",
    "align-content",
    "align-self",
    "order",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "gap",
    "grid-gap",
    "column-gap",
    "grid-column-gap",
    "row-gap",
    "grid-row-gap",
    "grid-template-columns",
    "grid-template-rows",
    "grid-column-start",
    "grid-column-end",
    "grid-column",
    "grid-row-start",
    "grid-row-end",
    "grid-row",
    "border-radius",
    "opacity",
    "box-shadow",
    "text-shadow",
    "box-sizing",
    "vertical-align",
    "background-image",
    "background-size",
    "background-position",
    "background-repeat",
    "text-decoration-line",
    "text-decoration-color",
    "text-decoration-style",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
    "word-break",
    "overflow-wrap",
    "word-wrap",
    "text-overflow",
    "content",
    "outline-width",
    "outline-color",
    "outline-style",
    "outline-offset",
    "outline",
    "transition",
    "direction",
    "animation",
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
    "transform",
    "transform-origin",
    "filter",
    "counter-reset",
    "counter-increment",
    "grid-auto-flow",
    "grid-template-areas",
    "grid-area",
    "table-layout",
    "will-change",
    "tab-size",
    "column-count",
    "column-width",
    "columns",
    "overflow-x",
    "overflow-y",
    "cursor",
    "pointer-events",
    "user-select",
    "aspect-ratio",
    "text-underline-offset",
    "object-position",
    "place-items",
    "place-content",
    "appearance",
    "-webkit-appearance",
    "-moz-appearance",
    "-webkit-line-clamp",
    "line-clamp",
    "-webkit-box-orient",
    "accent-color",
    "caret-color",
    "color-scheme",
    "isolation",
    "resize",
    "touch-action",
    "grid-template",
    // Mask longhands (compositor overhaul PR6).
    "mask-image",
    "mask-mode",
    "mask-composite",
    "mask-clip",
    "mask-origin",
    "mask-position",
    "mask-size",
    "mask-repeat",
    // Container queries.
    "container-type",
    "container-name",
    "container",
    // Form control sizing.
    "field-sizing",
];

/// Evaluate an `@supports` condition string.
///
/// Supports simple `(property: value)` conditions and `not (...)`.
/// Unknown or unsupported conditions evaluate to `false`.
pub(super) fn eval_supports_condition(condition: &str) -> bool {
    let condition = condition.trim();
    if condition.is_empty() {
        return false;
    }

    // Handle `not (...)`.
    if let Some(rest) = condition.strip_prefix("not ") {
        return !eval_supports_condition(rest.trim());
    }

    // Handle compound `(...) and (...)`
    if condition.contains(") and (") {
        let parts: Vec<&str> = condition.split(") and (").collect();
        return parts.iter().all(|p| {
            let trimmed = p.trim().trim_start_matches('(').trim_end_matches(')');
            eval_supports_single(&format!("({trimmed})"))
        });
    }

    // Handle compound `(...) or (...)`
    if condition.contains(") or (") {
        let parts: Vec<&str> = condition.split(") or (").collect();
        return parts.iter().any(|p| {
            let trimmed = p.trim().trim_start_matches('(').trim_end_matches(')');
            eval_supports_single(&format!("({trimmed})"))
        });
    }

    eval_supports_single(condition)
}

/// Evaluate a single `(property: value)` supports condition.
fn eval_supports_single(condition: &str) -> bool {
    let inner = condition
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();

    // Split on the first colon to get property name.
    if let Some(colon_pos) = inner.find(':') {
        let property = inner[..colon_pos].trim();
        // Check if property is supported (exists in apply_declaration).
        // Also allow custom properties (--*).
        if property.starts_with("--") {
            return true;
        }
        SUPPORTED_PROPERTIES.contains(&property)
    } else {
        false
    }
}
