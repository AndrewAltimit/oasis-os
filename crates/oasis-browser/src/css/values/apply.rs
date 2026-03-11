//! `ComputedStyle::apply_declaration` and `apply_initial` implementations.

use oasis_types::backend::Color;

use super::computed::ComputedStyle;
use super::resolve::{
    as_keyword, parse_grid_template, resolve_border_style, resolve_color, resolve_color_or_current,
    resolve_dimension, resolve_font_size, resolve_font_weight, resolve_length, resolve_line_height,
};
use super::types::{
    AlignItems, BackgroundImage, BorderCollapse, BoxSizing, Clear, Display, FlexDirection,
    FlexWrap, Float, FontFamily, FontStyle, JustifyContent, ListStylePosition, ListStyleType,
    Overflow, OverflowWrap, Position, TextAlign, TextDecoration, TextOverflow, TextShadow,
    TextTransform, VerticalAlign, Visibility, WhiteSpace, WordBreak,
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
                    self.box_shadow = None;
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
                        _ => return,
                    };
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

            // Unknown properties are silently ignored (per CSS spec).
            _ => {},
        }
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
            _ => {},
        }
    }
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
                    | "justify-content" | "align-items"
                    | "flex-grow" | "flex-shrink" | "flex-basis"
                    | "gap" | "row-gap" | "column-gap"
                    | "grid-template-columns" | "grid-template-rows"
                    | "grid-column" | "grid-column-start" | "grid-column-end"
                    | "grid-row" | "grid-row-start" | "grid-row-end"
                    | "grid-gap" | "grid-row-gap" | "grid-column-gap"
                    | "top" | "right" | "bottom" | "left"
                    | "max-width" | "min-width"
                    | "max-height" | "min-height"
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
}
