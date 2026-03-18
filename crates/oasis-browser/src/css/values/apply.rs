//! `ComputedStyle::apply_declaration` and `apply_initial` implementations.

use oasis_types::backend::Color;

use super::computed::ComputedStyle;
use super::resolve::{
    as_keyword, parse_grid_template, resolve_border_style, resolve_color, resolve_color_or_current,
    resolve_dimension, resolve_font_size, resolve_font_weight, resolve_length, resolve_line_height,
};
use super::types::{
    AlignContent, AlignItems, AlignSelf, Animation, AnimationDirection, AnimationFillMode,
    AnimationPlayState, BackgroundImage, BorderCollapse, BorderStyle, BoxSizing, Clear, Display,
    FlexDirection, FlexWrap, Float, FontFamily, FontStyle, JustifyContent, ListStylePosition,
    ListStyleType, ObjectFit, Overflow, OverflowWrap, Position, TextAlign, TextDecoration,
    TextOverflow, TextShadow, TextTransform, TimingFunction, Transition, VerticalAlign, Visibility,
    WhiteSpace, WordBreak,
};
use crate::css::parser::CssValue;

impl ComputedStyle {
    /// Apply a parsed CSS declaration to this style.
    ///
    /// Resolves relative units (`em`, `%`) against the parent font size
    /// so the resulting computed value is in absolute pixels.
    pub(crate) fn apply_declaration(
        &mut self,
        property: &str,
        value: &CssValue,
        parent_font_size: f32,
    ) {
        // Custom properties (--*) are stored in the properties map.
        if property.starts_with("--") {
            if let CssValue::String(ref raw) = *value {
                self.custom_properties
                    .insert(property.to_string(), raw.clone());
            }
            return;
        }

        // Handle `inherit` and `initial` keywords for any property.
        if let Some(kw) = as_keyword(value) {
            if kw == "initial" {
                self.apply_initial(property);
                return;
            }
            if kw == "inherit" {
                // The caller has already set up `self` via `inherit(parent)`,
                // so inherited properties already carry the parent value.
                // For non-inherited properties, we need the parent's computed
                // value. Since we don't have the parent here, we rely on the
                // cascade having called `inherit(parent)` beforehand -- the
                // parent_font_size parameter gives us the parent font context.
                // For properties that are already inherited (color, font-*, etc.)
                // the current value is already correct. For non-inherited
                // properties, `inherit` is rare; do nothing extra here.
                return;
            }
        }

        match property {
            // -- Display ------------------------------------------------
            "display" => {
                if let Some(kw) = as_keyword(value) {
                    self.display = match kw {
                        "block" => Display::Block,
                        "inline" => Display::Inline,
                        "inline-block" => Display::InlineBlock,
                        "list-item" => Display::ListItem,
                        "table" => Display::Table,
                        "table-row" => Display::TableRow,
                        "table-cell" => Display::TableCell,
                        "flex" => Display::Flex,
                        "grid" => Display::Grid,
                        "none" => Display::None,
                        _ => return,
                    };
                }
            },
            "visibility" => {
                if let Some(kw) = as_keyword(value) {
                    self.visibility = match kw {
                        "visible" => Visibility::Visible,
                        "hidden" => Visibility::Hidden,
                        _ => return,
                    };
                }
            },

            // -- Margins ------------------------------------------------
            "margin" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_top = 0.0;
                    self.margin_right = 0.0;
                    self.margin_bottom = 0.0;
                    self.margin_left = 0.0;
                    self.margin_left_auto = true;
                    self.margin_right_auto = true;
                    self.margin_top_auto = true;
                    self.margin_bottom_auto = true;
                    self.margin_top_pct = None;
                    self.margin_right_pct = None;
                    self.margin_bottom_pct = None;
                    self.margin_left_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_top_pct = Some(*p);
                    self.margin_right_pct = Some(*p);
                    self.margin_bottom_pct = Some(*p);
                    self.margin_left_pct = Some(*p);
                    self.margin_top = 0.0;
                    self.margin_right = 0.0;
                    self.margin_bottom = 0.0;
                    self.margin_left = 0.0;
                    self.margin_left_auto = false;
                    self.margin_right_auto = false;
                    self.margin_top_auto = false;
                    self.margin_bottom_auto = false;
                } else {
                    let px = resolve_length(value, parent_font_size);
                    self.margin_top = px;
                    self.margin_right = px;
                    self.margin_bottom = px;
                    self.margin_left = px;
                    self.margin_left_auto = false;
                    self.margin_right_auto = false;
                    self.margin_top_auto = false;
                    self.margin_bottom_auto = false;
                    self.margin_top_pct = None;
                    self.margin_right_pct = None;
                    self.margin_bottom_pct = None;
                    self.margin_left_pct = None;
                }
            },
            "margin-top" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_top = 0.0;
                    self.margin_top_auto = true;
                    self.margin_top_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_top_pct = Some(*p);
                    self.margin_top = 0.0;
                } else {
                    self.margin_top = resolve_length(value, parent_font_size);
                    self.margin_top_pct = None;
                }
            },
            "margin-right" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_right = 0.0;
                    self.margin_right_auto = true;
                    self.margin_right_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_right_pct = Some(*p);
                    self.margin_right = 0.0;
                    self.margin_right_auto = false;
                } else {
                    self.margin_right = resolve_length(value, parent_font_size);
                    self.margin_right_auto = false;
                    self.margin_right_pct = None;
                }
            },
            "margin-bottom" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_bottom = 0.0;
                    self.margin_bottom_auto = true;
                    self.margin_bottom_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_bottom_pct = Some(*p);
                    self.margin_bottom = 0.0;
                } else {
                    self.margin_bottom = resolve_length(value, parent_font_size);
                    self.margin_bottom_pct = None;
                }
            },
            "margin-left" => {
                if as_keyword(value) == Some("auto") {
                    self.margin_left = 0.0;
                    self.margin_left_auto = true;
                    self.margin_left_pct = None;
                } else if let CssValue::Percentage(p) = value {
                    self.margin_left_pct = Some(*p);
                    self.margin_left = 0.0;
                    self.margin_left_auto = false;
                } else {
                    self.margin_left = resolve_length(value, parent_font_size);
                    self.margin_left_auto = false;
                    self.margin_left_pct = None;
                }
            },

            // -- Padding ------------------------------------------------
            "padding" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_top_pct = Some(*p);
                    self.padding_right_pct = Some(*p);
                    self.padding_bottom_pct = Some(*p);
                    self.padding_left_pct = Some(*p);
                    self.padding_top = 0.0;
                    self.padding_right = 0.0;
                    self.padding_bottom = 0.0;
                    self.padding_left = 0.0;
                } else {
                    let px = resolve_length(value, parent_font_size);
                    self.padding_top = px;
                    self.padding_right = px;
                    self.padding_bottom = px;
                    self.padding_left = px;
                    self.padding_top_pct = None;
                    self.padding_right_pct = None;
                    self.padding_bottom_pct = None;
                    self.padding_left_pct = None;
                }
            },
            "padding-top" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_top_pct = Some(*p);
                    self.padding_top = 0.0;
                } else {
                    self.padding_top = resolve_length(value, parent_font_size);
                    self.padding_top_pct = None;
                }
            },
            "padding-right" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_right_pct = Some(*p);
                    self.padding_right = 0.0;
                } else {
                    self.padding_right = resolve_length(value, parent_font_size);
                    self.padding_right_pct = None;
                }
            },
            "padding-bottom" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_bottom_pct = Some(*p);
                    self.padding_bottom = 0.0;
                } else {
                    self.padding_bottom = resolve_length(value, parent_font_size);
                    self.padding_bottom_pct = None;
                }
            },
            "padding-left" => {
                if let CssValue::Percentage(p) = value {
                    self.padding_left_pct = Some(*p);
                    self.padding_left = 0.0;
                } else {
                    self.padding_left = resolve_length(value, parent_font_size);
                    self.padding_left_pct = None;
                }
            },

            // -- Border width -------------------------------------------
            "border-width" => {
                let px = resolve_length(value, parent_font_size);
                self.border_top_width = px;
                self.border_right_width = px;
                self.border_bottom_width = px;
                self.border_left_width = px;
            },
            "border-top-width" => {
                self.border_top_width = resolve_length(value, parent_font_size);
            },
            "border-right-width" => {
                self.border_right_width = resolve_length(value, parent_font_size);
            },
            "border-bottom-width" => {
                self.border_bottom_width = resolve_length(value, parent_font_size);
            },
            "border-left-width" => {
                self.border_left_width = resolve_length(value, parent_font_size);
            },

            // -- Border color -------------------------------------------
            "border-color" => {
                let c = resolve_color_or_current(value, self.color);
                if let Some(c) = c {
                    self.border_top_color = c;
                    self.border_right_color = c;
                    self.border_bottom_color = c;
                    self.border_left_color = c;
                }
            },
            "border-top-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.border_top_color = c;
                }
            },
            "border-right-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.border_right_color = c;
                }
            },
            "border-bottom-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.border_bottom_color = c;
                }
            },
            "border-left-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.border_left_color = c;
                }
            },

            // -- Border style -------------------------------------------
            "border-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_top_style = s;
                    self.border_right_style = s;
                    self.border_bottom_style = s;
                    self.border_left_style = s;
                }
            },
            "border-top-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_top_style = s;
                }
            },
            "border-right-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_right_style = s;
                }
            },
            "border-bottom-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_bottom_style = s;
                }
            },
            "border-left-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.border_left_style = s;
                }
            },

            // -- Dimensions ---------------------------------------------
            "width" => {
                self.width = resolve_dimension(value, parent_font_size);
            },
            "height" => {
                self.height = resolve_dimension(value, parent_font_size);
            },
            "max-width" => {
                self.max_width = resolve_dimension(value, parent_font_size);
            },
            "min-width" => {
                self.min_width = resolve_dimension(value, parent_font_size);
            },
            "max-height" => {
                self.max_height = resolve_dimension(value, parent_font_size);
            },
            "min-height" => {
                self.min_height = resolve_dimension(value, parent_font_size);
            },

            // -- Color --------------------------------------------------
            "color" => {
                if let Some(c) = resolve_color(value) {
                    self.color = c;
                }
            },

            // -- Font ---------------------------------------------------
            "font-size" => {
                self.font_size = resolve_font_size(value, parent_font_size);
                // Recompute line-height at the default ratio.
                self.line_height = self.font_size * 1.5;
            },
            "font-weight" => {
                self.font_weight = resolve_font_weight(value);
            },
            "font-style" => {
                if let Some(kw) = as_keyword(value) {
                    self.font_style = match kw {
                        "italic" | "oblique" => FontStyle::Italic,
                        "normal" => FontStyle::Normal,
                        _ => return,
                    };
                }
            },
            "font-family" => {
                if let Some(kw) = as_keyword(value) {
                    self.font_family = match kw {
                        "serif" => FontFamily::Serif,
                        "sans-serif" => FontFamily::SansSerif,
                        "monospace" => FontFamily::Monospace,
                        _ => return,
                    };
                }
            },

            // -- Text ---------------------------------------------------
            "text-align" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_align = match kw {
                        "left" => TextAlign::Left,
                        "center" => TextAlign::Center,
                        "right" => TextAlign::Right,
                        "justify" => TextAlign::Justify,
                        _ => return,
                    };
                }
            },
            "text-decoration" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_decoration = match kw {
                        "none" => TextDecoration::None,
                        "underline" => TextDecoration::Underline,
                        "line-through" => TextDecoration::LineThrough,
                        "overline" => TextDecoration::Overline,
                        _ => return,
                    };
                }
            },
            "text-indent" => {
                self.text_indent = resolve_length(value, parent_font_size);
            },
            "text-transform" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_transform = match kw {
                        "none" => TextTransform::None,
                        "uppercase" => TextTransform::Uppercase,
                        "lowercase" => TextTransform::Lowercase,
                        "capitalize" => TextTransform::Capitalize,
                        _ => return,
                    };
                }
            },
            "line-height" => {
                self.line_height = resolve_line_height(value, self.font_size, parent_font_size);
                // Track unitless factor for correct inheritance (CSS 2.1 §17.21).
                self.line_height_factor = match value {
                    CssValue::Number(n) => Some(*n),
                    CssValue::Keyword(kw) if kw == "normal" => Some(1.5),
                    _ => None,
                };
            },
            "letter-spacing" => {
                if let Some("normal") = as_keyword(value) {
                    self.letter_spacing = 0.0;
                    return;
                }
                self.letter_spacing = resolve_length(value, parent_font_size);
            },
            "word-spacing" => {
                if let Some("normal") = as_keyword(value) {
                    self.word_spacing = 0.0;
                    return;
                }
                self.word_spacing = resolve_length(value, parent_font_size);
            },
            "white-space" => {
                if let Some(kw) = as_keyword(value) {
                    self.white_space = match kw {
                        "normal" => WhiteSpace::Normal,
                        "nowrap" => WhiteSpace::NoWrap,
                        "pre" => WhiteSpace::Pre,
                        "pre-wrap" => WhiteSpace::PreWrap,
                        "pre-line" => WhiteSpace::PreLine,
                        _ => return,
                    };
                }
            },

            // -- Background ---------------------------------------------
            "background-color" | "background" => {
                if let Some(c) = resolve_color(value) {
                    self.background_color = c;
                }
            },

            // -- List ---------------------------------------------------
            "list-style-type" => {
                if let Some(kw) = as_keyword(value) {
                    self.list_style_type = match kw {
                        "none" => ListStyleType::None,
                        "disc" => ListStyleType::Disc,
                        "circle" => ListStyleType::Circle,
                        "square" => ListStyleType::Square,
                        "decimal" => ListStyleType::Decimal,
                        _ => return,
                    };
                }
            },
            "list-style-position" => {
                if let Some(kw) = as_keyword(value) {
                    self.list_style_position = match kw {
                        "outside" => ListStylePosition::Outside,
                        "inside" => ListStylePosition::Inside,
                        _ => return,
                    };
                }
            },

            // -- Table --------------------------------------------------
            "border-collapse" => {
                if let Some(kw) = as_keyword(value) {
                    self.border_collapse = match kw {
                        "separate" => BorderCollapse::Separate,
                        "collapse" => BorderCollapse::Collapse,
                        _ => return,
                    };
                }
            },
            "border-spacing" => {
                self.border_spacing = resolve_length(value, parent_font_size);
            },

            // -- Float --------------------------------------------------
            "float" => {
                if let Some(kw) = as_keyword(value) {
                    self.float = match kw {
                        "none" => Float::None,
                        "left" => Float::Left,
                        "right" => Float::Right,
                        _ => return,
                    };
                }
            },
            "clear" => {
                if let Some(kw) = as_keyword(value) {
                    self.clear = match kw {
                        "none" => Clear::None,
                        "left" => Clear::Left,
                        "right" => Clear::Right,
                        "both" => Clear::Both,
                        _ => return,
                    };
                }
            },

            // -- Overflow -----------------------------------------------
            "overflow" => {
                if let Some(kw) = as_keyword(value) {
                    self.overflow = match kw {
                        "visible" => Overflow::Visible,
                        "hidden" => Overflow::Hidden,
                        "scroll" => Overflow::Scroll,
                        "auto" => Overflow::Auto,
                        _ => return,
                    };
                }
            },

            // -- Replaced element sizing ---------------------------------
            "object-fit" => {
                if let Some(kw) = as_keyword(value) {
                    self.object_fit = match kw {
                        "fill" => ObjectFit::Fill,
                        "contain" => ObjectFit::Contain,
                        "cover" => ObjectFit::Cover,
                        "none" => ObjectFit::None,
                        "scale-down" => ObjectFit::ScaleDown,
                        _ => return,
                    };
                }
            },

            // -- Positioning --------------------------------------------
            "position" => {
                if let Some(kw) = as_keyword(value) {
                    self.position = match kw {
                        "static" => Position::Static,
                        "relative" => Position::Relative,
                        "absolute" => Position::Absolute,
                        "fixed" => Position::Fixed,
                        "sticky" => Position::Sticky,
                        _ => return,
                    };
                }
            },
            "top" => {
                self.top = resolve_dimension(value, parent_font_size);
            },
            "right" => {
                self.right = resolve_dimension(value, parent_font_size);
            },
            "bottom" => {
                self.bottom = resolve_dimension(value, parent_font_size);
            },
            "left" => {
                self.left = resolve_dimension(value, parent_font_size);
            },
            "z-index" => {
                if let CssValue::Number(n) = value {
                    self.z_index = *n as i32;
                    self.z_index_auto = false;
                } else if let Some(kw) = as_keyword(value)
                    && kw == "auto"
                {
                    self.z_index = 0;
                    self.z_index_auto = true;
                }
            },

            // -- Flexbox properties --
            "flex-direction" => {
                if let Some(kw) = as_keyword(value) {
                    self.flex_direction = match kw {
                        "row" => FlexDirection::Row,
                        "row-reverse" => FlexDirection::RowReverse,
                        "column" => FlexDirection::Column,
                        "column-reverse" => FlexDirection::ColumnReverse,
                        _ => return,
                    };
                }
            },
            "flex-wrap" => {
                if let Some(kw) = as_keyword(value) {
                    self.flex_wrap = match kw {
                        "nowrap" => FlexWrap::NoWrap,
                        "wrap" => FlexWrap::Wrap,
                        "wrap-reverse" => FlexWrap::WrapReverse,
                        _ => return,
                    };
                }
            },
            "justify-content" => {
                if let Some(kw) = as_keyword(value) {
                    self.justify_content = match kw {
                        "flex-start" | "start" => JustifyContent::FlexStart,
                        "flex-end" | "end" => JustifyContent::FlexEnd,
                        "center" => JustifyContent::Center,
                        "space-between" => JustifyContent::SpaceBetween,
                        "space-around" => JustifyContent::SpaceAround,
                        "space-evenly" => JustifyContent::SpaceEvenly,
                        _ => return,
                    };
                }
            },
            "align-items" => {
                if let Some(kw) = as_keyword(value) {
                    self.align_items = match kw {
                        "flex-start" | "start" => AlignItems::FlexStart,
                        "flex-end" | "end" => AlignItems::FlexEnd,
                        "center" => AlignItems::Center,
                        "stretch" => AlignItems::Stretch,
                        "baseline" => AlignItems::Baseline,
                        _ => return,
                    };
                }
            },
            "align-content" => {
                if let Some(kw) = as_keyword(value) {
                    self.align_content = match kw {
                        "flex-start" | "start" => AlignContent::FlexStart,
                        "flex-end" | "end" => AlignContent::FlexEnd,
                        "center" => AlignContent::Center,
                        "space-between" => AlignContent::SpaceBetween,
                        "space-around" => AlignContent::SpaceAround,
                        "space-evenly" => AlignContent::SpaceEvenly,
                        "stretch" => AlignContent::Stretch,
                        _ => return,
                    };
                }
            },
            "align-self" => {
                if let Some(kw) = as_keyword(value) {
                    self.align_self = match kw {
                        "auto" => AlignSelf::Auto,
                        "flex-start" | "start" => AlignSelf::FlexStart,
                        "flex-end" | "end" => AlignSelf::FlexEnd,
                        "center" => AlignSelf::Center,
                        "stretch" => AlignSelf::Stretch,
                        "baseline" => AlignSelf::Baseline,
                        _ => return,
                    };
                }
            },
            "order" => {
                if let CssValue::Number(n) = value {
                    self.order = *n as i32;
                }
            },
            "flex-grow" => {
                if let CssValue::Number(n) = value {
                    self.flex_grow = *n;
                }
            },
            "flex-shrink" => {
                if let CssValue::Number(n) = value {
                    self.flex_shrink = *n;
                }
            },
            "flex-basis" => {
                self.flex_basis = resolve_dimension(value, parent_font_size);
            },
            "gap" | "grid-gap" => {
                let v = resolve_length(value, parent_font_size);
                self.gap = v;
                self.column_gap = v;
                self.row_gap = v;
            },
            "column-gap" | "grid-column-gap" => {
                self.column_gap = resolve_length(value, parent_font_size);
            },
            "row-gap" | "grid-row-gap" => {
                self.row_gap = resolve_length(value, parent_font_size);
            },
            "grid-template-columns" => {
                self.grid_template_columns = parse_grid_template(value, parent_font_size);
            },
            "grid-template-rows" => {
                self.grid_template_rows = parse_grid_template(value, parent_font_size);
            },
            "grid-column-start" => {
                if let CssValue::Number(n) = value {
                    self.grid_column_start = Some(*n as i32);
                }
            },
            "grid-column-end" => {
                if let CssValue::Number(n) = value {
                    self.grid_column_end = Some(*n as i32);
                }
            },
            "grid-column" => {
                if let CssValue::Number(n) = value {
                    self.grid_column_start = Some(*n as i32);
                }
            },
            "grid-row-start" => {
                if let CssValue::Number(n) = value {
                    self.grid_row_start = Some(*n as i32);
                }
            },
            "grid-row-end" => {
                if let CssValue::Number(n) = value {
                    self.grid_row_end = Some(*n as i32);
                }
            },
            "grid-row" => {
                if let CssValue::Number(n) = value {
                    self.grid_row_start = Some(*n as i32);
                }
            },

            // -- Visual effects -----------------------------------------
            "border-radius" => {
                self.border_radius = resolve_length(value, parent_font_size);
            },
            "opacity" => {
                if let CssValue::Number(n) = value {
                    self.opacity = n.clamp(0.0, 1.0);
                }
            },
            "box-shadow" => {
                if let Some(kw) = as_keyword(value)
                    && kw == "none"
                {
                    self.box_shadow = Vec::new();
                }
                // Complex box-shadow values are parsed from the raw
                // declaration list in the cascade.
            },
            "text-shadow" => {
                if let Some(kw) = as_keyword(value) {
                    if kw == "none" {
                        self.text_shadow = None;
                    }
                } else if let CssValue::Multiple(vs) = value {
                    // text-shadow: <offset-x> <offset-y> [blur] [color]
                    let mut nums = Vec::new();
                    let mut color = None;
                    for v in vs {
                        match v {
                            CssValue::Length(n, _) | CssValue::Number(n) => nums.push(*n),
                            CssValue::Color(c) => {
                                color = Some(Color::rgba(c.r, c.g, c.b, c.a));
                            },
                            CssValue::Keyword(kw) => {
                                if let Some(c) = crate::css::helpers::named_color(kw) {
                                    color = Some(Color::rgba(c.r, c.g, c.b, c.a));
                                }
                            },
                            _ => {},
                        }
                    }
                    if nums.len() >= 2 {
                        self.text_shadow = Some(TextShadow {
                            offset_x: nums[0],
                            offset_y: nums[1],
                            blur: nums.get(2).copied().unwrap_or(0.0),
                            color: color.unwrap_or(Color::rgba(0, 0, 0, 255)),
                        });
                    }
                }
            },

            // -- Box sizing ---------------------------------------------
            "box-sizing" => {
                if let Some(kw) = as_keyword(value) {
                    self.box_sizing = match kw {
                        "content-box" => BoxSizing::ContentBox,
                        "border-box" => BoxSizing::BorderBox,
                        _ => return,
                    };
                }
            },

            // -- Vertical alignment -------------------------------------
            "vertical-align" => {
                if let Some(kw) = as_keyword(value) {
                    self.vertical_align = match kw {
                        "baseline" => VerticalAlign::Baseline,
                        "top" => VerticalAlign::Top,
                        "middle" => VerticalAlign::Middle,
                        "bottom" => VerticalAlign::Bottom,
                        "text-top" => VerticalAlign::TextTop,
                        "text-bottom" => VerticalAlign::TextBottom,
                        "sub" => VerticalAlign::Sub,
                        "super" => VerticalAlign::Super,
                        _ => return,
                    };
                } else {
                    let len = resolve_length(value, parent_font_size);
                    if len != 0.0 {
                        self.vertical_align = VerticalAlign::Length(len);
                    }
                }
            },

            // -- Background image ---------------------------------------
            "background-image" => {
                if let Some(kw) = as_keyword(value) {
                    if kw == "none" {
                        self.background_image = BackgroundImage::None;
                    }
                } else if let CssValue::Url(ref url) = *value {
                    self.background_image = BackgroundImage::Url(url.clone());
                } else if let CssValue::Gradient(ref grad) = *value {
                    self.background_image = BackgroundImage::Gradient(grad.clone());
                } else if let CssValue::RadialGradient(ref grad) = *value {
                    self.background_image = BackgroundImage::RadialGradient(grad.clone());
                }
            },

            // -- Text overflow ------------------------------------------
            "word-break" => {
                if let Some(kw) = as_keyword(value) {
                    self.word_break = match kw {
                        "break-all" => WordBreak::BreakAll,
                        _ => WordBreak::Normal,
                    };
                }
            },
            "overflow-wrap" | "word-wrap" => {
                if let Some(kw) = as_keyword(value) {
                    self.overflow_wrap = match kw {
                        "break-word" => OverflowWrap::BreakWord,
                        "anywhere" => OverflowWrap::Anywhere,
                        _ => OverflowWrap::Normal,
                    };
                }
            },
            "text-overflow" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_overflow = match kw {
                        "ellipsis" => TextOverflow::Ellipsis,
                        _ => TextOverflow::Clip,
                    };
                }
            },

            // -- Generated content --------------------------------------
            "content" => match value {
                CssValue::String(s) => {
                    self.content = Some(s.clone());
                },
                CssValue::Keyword(kw) if kw == "none" || kw == "normal" => {
                    self.content = None;
                },
                _ => {},
            },

            // -- Outline ------------------------------------------------
            "outline-width" => {
                self.outline_width = resolve_length(value, parent_font_size);
            },
            "outline-color" => {
                if let Some(c) = resolve_color_or_current(value, self.color) {
                    self.outline_color = c;
                }
            },
            "outline-style" => {
                if let Some(s) = resolve_border_style(value) {
                    self.outline_style = s;
                }
            },
            "outline-offset" => {
                self.outline_offset = resolve_length(value, parent_font_size);
            },
            "outline" => {
                // Shorthand: outline: [width] [style] [color]
                if let Some(kw) = as_keyword(value)
                    && kw == "none"
                {
                    self.outline_style = BorderStyle::None;
                    self.outline_width = 0.0;
                    return;
                }
                if let CssValue::Multiple(vs) = value {
                    for v in vs {
                        if let Some(s) = resolve_border_style(v) {
                            self.outline_style = s;
                        } else if let Some(c) = resolve_color(v) {
                            self.outline_color = c;
                        } else {
                            let len = resolve_length(v, parent_font_size);
                            if len > 0.0 {
                                self.outline_width = len;
                            }
                        }
                    }
                }
            },

            // -- Transform origin -------------------------------------------
            "transform-origin" => {
                self.transform_origin = Some(parse_transform_origin(value, parent_font_size));
            },

            // -- Filter ----------------------------------------------------
            "filter" => {
                self.filters = parse_filter(value);
            },

            // -- Counters --------------------------------------------------
            "counter-reset" => {
                self.counter_reset = parse_counter_directive(value);
            },
            "counter-increment" => {
                self.counter_increment = parse_counter_directive(value);
            },

            // -- Will-change -----------------------------------------------
            "will-change" => {
                if let Some(kw) = as_keyword(value) {
                    self.will_change_transform = matches!(kw, "transform" | "opacity" | "filter");
                } else if let CssValue::String(s) = value {
                    self.will_change_transform =
                        s.contains("transform") || s.contains("opacity") || s.contains("filter");
                }
            },

            // -- Tab size ---------------------------------------------------
            "tab-size" => {
                if let CssValue::Number(n) = value {
                    self.tab_size = (*n as u32).max(1);
                }
            },

            // -- Multi-column -----------------------------------------------
            "column-count" => {
                if let CssValue::Number(n) = value {
                    self.column_count = (*n as u32).max(1);
                } else if as_keyword(value) == Some("auto") {
                    self.column_count = 0;
                }
            },
            "column-width" => {
                if as_keyword(value) == Some("auto") {
                    self.column_width = 0.0;
                } else {
                    self.column_width = resolve_length(value, parent_font_size);
                }
            },
            "columns" => {
                // Shorthand: columns: [count] [width]
                if let CssValue::Multiple(vs) = value {
                    for v in vs {
                        if let CssValue::Number(n) = v {
                            self.column_count = (*n as u32).max(1);
                        } else {
                            let len = resolve_length(v, parent_font_size);
                            if len > 0.0 {
                                self.column_width = len;
                            }
                        }
                    }
                } else if let CssValue::Number(n) = value {
                    self.column_count = (*n as u32).max(1);
                }
            },

            // -- Grid extensions -------------------------------------------
            "grid-auto-flow" => {
                if let Some(kw) = as_keyword(value) {
                    self.grid_auto_flow_column = kw.contains("column");
                } else if let CssValue::String(s) = value {
                    self.grid_auto_flow_column = s.contains("column");
                }
            },
            "grid-template-areas" => {
                self.grid_template_areas = parse_grid_template_areas(value);
            },
            "grid-area" => {
                if let Some(kw) = as_keyword(value) {
                    self.grid_area = Some(kw.to_string());
                } else if let CssValue::String(s) = value {
                    self.grid_area = Some(s.clone());
                } else if let CssValue::Number(n) = value {
                    self.grid_row_start = Some(*n as i32);
                }
            },
            "grid-auto-rows" => {
                self.grid_auto_rows = parse_grid_template(value, parent_font_size);
            },
            "grid-auto-columns" => {
                self.grid_auto_columns = parse_grid_template(value, parent_font_size);
            },

            // -- Table layout -----------------------------------------------
            "table-layout" => {
                if let Some(kw) = as_keyword(value) {
                    self.table_layout_fixed = kw == "fixed";
                }
            },

            // -- Transforms -------------------------------------------------
            "transform" => {
                self.transforms = parse_transform(value, parent_font_size);
            },

            // -- Transitions ------------------------------------------------
            "transition" => {
                if let Some(t) = Self::parse_transition(value) {
                    self.transitions = vec![t];
                }
            },

            // -- Animations -------------------------------------------------
            "animation" => {
                if let Some(a) = Self::parse_animation(value) {
                    self.animations = vec![a];
                }
            },
            "animation-name" => {
                if let Some(name) = as_keyword(value).or(match value {
                    CssValue::String(s) => Some(s.as_str()),
                    _ => None,
                }) {
                    if self.animations.is_empty() {
                        self.animations.push(Animation {
                            name: name.to_string(),
                            duration_ms: 0.0,
                            timing: TimingFunction::Ease,
                            delay_ms: 0.0,
                            iteration_count: 1.0,
                            direction: AnimationDirection::Normal,
                            fill_mode: AnimationFillMode::None,
                            play_state: AnimationPlayState::Running,
                        });
                    } else {
                        self.animations[0].name = name.to_string();
                    }
                }
            },
            "animation-duration" => {
                if let CssValue::String(s) = value
                    && let Some(ms) = parse_time(s)
                {
                    if self.animations.is_empty() {
                        self.animations.push(Animation {
                            name: String::new(),
                            duration_ms: ms,
                            timing: TimingFunction::Ease,
                            delay_ms: 0.0,
                            iteration_count: 1.0,
                            direction: AnimationDirection::Normal,
                            fill_mode: AnimationFillMode::None,
                            play_state: AnimationPlayState::Running,
                        });
                    } else {
                        self.animations[0].duration_ms = ms;
                    }
                }
            },
            "animation-timing-function" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(tf) = parse_timing_function(kw)
                {
                    if self.animations.is_empty() {
                        self.animations.push(Animation {
                            name: String::new(),
                            duration_ms: 0.0,
                            timing: tf,
                            delay_ms: 0.0,
                            iteration_count: 1.0,
                            direction: AnimationDirection::Normal,
                            fill_mode: AnimationFillMode::None,
                            play_state: AnimationPlayState::Running,
                        });
                    } else {
                        self.animations[0].timing = tf;
                    }
                }
            },
            "animation-delay" => {
                if let CssValue::String(s) = value
                    && let Some(ms) = parse_time(s)
                    && !self.animations.is_empty()
                {
                    self.animations[0].delay_ms = ms;
                }
            },
            "animation-iteration-count" => {
                if let Some(kw) = as_keyword(value) {
                    let count = parse_iteration_count(kw);
                    if !self.animations.is_empty() {
                        self.animations[0].iteration_count = count;
                    }
                } else if let CssValue::Number(n) = value
                    && !self.animations.is_empty()
                {
                    self.animations[0].iteration_count = *n;
                }
            },
            "animation-direction" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(dir) = parse_animation_direction(kw)
                    && !self.animations.is_empty()
                {
                    self.animations[0].direction = dir;
                }
            },
            "animation-fill-mode" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(fm) = parse_animation_fill_mode(kw)
                    && !self.animations.is_empty()
                {
                    self.animations[0].fill_mode = fm;
                }
            },
            "animation-play-state" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(ps) = parse_animation_play_state(kw)
                    && !self.animations.is_empty()
                {
                    self.animations[0].play_state = ps;
                }
            },

            // Unknown properties are silently ignored (per CSS spec).
            _ => {},
        }
    }

    /// Parse a `transition` shorthand value into a [`Transition`].
    ///
    /// Format: `<property> <duration> [<timing>] [<delay>]`
    /// Example: `all 0.3s ease`, `color 200ms linear 50ms`
    fn parse_transition(value: &CssValue) -> Option<Transition> {
        let raw = match value {
            CssValue::String(s) => s.clone(),
            CssValue::Keyword(s) => s.clone(),
            CssValue::Multiple(vs) => {
                let mut parts = Vec::new();
                for v in vs {
                    match v {
                        CssValue::Keyword(s) | CssValue::String(s) => {
                            parts.push(s.clone());
                        },
                        CssValue::Length(n, _) => parts.push(format!("{n}px")),
                        CssValue::Number(n) => parts.push(format!("{n}")),
                        _ => {},
                    }
                }
                parts.join(" ")
            },
            _ => return None,
        };

        let tokens: Vec<&str> = raw.split_whitespace().collect();
        if tokens.is_empty() {
            return None;
        }

        let property = tokens[0].to_string();
        let duration_ms = tokens.get(1).and_then(|s| parse_time(s)).unwrap_or(0.0);
        let mut timing = TimingFunction::Ease;
        let mut delay_ms = 0.0;

        if let Some(t) = tokens.get(2) {
            if let Some(tf) = parse_timing_function(t) {
                timing = tf;
                if let Some(d) = tokens.get(3) {
                    delay_ms = parse_time(d).unwrap_or(0.0);
                }
            } else {
                // Not a timing function keyword, try as delay.
                delay_ms = parse_time(t).unwrap_or(0.0);
            }
        }

        Some(Transition {
            property,
            duration_ms,
            timing,
            delay_ms,
        })
    }

    /// Parse an `animation` shorthand value into an [`Animation`].
    ///
    /// Format: `<name> <duration> [<timing>] [<delay>] [<iteration-count>]
    ///          [<direction>] [<fill-mode>] [<play-state>]`
    /// Example: `spin 2s linear infinite`
    fn parse_animation(value: &CssValue) -> Option<Animation> {
        let raw = match value {
            CssValue::String(s) => s.clone(),
            CssValue::Keyword(s) => s.clone(),
            CssValue::Multiple(vs) => {
                let mut parts = Vec::new();
                for v in vs {
                    match v {
                        CssValue::Keyword(s) | CssValue::String(s) => {
                            parts.push(s.clone());
                        },
                        CssValue::Length(n, _) => parts.push(format!("{n}px")),
                        CssValue::Number(n) => parts.push(format!("{n}")),
                        _ => {},
                    }
                }
                parts.join(" ")
            },
            _ => return None,
        };

        let tokens: Vec<&str> = raw.split_whitespace().collect();
        if tokens.is_empty() {
            return None;
        }

        let name = tokens[0].to_string();
        let duration_ms = tokens.get(1).and_then(|s| parse_time(s)).unwrap_or(0.0);
        let mut timing = TimingFunction::Ease;
        let mut delay_ms = 0.0;
        let mut iteration_count = 1.0_f32;
        let mut direction = AnimationDirection::Normal;
        let mut fill_mode = AnimationFillMode::None;
        let mut play_state = AnimationPlayState::Running;

        // Parse remaining tokens positionally, classifying each.
        let mut time_idx = 0; // track which time value we're on
        for &tok in tokens.iter().skip(2) {
            if let Some(tf) = parse_timing_function(tok) {
                timing = tf;
            } else if let Some(dir) = parse_animation_direction(tok) {
                direction = dir;
            } else if let Some(fm) = parse_animation_fill_mode(tok) {
                fill_mode = fm;
            } else if let Some(ps) = parse_animation_play_state(tok) {
                play_state = ps;
            } else if tok == "infinite" {
                iteration_count = f32::INFINITY;
            } else if let Ok(n) = tok.parse::<f32>() {
                iteration_count = n;
            } else if let Some(ms) = parse_time(tok)
                && time_idx == 0
            {
                delay_ms = ms;
                time_idx += 1;
            }
        }

        Some(Animation {
            name,
            duration_ms,
            timing,
            delay_ms,
            iteration_count,
            direction,
            fill_mode,
            play_state,
        })
    }

    /// Reset a single property to its CSS initial value.
    fn apply_initial(&mut self, property: &str) {
        let initial = ComputedStyle::default();
        match property {
            "display" => self.display = initial.display,
            "visibility" => self.visibility = initial.visibility,
            "margin" | "margin-top" => self.margin_top = 0.0,
            "margin-right" => self.margin_right = 0.0,
            "margin-bottom" => self.margin_bottom = 0.0,
            "margin-left" => self.margin_left = 0.0,
            "padding" | "padding-top" => self.padding_top = 0.0,
            "padding-right" => self.padding_right = 0.0,
            "padding-bottom" => self.padding_bottom = 0.0,
            "padding-left" => self.padding_left = 0.0,
            "color" => self.color = initial.color,
            "background-color" | "background" => self.background_color = initial.background_color,
            "font-size" => {
                self.font_size = initial.font_size;
                self.line_height = initial.line_height;
            },
            "font-weight" => self.font_weight = initial.font_weight,
            "font-style" => self.font_style = initial.font_style,
            "font-family" => self.font_family = initial.font_family,
            "text-align" => self.text_align = initial.text_align,
            "text-decoration" => self.text_decoration = initial.text_decoration,
            "text-transform" => self.text_transform = initial.text_transform,
            "white-space" => self.white_space = initial.white_space,
            "line-height" => self.line_height = initial.line_height,
            "float" => self.float = initial.float,
            "clear" => self.clear = initial.clear,
            "position" => self.position = initial.position,
            "overflow" => self.overflow = initial.overflow,
            "width" => self.width = initial.width,
            "height" => self.height = initial.height,
            "border-collapse" => self.border_collapse = initial.border_collapse,
            "outline" | "outline-width" => self.outline_width = initial.outline_width,
            "outline-color" => self.outline_color = initial.outline_color,
            "outline-style" => self.outline_style = initial.outline_style,
            "outline-offset" => self.outline_offset = initial.outline_offset,
            "transform" => self.transforms = Vec::new(),
            "transform-origin" => self.transform_origin = None,
            "filter" => self.filters = Vec::new(),
            "counter-reset" => self.counter_reset = Vec::new(),
            "counter-increment" => self.counter_increment = Vec::new(),
            "will-change" => self.will_change_transform = false,
            "tab-size" => self.tab_size = 8,
            "column-count" => self.column_count = 0,
            "column-width" => self.column_width = 0.0,
            "columns" => {
                self.column_count = 0;
                self.column_width = 0.0;
            },
            "grid-auto-flow" => self.grid_auto_flow_column = false,
            "grid-template-areas" => self.grid_template_areas = Vec::new(),
            "grid-area" => self.grid_area = None,
            "object-fit" => self.object_fit = ObjectFit::Fill,
            "grid-auto-rows" => self.grid_auto_rows = Vec::new(),
            "grid-auto-columns" => self.grid_auto_columns = Vec::new(),
            "table-layout" => self.table_layout_fixed = false,
            "animation" => self.animations = Vec::new(),
            _ => {},
        }
    }
}

/// Parse a CSS time value (e.g. `0.3s`, `200ms`) into milliseconds.
fn parse_time(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix("ms") {
        rest.parse::<f32>().ok()
    } else if let Some(rest) = s.strip_suffix('s') {
        rest.parse::<f32>().ok().map(|v| v * 1000.0)
    } else {
        // Try bare number as seconds.
        s.parse::<f32>().ok().map(|v| v * 1000.0)
    }
}

/// Parse a CSS timing-function keyword.
fn parse_timing_function(s: &str) -> Option<TimingFunction> {
    match s {
        "linear" => Some(TimingFunction::Linear),
        "ease" => Some(TimingFunction::Ease),
        "ease-in" => Some(TimingFunction::EaseIn),
        "ease-out" => Some(TimingFunction::EaseOut),
        "ease-in-out" => Some(TimingFunction::EaseInOut),
        _ => None,
    }
}

/// Parse a CSS `animation-iteration-count` value.
fn parse_iteration_count(s: &str) -> f32 {
    if s == "infinite" {
        f32::INFINITY
    } else {
        s.parse::<f32>().unwrap_or(1.0)
    }
}

/// Parse a CSS `animation-direction` keyword.
fn parse_animation_direction(s: &str) -> Option<AnimationDirection> {
    match s {
        "normal" => Some(AnimationDirection::Normal),
        "reverse" => Some(AnimationDirection::Reverse),
        "alternate" => Some(AnimationDirection::Alternate),
        "alternate-reverse" => Some(AnimationDirection::AlternateReverse),
        _ => None,
    }
}

/// Parse a CSS `animation-fill-mode` keyword.
fn parse_animation_fill_mode(s: &str) -> Option<AnimationFillMode> {
    match s {
        "none" => Some(AnimationFillMode::None),
        "forwards" => Some(AnimationFillMode::Forwards),
        "backwards" => Some(AnimationFillMode::Backwards),
        "both" => Some(AnimationFillMode::Both),
        _ => None,
    }
}

/// Parse a CSS `animation-play-state` keyword.
fn parse_animation_play_state(s: &str) -> Option<AnimationPlayState> {
    match s {
        "running" => Some(AnimationPlayState::Running),
        "paused" => Some(AnimationPlayState::Paused),
        _ => None,
    }
}

/// Parse a CSS `transform` value into a list of [`TransformFunction`]s.
///
/// Supports: `translate(x, y)`, `translateX(x)`, `translateY(y)`,
/// `scale(s)`, `scale(sx, sy)`, `scaleX(sx)`, `scaleY(sy)`,
/// `rotate(angle)`, and `none`.
///
/// Multiple functions can be chained: `translate(10px, 0) scale(1.5)`.
fn parse_transform(
    value: &CssValue,
    parent_font_size: f32,
) -> Vec<super::types::TransformFunction> {
    use super::types::TransformFunction;

    let raw = match value {
        CssValue::Keyword(s) if s == "none" => return Vec::new(),
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        _ => return Vec::new(),
    };

    let mut result = Vec::new();
    let mut rest = raw.as_str();

    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        // Find function name and opening paren.
        let Some(paren_pos) = rest.find('(') else {
            break;
        };
        let func_name = rest[..paren_pos].trim();
        let after_paren = &rest[paren_pos + 1..];
        let Some(close_pos) = after_paren.find(')') else {
            break;
        };
        let args_str = after_paren[..close_pos].trim();
        rest = &after_paren[close_pos + 1..];

        // Parse comma-separated arguments.
        let args: Vec<&str> = args_str.split(',').map(|s| s.trim()).collect();

        match func_name {
            "translate" => {
                let x =
                    parse_transform_length(args.first().copied().unwrap_or("0"), parent_font_size);
                let y =
                    parse_transform_length(args.get(1).copied().unwrap_or("0"), parent_font_size);
                result.push(TransformFunction::Translate(x, y));
            },
            "translateX" | "translatex" => {
                let x =
                    parse_transform_length(args.first().copied().unwrap_or("0"), parent_font_size);
                result.push(TransformFunction::Translate(x, 0.0));
            },
            "translateY" | "translatey" => {
                let y =
                    parse_transform_length(args.first().copied().unwrap_or("0"), parent_font_size);
                result.push(TransformFunction::Translate(0.0, y));
            },
            "scale" => {
                let sx = args
                    .first()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                let sy = args
                    .get(1)
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(sx);
                result.push(TransformFunction::Scale(sx, sy));
            },
            "scaleX" | "scalex" => {
                let sx = args
                    .first()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                result.push(TransformFunction::Scale(sx, 1.0));
            },
            "scaleY" | "scaley" => {
                let sy = args
                    .first()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                result.push(TransformFunction::Scale(1.0, sy));
            },
            "rotate" => {
                let angle = parse_angle(args.first().copied().unwrap_or("0"));
                result.push(TransformFunction::Rotate(angle));
            },
            "skew" => {
                let ax = parse_angle(args.first().copied().unwrap_or("0"));
                let ay = parse_angle(args.get(1).copied().unwrap_or("0"));
                result.push(TransformFunction::Skew(ax, ay));
            },
            "skewX" | "skewx" => {
                let ax = parse_angle(args.first().copied().unwrap_or("0"));
                result.push(TransformFunction::Skew(ax, 0.0));
            },
            "skewY" | "skewy" => {
                let ay = parse_angle(args.first().copied().unwrap_or("0"));
                result.push(TransformFunction::Skew(0.0, ay));
            },
            "matrix" => {
                if args.len() >= 6 {
                    let a = args[0].parse::<f32>().unwrap_or(1.0);
                    let b = args[1].parse::<f32>().unwrap_or(0.0);
                    let c = args[2].parse::<f32>().unwrap_or(0.0);
                    let d = args[3].parse::<f32>().unwrap_or(1.0);
                    let e = args[4].parse::<f32>().unwrap_or(0.0);
                    let f = args[5].parse::<f32>().unwrap_or(0.0);
                    result.push(TransformFunction::Matrix(a, b, c, d, e, f));
                }
            },
            _ => {},
        }
    }

    // Helper: use resolve_length for px/em/rem values in transform args.
    fn parse_transform_length(s: &str, parent_font_size: f32) -> f32 {
        let s = s.trim();
        if let Some(px) = s.strip_suffix("px") {
            px.trim().parse::<f32>().unwrap_or(0.0)
        } else if let Some(em) = s.strip_suffix("em") {
            em.trim().parse::<f32>().unwrap_or(0.0) * parent_font_size
        } else if let Some(rem) = s.strip_suffix("rem") {
            rem.trim().parse::<f32>().unwrap_or(0.0) * super::types::ROOT_FONT_SIZE
        } else {
            // Bare number treated as px.
            s.parse::<f32>().unwrap_or(0.0)
        }
    }

    fn parse_angle(s: &str) -> f32 {
        let s = s.trim();
        if let Some(deg) = s.strip_suffix("deg") {
            deg.trim().parse::<f32>().unwrap_or(0.0)
        } else if let Some(rad) = s.strip_suffix("rad") {
            rad.trim().parse::<f32>().unwrap_or(0.0).to_degrees()
        } else if let Some(turn) = s.strip_suffix("turn") {
            turn.trim().parse::<f32>().unwrap_or(0.0) * 360.0
        } else {
            // Bare number treated as degrees.
            s.parse::<f32>().unwrap_or(0.0)
        }
    }

    result
}

/// Parse a CSS `transform-origin` value.
fn parse_transform_origin(
    value: &CssValue,
    parent_font_size: f32,
) -> super::types::TransformOrigin {
    use super::types::TransformOrigin;

    let raw = match value {
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        CssValue::Length(_, _) => {
            let px = resolve_length(value, parent_font_size);
            return TransformOrigin {
                x: px,
                y: 0.0,
                x_pct: None,
                y_pct: None,
            };
        },
        CssValue::Percentage(p) => {
            return TransformOrigin {
                x: 0.0,
                y: 0.0,
                x_pct: Some(*p / 100.0),
                y_pct: Some(0.5),
            };
        },
        _ => {
            return TransformOrigin {
                x: 0.0,
                y: 0.0,
                x_pct: Some(0.5),
                y_pct: Some(0.5),
            };
        },
    };

    let parts: Vec<&str> = raw.split_whitespace().collect();
    let mut x_pct: Option<f32> = None;
    let y_pct: Option<f32>;
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;

    let resolve_part = |s: &str| -> (f32, Option<f32>) {
        match s {
            "left" => (0.0, Some(0.0)),
            "center" => (0.0, Some(0.5)),
            "right" => (0.0, Some(1.0)),
            "top" => (0.0, Some(0.0)),
            "bottom" => (0.0, Some(1.0)),
            _ => {
                if let Some(pct) = s.strip_suffix('%')
                    && let Ok(v) = pct.trim().parse::<f32>()
                {
                    return (0.0, Some(v / 100.0));
                }
                if let Some(px) = s.strip_suffix("px")
                    && let Ok(v) = px.trim().parse::<f32>()
                {
                    return (v, None);
                }
                if let Ok(v) = s.parse::<f32>() {
                    return (v, None);
                }
                (0.0, Some(0.5))
            },
        }
    };

    if let Some(p0) = parts.first() {
        let (px, pct) = resolve_part(p0);
        x = px;
        x_pct = pct;
    }
    if let Some(p1) = parts.get(1) {
        let (px, pct) = resolve_part(p1);
        y = px;
        y_pct = pct;
    } else {
        // Default Y is center.
        y_pct = Some(0.5);
    }

    TransformOrigin { x, y, x_pct, y_pct }
}

/// Parse a CSS `filter` value into a list of [`FilterFunction`]s.
fn parse_filter(value: &CssValue) -> Vec<super::types::FilterFunction> {
    use super::types::FilterFunction;

    let raw = match value {
        CssValue::Keyword(s) if s == "none" => return Vec::new(),
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        _ => return Vec::new(),
    };

    let mut result = Vec::new();
    let mut rest = raw.as_str();

    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let Some(paren_pos) = rest.find('(') else {
            break;
        };
        let func_name = rest[..paren_pos].trim();
        let after_paren = &rest[paren_pos + 1..];
        let Some(close_pos) = after_paren.find(')') else {
            break;
        };
        let arg_str = after_paren[..close_pos].trim();
        rest = &after_paren[close_pos + 1..];

        let val = if let Some(pct) = arg_str.strip_suffix('%') {
            pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0
        } else if let Some(px) = arg_str.strip_suffix("px") {
            px.trim().parse::<f32>().unwrap_or(0.0)
        } else if let Some(deg) = arg_str.strip_suffix("deg") {
            deg.trim().parse::<f32>().unwrap_or(0.0)
        } else if let Some(rad) = arg_str.strip_suffix("rad") {
            rad.trim().parse::<f32>().unwrap_or(0.0).to_degrees()
        } else {
            arg_str.parse::<f32>().unwrap_or(0.0)
        };

        let f = match func_name {
            "blur" => FilterFunction::Blur(val),
            "brightness" => FilterFunction::Brightness(val),
            "contrast" => FilterFunction::Contrast(val),
            "grayscale" => FilterFunction::Grayscale(val),
            "invert" => FilterFunction::Invert(val),
            "opacity" => FilterFunction::Opacity(val),
            "saturate" => FilterFunction::Saturate(val),
            "sepia" => FilterFunction::Sepia(val),
            "hue-rotate" => FilterFunction::HueRotate(val),
            _ => continue,
        };
        result.push(f);
    }

    result
}

/// Parse a CSS `counter-reset` or `counter-increment` value.
fn parse_counter_directive(value: &CssValue) -> Vec<(String, i32)> {
    let raw = match value {
        CssValue::Keyword(s) if s == "none" => return Vec::new(),
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        _ => return Vec::new(),
    };

    let mut result = Vec::new();
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let name = tokens[i].to_string();
        let value = if i + 1 < tokens.len() {
            if let Ok(v) = tokens[i + 1].parse::<i32>() {
                i += 1;
                v
            } else {
                0
            }
        } else {
            0
        };
        result.push((name, value));
        i += 1;
    }
    result
}

/// Resolve counters in a `content` property value.
///
/// Replaces `counter(name)` references with the current counter value.
#[allow(dead_code)]
fn resolve_content_counters(
    content: &str,
    _counters: &std::collections::HashMap<String, i32>,
) -> String {
    // Placeholder implementation -- returns content unchanged.
    content.to_string()
}

/// Parse `grid-template-areas` value.
fn parse_grid_template_areas(value: &CssValue) -> Vec<Vec<String>> {
    let raw = match value {
        CssValue::Keyword(s) if s == "none" => return Vec::new(),
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        _ => return Vec::new(),
    };

    let mut areas = Vec::new();
    // Each quoted row is separated by whitespace outside quotes.
    // For simplicity, split on '"' and take every other segment.
    let parts: Vec<&str> = raw.split('"').collect();
    for (i, part) in parts.iter().enumerate() {
        if i % 2 == 1 {
            // Inside quotes: split by whitespace.
            let row: Vec<String> = part.split_whitespace().map(String::from).collect();
            if !row.is_empty() {
                areas.push(row);
            }
        }
    }
    areas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parser::{CssColor, LengthUnit};
    use crate::css::values::types::ROOT_FONT_SIZE;
    use crate::css::values::{
        BackgroundImage, BorderStyle, Dimension, Display, FontWeight, GradientDirection,
        GradientStop, LinearGradient,
    };

    #[test]
    fn apply_keyword_display() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("display", &CssValue::Keyword("block".into()), 16.0);
        assert_eq!(s.display, Display::Block);
    }

    #[test]
    fn apply_px_margin() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("margin", &CssValue::Length(10.0, LengthUnit::Px), 16.0);
        assert!((s.margin_top - 10.0).abs() < f32::EPSILON);
        assert!((s.margin_right - 10.0).abs() < f32::EPSILON);
        assert!((s.margin_bottom - 10.0).abs() < f32::EPSILON);
        assert!((s.margin_left - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_em_padding() {
        let mut s = ComputedStyle::default();
        // 1.5em with parent font-size 20px = 30px.
        s.apply_declaration("padding-top", &CssValue::Length(1.5, LengthUnit::Em), 20.0);
        assert!((s.padding_top - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_color_keyword() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("color", &CssValue::Keyword("red".into()), 16.0);
        assert_eq!(s.color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn apply_color_value() {
        let mut s = ComputedStyle::default();
        let c = CssColor {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        };
        s.apply_declaration("color", &CssValue::Color(c), 16.0);
        assert_eq!(s.color, Color::rgb(10, 20, 30));
    }

    #[test]
    fn apply_font_size_updates_line_height() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-size", &CssValue::Length(20.0, LengthUnit::Px), 16.0);
        assert!((s.font_size - 20.0).abs() < f32::EPSILON);
        // Line height should be recomputed: 20 * 1.5 = 30.
        assert!((s.line_height - 30.0).abs() < 0.01);
    }

    #[test]
    fn apply_font_weight_bold_keyword() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-weight", &CssValue::Keyword("bold".into()), 16.0);
        assert_eq!(s.font_weight, FontWeight::Bold);
    }

    #[test]
    fn apply_font_weight_bold_number() {
        // The CSS parser normalises "bold" to Number(700.0).
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-weight", &CssValue::Number(700.0), 16.0);
        assert_eq!(s.font_weight, FontWeight::Bold);
    }

    #[test]
    fn apply_font_weight_normal_number() {
        let mut s = ComputedStyle::default();
        s.font_weight = FontWeight::Bold;
        s.apply_declaration("font-weight", &CssValue::Number(400.0), 16.0);
        assert_eq!(s.font_weight, FontWeight::Normal);
    }

    #[test]
    fn apply_dimension_percent() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("width", &CssValue::Percentage(50.0), 16.0);
        assert_eq!(s.width, Dimension::Percent(50.0));
    }

    #[test]
    fn apply_dimension_auto() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("width", &CssValue::Keyword("auto".into()), 16.0);
        assert_eq!(s.width, Dimension::Auto);
    }

    #[test]
    fn apply_border_shorthand() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("border-style", &CssValue::Keyword("solid".into()), 16.0);
        assert_eq!(s.border_top_style, BorderStyle::Solid);
        assert_eq!(s.border_right_style, BorderStyle::Solid);
        assert_eq!(s.border_bottom_style, BorderStyle::Solid);
        assert_eq!(s.border_left_style, BorderStyle::Solid);
    }

    #[test]
    fn apply_background_color() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("background-color", &CssValue::Keyword("white".into()), 16.0);
        assert_eq!(s.background_color, Color::WHITE);
    }

    #[test]
    fn apply_unknown_property_is_noop() {
        let mut s = ComputedStyle::default();
        let before = s.clone();
        s.apply_declaration("unknown-prop", &CssValue::Keyword("something".into()), 16.0);
        // Nothing should have changed.
        assert_eq!(s.display, before.display);
        assert_eq!(s.color, before.color);
    }

    #[test]
    fn resolve_font_size_keywords() {
        let mut s = ComputedStyle::default();
        let parent = ROOT_FONT_SIZE;
        s.apply_declaration("font-size", &CssValue::Keyword("small".into()), parent);
        let expected_small = ROOT_FONT_SIZE * 0.8125;
        assert!((s.font_size - expected_small).abs() < f32::EPSILON);

        s.apply_declaration("font-size", &CssValue::Keyword("larger".into()), parent);
        let expected_larger = parent * 1.2;
        assert!((s.font_size - expected_larger).abs() < 0.01);
    }

    #[test]
    fn resolve_line_height_number_multiplier() {
        let mut s = ComputedStyle::default();
        s.font_size = 20.0;
        s.apply_declaration("line-height", &CssValue::Number(1.5), 16.0);
        // 1.5 * 20.0 = 30.0
        assert!((s.line_height - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_margin_auto_vertical_flags() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("margin-top", &CssValue::Keyword("auto".into()), 16.0);
        assert!(s.margin_top_auto, "margin-top: auto should set flag");
        assert_eq!(s.margin_top, 0.0);

        s.apply_declaration("margin-bottom", &CssValue::Keyword("auto".into()), 16.0);
        assert!(s.margin_bottom_auto, "margin-bottom: auto should set flag");
        assert_eq!(s.margin_bottom, 0.0);
    }

    #[test]
    fn test_margin_shorthand_preserves_auto() {
        use crate::css::parser::LengthUnit;

        let mut s = ComputedStyle::default();
        // margin: 0 auto => top/bottom=0, left/right=auto
        // The shorthand is expanded by the parser, but here we test
        // individual property application after expansion.
        s.apply_declaration("margin-top", &CssValue::Length(0.0, LengthUnit::Px), 16.0);
        s.apply_declaration("margin-right", &CssValue::Keyword("auto".into()), 16.0);
        s.apply_declaration(
            "margin-bottom",
            &CssValue::Length(0.0, LengthUnit::Px),
            16.0,
        );
        s.apply_declaration("margin-left", &CssValue::Keyword("auto".into()), 16.0);

        assert!(s.margin_left_auto);
        assert!(s.margin_right_auto);
        assert!(!s.margin_top_auto);
        assert!(!s.margin_bottom_auto);
    }

    #[test]
    fn test_currentcolor_resolves_to_element_color() {
        let mut s = ComputedStyle::default();
        s.color = Color::rgb(255, 0, 0);
        s.apply_declaration(
            "border-top-color",
            &CssValue::Keyword("currentcolor".into()),
            16.0,
        );
        assert_eq!(
            s.border_top_color,
            Color::rgb(255, 0, 0),
            "currentcolor should resolve to element's color",
        );
    }

    #[test]
    fn text_shadow_parsed() {
        let mut s = ComputedStyle::default();
        let value = CssValue::Multiple(vec![
            CssValue::Length(2.0, LengthUnit::Px),
            CssValue::Length(3.0, LengthUnit::Px),
            CssValue::Length(1.0, LengthUnit::Px),
            CssValue::Color(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            }),
        ]);
        s.apply_declaration("text-shadow", &value, 16.0);
        let ts = s.text_shadow.expect("should parse text-shadow");
        assert_eq!(ts.offset_x, 2.0);
        assert_eq!(ts.offset_y, 3.0);
        assert_eq!(ts.blur, 1.0);
        assert_eq!(ts.color, Color::rgba(0, 0, 0, 255));
    }

    #[test]
    fn text_shadow_none() {
        let mut s = ComputedStyle::default();
        s.text_shadow = Some(TextShadow {
            offset_x: 1.0,
            offset_y: 1.0,
            blur: 0.0,
            color: Color::rgb(0, 0, 0),
        });
        let value = CssValue::Keyword("none".into());
        s.apply_declaration("text-shadow", &value, 16.0);
        assert!(s.text_shadow.is_none());
    }

    #[test]
    fn gradient_background_image_applied() {
        let mut s = ComputedStyle::default();
        let grad = LinearGradient {
            direction: GradientDirection::ToRight,
            stops: vec![
                GradientStop {
                    color: Color::rgb(255, 0, 0),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgb(0, 0, 255),
                    position: 1.0,
                },
            ],
            repeating: false,
        };
        let value = CssValue::Gradient(grad.clone());
        s.apply_declaration("background-image", &value, 16.0);
        assert_eq!(s.background_image, BackgroundImage::Gradient(grad));
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// apply_declaration with unknown property is a no-op.
            #[test]
            fn apply_unknown_property_noop(
                prop_name in "[a-z\\-]{1,20}",
            ) {
                // Filter out known properties.
                if matches!(
                    prop_name.as_str(),
                    "display" | "color" | "margin" | "padding"
                    | "width" | "height" | "font-size"
                    | "background-color" | "background"
                    | "border-width" | "border-style"
                    | "border-color" | "overflow" | "position"
                    | "float" | "clear" | "visibility"
                    | "text-align" | "text-decoration"
                    | "text-indent" | "text-transform"
                    | "white-space" | "line-height"
                    | "letter-spacing" | "word-spacing"
                    | "font-weight" | "font-style" | "font-family"
                    | "list-style-type" | "list-style-position"
                    | "border-collapse" | "border-spacing"
                    | "z-index" | "flex-direction" | "flex-wrap"
                    | "justify-content" | "align-items" | "align-content"
                    | "align-self" | "order"
                    | "flex-grow" | "flex-shrink" | "flex-basis"
                    | "gap" | "row-gap" | "column-gap"
                    | "grid-template-columns" | "grid-template-rows"
                    | "grid-column" | "grid-column-start" | "grid-column-end"
                    | "grid-row" | "grid-row-start" | "grid-row-end"
                    | "grid-gap" | "grid-row-gap" | "grid-column-gap"
                    | "grid-auto-rows" | "grid-auto-columns"
                    | "top" | "right" | "bottom" | "left"
                    | "max-width" | "min-width"
                    | "max-height" | "min-height"
                    | "transform-origin" | "filter"
                    | "counter-reset" | "counter-increment"
                    | "will-change" | "tab-size" | "column-count" | "column-width"
                    | "columns" | "grid-auto-flow" | "grid-template-areas"
                    | "grid-area" | "table-layout"
                ) {
                    return Ok(());
                }
                let mut s = ComputedStyle::default();
                let before_color = s.color;
                s.apply_declaration(
                    &prop_name,
                    &CssValue::Keyword("x".into()),
                    16.0,
                );
                prop_assert_eq!(s.color, before_color);
            }
        }
    }

    // -- Transition parsing tests ----------------------------------------

    #[test]
    fn parse_transition_all_ease() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "transition",
            &CssValue::String("all 0.3s ease".into()),
            16.0,
        );
        assert_eq!(s.transitions.len(), 1);
        let t = &s.transitions[0];
        assert_eq!(t.property, "all");
        assert!((t.duration_ms - 300.0).abs() < 0.1);
        assert_eq!(t.timing, TimingFunction::Ease);
        assert!((t.delay_ms).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_transition_ms_with_delay() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "transition",
            &CssValue::String("color 200ms linear 50ms".into()),
            16.0,
        );
        assert_eq!(s.transitions.len(), 1);
        let t = &s.transitions[0];
        assert_eq!(t.property, "color");
        assert!((t.duration_ms - 200.0).abs() < 0.1);
        assert_eq!(t.timing, TimingFunction::Linear);
        assert!((t.delay_ms - 50.0).abs() < 0.1);
    }

    #[test]
    fn parse_transition_ease_in_out() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "transition",
            &CssValue::String("opacity 1s ease-in-out".into()),
            16.0,
        );
        assert_eq!(s.transitions.len(), 1);
        let t = &s.transitions[0];
        assert_eq!(t.property, "opacity");
        assert!((t.duration_ms - 1000.0).abs() < 0.1);
        assert_eq!(t.timing, TimingFunction::EaseInOut);
    }

    #[test]
    fn parse_time_seconds() {
        assert!((parse_time("0.3s").unwrap() - 300.0).abs() < 0.1);
        assert!((parse_time("1s").unwrap() - 1000.0).abs() < 0.1);
    }

    #[test]
    fn parse_time_milliseconds() {
        assert!((parse_time("200ms").unwrap() - 200.0).abs() < 0.1);
        assert!((parse_time("50ms").unwrap() - 50.0).abs() < 0.1);
    }
}
