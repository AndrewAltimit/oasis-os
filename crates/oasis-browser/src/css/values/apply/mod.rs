//! `ComputedStyle::apply_declaration` — the per-property dispatch.
//!
//! Takes a parsed CSS declaration and writes the resolved value into
//! the `ComputedStyle`. Helpers that grew large enough to warrant
//! their own files live in submodules:
//!
//! - [`container`] — `container`, `container-name`, `will-change` parsing
//! - [`initial`] — `apply_initial` reset, transition/animation, logical props
//! - [`parsers`] — free-function value parsers (`transform`, `clip-path`, ...)

use oasis_types::backend::Color;

use super::computed::ComputedStyle;
use super::resolve::{
    as_keyword, parse_grid_template, resolve_border_style, resolve_color_or_current_with_scheme,
    resolve_color_with_scheme, resolve_dimension, resolve_font_size, resolve_font_weight,
    resolve_length, resolve_line_height,
};
use super::types::{
    AlignContent, AlignItems, AlignSelf, Animation, AnimationDirection, AnimationFillMode,
    AnimationPlayState, Appearance, BackfaceVisibility, BackgroundBox, BackgroundImage,
    BackgroundRepeat, BackgroundSize, BorderCollapse, BorderRadius, BorderStyle, BoxSizing, Clear,
    ColorScheme, ContainerType, ContentVisibility, Cursor, Display, FieldSizing, FlexDirection,
    FlexWrap, Float, FontKerning, FontStretch, FontStyle, FontVariant, Hyphens, ImageRendering,
    Isolation, JustifyContent, ListStylePosition, ListStyleType, ObjectFit, Overflow, OverflowWrap,
    PointerEvents, Position, Resize, ScrollBehavior, ScrollSnapAlign, ScrollSnapStop, TextAlign,
    TextAlignLast, TextDecorationLine, TextDecorationStyle, TextDirection, TextJustify,
    TextOverflow, TextRendering, TextShadow, TextTransform, TextUnderlinePosition, TextWrap,
    TimingFunction, TouchAction, TransformStyle, UserSelect, VerticalAlign, Visibility, WhiteSpace,
    WordBreak,
};
use crate::css::parser::CssValue;

mod container;
mod initial;
mod parsers;

#[cfg(test)]
mod tests;

use container::{parse_container_name_list, parse_container_shorthand, will_change_promotes};
use parsers::{
    parse_animation_direction, parse_animation_fill_mode, parse_animation_play_state,
    parse_background_box, parse_blend_mode, parse_clip_path, parse_counter_directive, parse_filter,
    parse_font_family_value, parse_grid_template_areas, parse_iteration_count, parse_justify_self,
    parse_overscroll, parse_perspective_origin, parse_time, parse_timing_function, parse_transform,
    parse_transform_origin, string_or_keyword,
};

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

        // Resolve inline-axis logical properties to physical names
        // based on the element's computed `direction`. Block-axis and
        // size logical properties are already rewritten at parse time
        // (direction-independent), but inline-axis properties need
        // the element's direction to decide left vs. right.
        if let Some(physical) = self.resolve_inline_logical(property) {
            self.apply_declaration(physical, value, parent_font_size);
            return;
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
                        "inline-flex" => Display::InlineFlex,
                        "grid" => Display::Grid,
                        "inline-grid" => Display::InlineGrid,
                        "none" => Display::None,
                        // -webkit-box is used with -webkit-line-clamp
                        "-webkit-box" => Display::Block,
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
                let c =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme());
                if let Some(c) = c {
                    self.border_top_color = c;
                    self.border_right_color = c;
                    self.border_bottom_color = c;
                    self.border_left_color = c;
                }
            },
            "border-top-color" => {
                if let Some(c) =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme())
                {
                    self.border_top_color = c;
                }
            },
            "border-right-color" => {
                if let Some(c) =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme())
                {
                    self.border_right_color = c;
                }
            },
            "border-bottom-color" => {
                if let Some(c) =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme())
                {
                    self.border_bottom_color = c;
                }
            },
            "border-left-color" => {
                if let Some(c) =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme())
                {
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
                if let Some(c) = resolve_color_with_scheme(value, self.is_dark_scheme()) {
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
                self.font_weight = resolve_font_weight(value, self.font_weight);
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
                self.font_family = parse_font_family_value(value);
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
                    self.text_decoration.line = match kw {
                        "none" => TextDecorationLine::NONE,
                        "underline" => TextDecorationLine::UNDERLINE,
                        "line-through" => TextDecorationLine::LINE_THROUGH,
                        "overline" => TextDecorationLine::OVERLINE,
                        _ => return,
                    };
                } else if let CssValue::Multiple(vs) = value {
                    // Multi-value shorthand: e.g. "underline line-through wavy red"
                    // Reset line first, then accumulate with |=.
                    self.text_decoration.line = TextDecorationLine::NONE;
                    for v in vs {
                        if let Some(kw) = as_keyword(v) {
                            match kw {
                                "none" => {
                                    // CSS spec: `none` is exclusive — clears all
                                    // line decorations and ignores further keywords.
                                    self.text_decoration.line = TextDecorationLine::NONE;
                                    break;
                                },
                                "underline" => {
                                    self.text_decoration.line |= TextDecorationLine::UNDERLINE;
                                },
                                "line-through" => {
                                    self.text_decoration.line |= TextDecorationLine::LINE_THROUGH;
                                },
                                "overline" => {
                                    self.text_decoration.line |= TextDecorationLine::OVERLINE;
                                },
                                "solid" => {
                                    self.text_decoration.style = TextDecorationStyle::Solid;
                                },
                                "dashed" => {
                                    self.text_decoration.style = TextDecorationStyle::Dashed;
                                },
                                "dotted" => {
                                    self.text_decoration.style = TextDecorationStyle::Dotted;
                                },
                                "double" => {
                                    self.text_decoration.style = TextDecorationStyle::Double;
                                },
                                "wavy" => {
                                    self.text_decoration.style = TextDecorationStyle::Wavy;
                                },
                                _ => {
                                    if let Some(c) =
                                        resolve_color_with_scheme(v, self.is_dark_scheme())
                                    {
                                        self.text_decoration.color = Some(c);
                                    }
                                },
                            }
                        } else if let Some(c) = resolve_color_with_scheme(v, self.is_dark_scheme())
                        {
                            self.text_decoration.color = Some(c);
                        }
                    }
                }
            },
            "text-decoration-line" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_decoration.line = match kw {
                        "none" => TextDecorationLine::NONE,
                        "underline" => TextDecorationLine::UNDERLINE,
                        "line-through" => TextDecorationLine::LINE_THROUGH,
                        "overline" => TextDecorationLine::OVERLINE,
                        _ => return,
                    };
                } else if let CssValue::Multiple(vs) = value {
                    // Multi-value longhand: e.g. "underline line-through".
                    // Reset to NONE then accumulate with |=, mirroring the
                    // shorthand parser.
                    self.text_decoration.line = TextDecorationLine::NONE;
                    for v in vs {
                        if let Some(kw) = as_keyword(v) {
                            match kw {
                                "none" => {
                                    self.text_decoration.line = TextDecorationLine::NONE;
                                    break;
                                },
                                "underline" => {
                                    self.text_decoration.line |= TextDecorationLine::UNDERLINE;
                                },
                                "line-through" => {
                                    self.text_decoration.line |= TextDecorationLine::LINE_THROUGH;
                                },
                                "overline" => {
                                    self.text_decoration.line |= TextDecorationLine::OVERLINE;
                                },
                                _ => {},
                            }
                        }
                    }
                }
            },
            "text-decoration-color" => {
                if let Some(c) =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme())
                {
                    self.text_decoration.color = Some(c);
                }
            },
            "text-decoration-style" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_decoration.style = match kw {
                        "solid" => TextDecorationStyle::Solid,
                        "dashed" => TextDecorationStyle::Dashed,
                        "dotted" => TextDecorationStyle::Dotted,
                        "double" => TextDecorationStyle::Double,
                        "wavy" => TextDecorationStyle::Wavy,
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
            "text-wrap" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_wrap = match kw {
                        "wrap" => TextWrap::Wrap,
                        "nowrap" => TextWrap::Nowrap,
                        "balance" => TextWrap::Balance,
                        "pretty" => TextWrap::Pretty,
                        "stable" => TextWrap::Stable,
                        _ => return,
                    };
                }
            },

            // -- Background ---------------------------------------------
            "background-color" | "background" => {
                if let Some(c) = resolve_color_with_scheme(value, self.is_dark_scheme()) {
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
                        "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
                        "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
                        "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
                        "lower-roman" => ListStyleType::LowerRoman,
                        "upper-roman" => ListStyleType::UpperRoman,
                        other => ListStyleType::Custom(other.to_string()),
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
                    let v = match kw {
                        "visible" => Overflow::Visible,
                        "hidden" => Overflow::Hidden,
                        "scroll" => Overflow::Scroll,
                        "auto" => Overflow::Auto,
                        _ => return,
                    };
                    self.overflow = v;
                    self.overflow_x = v;
                    self.overflow_y = v;
                }
            },
            "overflow-x" => {
                if let Some(kw) = as_keyword(value) {
                    self.overflow_x = match kw {
                        "visible" => Overflow::Visible,
                        "hidden" => Overflow::Hidden,
                        "scroll" => Overflow::Scroll,
                        "auto" => Overflow::Auto,
                        _ => return,
                    };
                    // Promote to main overflow so scroll container detection works.
                    self.overflow = Self::more_restrictive(self.overflow_x, self.overflow_y);
                }
            },
            "overflow-y" => {
                if let Some(kw) = as_keyword(value) {
                    self.overflow_y = match kw {
                        "visible" => Overflow::Visible,
                        "hidden" => Overflow::Hidden,
                        "scroll" => Overflow::Scroll,
                        "auto" => Overflow::Auto,
                        _ => return,
                    };
                    self.overflow = Self::more_restrictive(self.overflow_x, self.overflow_y);
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

            "object-position" => {
                self.object_position = Self::resolve_obj_position(value, parent_font_size);
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
                let r = resolve_length(value, parent_font_size);
                self.border_radius = BorderRadius::uniform(r);
            },
            "border-top-left-radius" => {
                self.border_radius.top_left = resolve_length(value, parent_font_size);
            },
            "border-top-right-radius" => {
                self.border_radius.top_right = resolve_length(value, parent_font_size);
            },
            "border-bottom-right-radius" => {
                self.border_radius.bottom_right = resolve_length(value, parent_font_size);
            },
            "border-bottom-left-radius" => {
                self.border_radius.bottom_left = resolve_length(value, parent_font_size);
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
            //
            // CSS `background-image` accepts a comma-separated list of
            // layers; the engine only supports single-layer semantics,
            // so a `CssValue::Multiple` is reduced to its first item.
            // Without this arm, `background-image: url(a), url(b)`
            // would fall through every branch and leave the property
            // unchanged — effectively dropping the whole declaration.
            "background-image" => {
                // `background-image` is a comma-separated layer stack; the
                // first value is the TOPMOST layer and the last is the
                // bottom. The engine only stores one layer today, so we
                // have to pick one. Naively taking the first value breaks
                // the widespread "transparent-gradient-over-URL" fallback
                // pattern (Wikipedia's `.sprite{background-image:
                // linear-gradient(transparent,transparent),url(...svg)}`)
                // where the gradient is a no-op overlay and the URL is
                // the actual sprite. Walk the layers and prefer a `url(...)`
                // whenever one exists; otherwise fall back to the first
                // non-`none` layer.
                let chosen: Option<&CssValue> = match value {
                    CssValue::Multiple(vs) => vs
                        .iter()
                        .find(|v| matches!(v, CssValue::Url(_)))
                        .or_else(|| {
                            vs.iter().find(|v| {
                                matches!(v, CssValue::Gradient(_) | CssValue::RadialGradient(_))
                            })
                        })
                        .or_else(|| vs.first()),
                    _ => Some(value),
                };
                let Some(first) = chosen else { return };
                if let Some(kw) = as_keyword(first) {
                    if kw == "none" {
                        self.background_image = BackgroundImage::None;
                    }
                } else if let CssValue::Url(ref url) = *first {
                    self.background_image = BackgroundImage::Url(url.clone());
                } else if let CssValue::Gradient(ref grad) = *first {
                    self.background_image = BackgroundImage::Gradient(grad.clone());
                } else if let CssValue::RadialGradient(ref grad) = *first {
                    self.background_image = BackgroundImage::RadialGradient(grad.clone());
                }
            },

            // -- Background size/position/repeat ---------------------------
            "background-size" => {
                if let Some(kw) = as_keyword(value) {
                    self.background_size = match kw {
                        "cover" => BackgroundSize::Cover,
                        "contain" => BackgroundSize::Contain,
                        "auto" => BackgroundSize::Auto,
                        _ => return,
                    };
                } else if let CssValue::Multiple(vs) = value {
                    // Two-value form: width height
                    let Some(first) = vs.first() else { return };
                    let w = Self::resolve_bg_size_component(first, parent_font_size);
                    let h = vs
                        .get(1)
                        .map(|v| Self::resolve_bg_size_component(v, parent_font_size))
                        .unwrap_or(None);
                    self.background_size = BackgroundSize::Explicit(w, h);
                } else {
                    let w = Self::resolve_bg_size_component(value, parent_font_size);
                    self.background_size = BackgroundSize::Explicit(w, None);
                }
            },
            "background-position" => {
                self.background_position = Self::resolve_bg_position(value, parent_font_size);
            },
            "background-repeat" => {
                if let Some(kw) = as_keyword(value) {
                    self.background_repeat = match kw {
                        "repeat" => BackgroundRepeat::Repeat,
                        "no-repeat" => BackgroundRepeat::NoRepeat,
                        "repeat-x" => BackgroundRepeat::RepeatX,
                        "repeat-y" => BackgroundRepeat::RepeatY,
                        _ => return,
                    };
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

            // -- Cursor -------------------------------------------------
            "cursor" => {
                if let Some(kw) = as_keyword(value) {
                    self.cursor = match kw {
                        "auto" => Cursor::Auto,
                        "default" => Cursor::Default,
                        "pointer" => Cursor::Pointer,
                        "text" => Cursor::Text,
                        "move" => Cursor::Move,
                        "not-allowed" => Cursor::NotAllowed,
                        "crosshair" => Cursor::Crosshair,
                        "wait" => Cursor::Wait,
                        "help" => Cursor::Help,
                        "grab" => Cursor::Grab,
                        "grabbing" => Cursor::Grabbing,
                        "col-resize" => Cursor::ColResize,
                        "row-resize" => Cursor::RowResize,
                        "n-resize" => Cursor::NResize,
                        "e-resize" => Cursor::EResize,
                        "s-resize" => Cursor::SResize,
                        "w-resize" => Cursor::WResize,
                        "ne-resize" => Cursor::NeResize,
                        "nw-resize" => Cursor::NwResize,
                        "se-resize" => Cursor::SeResize,
                        "sw-resize" => Cursor::SwResize,
                        "ew-resize" => Cursor::EwResize,
                        "ns-resize" => Cursor::NsResize,
                        "nesw-resize" => Cursor::NeswResize,
                        "nwse-resize" => Cursor::NwseResize,
                        "zoom-in" => Cursor::ZoomIn,
                        "zoom-out" => Cursor::ZoomOut,
                        "none" => Cursor::None,
                        _ => return,
                    };
                }
            },

            // -- Pointer events -----------------------------------------
            "pointer-events" => {
                if let Some(kw) = as_keyword(value) {
                    self.pointer_events = match kw {
                        "auto" => PointerEvents::Auto,
                        "none" => PointerEvents::None,
                        _ => return,
                    };
                }
            },

            // -- User select --------------------------------------------
            "user-select" | "-webkit-user-select" | "-moz-user-select" => {
                if let Some(kw) = as_keyword(value) {
                    self.user_select = match kw {
                        "auto" => UserSelect::Auto,
                        "none" => UserSelect::None,
                        "text" => UserSelect::Text,
                        "all" => UserSelect::All,
                        _ => return,
                    };
                }
            },

            // -- Aspect ratio -------------------------------------------
            "aspect-ratio" => {
                if let Some(kw) = as_keyword(value) {
                    if kw == "auto" {
                        self.aspect_ratio = None;
                    }
                } else if let CssValue::Number(n) = value {
                    if *n > 0.0 {
                        self.aspect_ratio = Some(*n);
                    }
                } else if let CssValue::Multiple(vs) = value {
                    // Parse "width / height" form.
                    if let (Some(CssValue::Number(w)), Some(CssValue::Number(h))) =
                        (vs.first(), vs.last())
                        && *w > 0.0
                        && *h > 0.0
                    {
                        self.aspect_ratio = Some(*w / *h);
                    }
                }
            },

            // -- Text underline offset ----------------------------------
            "text-underline-offset" => {
                if let Some("auto") = as_keyword(value) {
                    self.text_underline_offset = 0.0;
                } else {
                    self.text_underline_offset = resolve_length(value, parent_font_size);
                }
            },

            // -- Direction (RTL/LTR) ------------------------------------
            "direction" => {
                if let Some(kw) = as_keyword(value) {
                    self.direction = match kw {
                        "ltr" => TextDirection::Ltr,
                        "rtl" => TextDirection::Rtl,
                        _ => return,
                    };
                }
            },

            // -- Place shorthands (grid/flex) ----------------------------
            "place-items" => {
                if let Some(kw) = as_keyword(value) {
                    let ai = match kw {
                        "start" | "flex-start" => AlignItems::FlexStart,
                        "end" | "flex-end" => AlignItems::FlexEnd,
                        "center" => AlignItems::Center,
                        "stretch" => AlignItems::Stretch,
                        "baseline" => AlignItems::Baseline,
                        _ => return,
                    };
                    self.align_items = ai;
                    // justify-items maps to justify-content for our model.
                    self.justify_content = match kw {
                        "start" | "flex-start" => JustifyContent::FlexStart,
                        "end" | "flex-end" => JustifyContent::FlexEnd,
                        "center" => JustifyContent::Center,
                        _ => self.justify_content,
                    };
                }
            },
            "place-content" => {
                if let Some(kw) = as_keyword(value) {
                    let ac = match kw {
                        "start" | "flex-start" => AlignContent::FlexStart,
                        "end" | "flex-end" => AlignContent::FlexEnd,
                        "center" => AlignContent::Center,
                        "stretch" => AlignContent::Stretch,
                        "space-between" => AlignContent::SpaceBetween,
                        "space-around" => AlignContent::SpaceAround,
                        "space-evenly" => AlignContent::SpaceEvenly,
                        _ => return,
                    };
                    self.align_content = ac;
                    self.justify_content = match kw {
                        "start" | "flex-start" => JustifyContent::FlexStart,
                        "end" | "flex-end" => JustifyContent::FlexEnd,
                        "center" => JustifyContent::Center,
                        "space-between" => JustifyContent::SpaceBetween,
                        "space-around" => JustifyContent::SpaceAround,
                        "space-evenly" => JustifyContent::SpaceEvenly,
                        _ => self.justify_content,
                    };
                }
            },

            // -- Appearance ------------------------------------------------
            "appearance" | "-webkit-appearance" | "-moz-appearance" => {
                if let Some(kw) = as_keyword(value) {
                    self.appearance = match kw {
                        "none" => Appearance::None,
                        "auto" => Appearance::Auto,
                        _ => return,
                    };
                }
            },

            // -- Line clamp -----------------------------------------------
            "-webkit-line-clamp" | "line-clamp" => {
                if let CssValue::Number(n) = value {
                    self.line_clamp = *n as u32;
                } else if let Some("none") = as_keyword(value) {
                    self.line_clamp = 0;
                }
            },
            // Also recognize -webkit-box-orient for line-clamp compatibility
            "-webkit-box-orient" => {
                // Accepted but no separate field needed — line_clamp is sufficient.
            },

            // -- Accent color ---------------------------------------------
            "accent-color" => {
                if let Some("auto") = as_keyword(value) {
                    self.accent_color = None;
                } else if let Some(c) = resolve_color_with_scheme(value, self.is_dark_scheme()) {
                    self.accent_color = Some(c);
                }
            },

            // -- Caret color ----------------------------------------------
            "caret-color" => {
                if let Some("auto") = as_keyword(value) {
                    self.caret_color = None;
                } else if let Some(c) =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme())
                {
                    self.caret_color = Some(c);
                }
            },

            // -- Color scheme ---------------------------------------------
            "color-scheme" => {
                if let Some(kw) = as_keyword(value) {
                    self.color_scheme = match kw {
                        "normal" => ColorScheme::Normal,
                        "light" => ColorScheme::Light,
                        "dark" => ColorScheme::Dark,
                        _ => return,
                    };
                } else if let CssValue::Multiple(vs) = value {
                    // "light dark" form
                    let has_light = vs.iter().any(|v| as_keyword(v) == Some("light"));
                    let has_dark = vs.iter().any(|v| as_keyword(v) == Some("dark"));
                    if has_light && has_dark {
                        self.color_scheme = ColorScheme::LightDark;
                    } else if has_dark {
                        self.color_scheme = ColorScheme::Dark;
                    } else if has_light {
                        self.color_scheme = ColorScheme::Light;
                    }
                }
            },

            // -- Isolation ------------------------------------------------
            "isolation" => {
                if let Some(kw) = as_keyword(value) {
                    self.isolation = match kw {
                        "auto" => Isolation::Auto,
                        "isolate" => Isolation::Isolate,
                        _ => return,
                    };
                }
            },

            // -- Resize ---------------------------------------------------
            "resize" => {
                if let Some(kw) = as_keyword(value) {
                    self.resize = match kw {
                        "none" => Resize::None,
                        "both" => Resize::Both,
                        "horizontal" => Resize::Horizontal,
                        "vertical" => Resize::Vertical,
                        _ => return,
                    };
                }
            },

            // -- Touch action ---------------------------------------------
            "touch-action" => {
                if let Some(kw) = as_keyword(value) {
                    self.touch_action = match kw {
                        "auto" => TouchAction::Auto,
                        "none" => TouchAction::None,
                        "manipulation" => TouchAction::Manipulation,
                        "pan-x" => TouchAction::PanX,
                        "pan-y" => TouchAction::PanY,
                        _ => return,
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
                if let Some(c) =
                    resolve_color_or_current_with_scheme(value, self.color, self.is_dark_scheme())
                {
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
                        } else if let Some(c) = resolve_color_with_scheme(v, self.is_dark_scheme())
                        {
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
            //
            // The spec lets authors list any animatable property; we
            // only care about the subset that benefits from layer
            // promotion: `transform`, `opacity`, `filter`,
            // `scroll-position`, `contents`. Other listed properties
            // (e.g. `top`, `left`) are ignored — they're hints, not
            // guarantees, and we can't paint them faster anyway.
            "will-change" => {
                self.will_change_promotes_layer = will_change_promotes(value);
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

            // -- Scrolling / snapping ----------------------------------
            "scroll-behavior" => {
                if let Some(kw) = as_keyword(value) {
                    self.scroll_behavior = match kw {
                        "smooth" => ScrollBehavior::Smooth,
                        _ => ScrollBehavior::Auto,
                    };
                }
            },
            "scroll-snap-type" => {
                self.scroll_snap_type = string_or_keyword(value);
            },
            "scroll-snap-align" => {
                if let Some(kw) = as_keyword(value) {
                    self.scroll_snap_align = match kw {
                        "start" => ScrollSnapAlign::Start,
                        "end" => ScrollSnapAlign::End,
                        "center" => ScrollSnapAlign::Center,
                        _ => ScrollSnapAlign::None,
                    };
                }
            },
            "scroll-snap-stop" => {
                if let Some(kw) = as_keyword(value) {
                    self.scroll_snap_stop = match kw {
                        "always" => ScrollSnapStop::Always,
                        _ => ScrollSnapStop::Normal,
                    };
                }
            },
            "overscroll-behavior" => {
                if let Some(b) = parse_overscroll(value) {
                    self.overscroll_behavior_x = b;
                    self.overscroll_behavior_y = b;
                }
            },
            "overscroll-behavior-x" => {
                if let Some(b) = parse_overscroll(value) {
                    self.overscroll_behavior_x = b;
                }
            },
            "overscroll-behavior-y" => {
                if let Some(b) = parse_overscroll(value) {
                    self.overscroll_behavior_y = b;
                }
            },

            // -- Compositing ------------------------------------------
            "mix-blend-mode" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(m) = parse_blend_mode(kw)
                {
                    self.mix_blend_mode = m;
                }
            },
            "background-blend-mode" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(m) = parse_blend_mode(kw)
                {
                    self.background_blend_mode = m;
                }
            },
            "backdrop-filter" | "-webkit-backdrop-filter" => {
                if as_keyword(value) == Some("none") {
                    self.backdrop_filters.clear();
                } else {
                    self.backdrop_filters = parse_filter(value);
                }
            },
            "background-clip" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(b) = parse_background_box(kw)
                {
                    self.background_clip = b;
                }
            },
            "background-origin" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(b) = parse_background_box(kw)
                    && b != BackgroundBox::Text
                {
                    self.background_origin = b;
                }
            },
            "image-rendering" => {
                if let Some(kw) = as_keyword(value) {
                    self.image_rendering = match kw {
                        "crisp-edges" | "-webkit-optimize-contrast" => ImageRendering::CrispEdges,
                        "pixelated" => ImageRendering::Pixelated,
                        _ => ImageRendering::Auto,
                    };
                }
            },

            // -- Mask properties (compositor overhaul PR6) ------------
            // The 8 `mask-*` longhands. Values are parsed and stored on
            // the computed style today; the destination-in composite
            // path lives in a follow-up (the filter-chain readback
            // pipeline is the foundation).
            "mask-image" | "-webkit-mask-image" => {
                // Multi-layer mask lists reduce to their first layer
                // (matches the single-layer collapse documented on
                // `MaskComposite`). Without this, a page using
                // `mask-image: url(a), url(b)` would fall through all
                // arms and end up with `BackgroundImage::None`,
                // silently removing the mask entirely.
                let first = match value {
                    CssValue::Multiple(vs) => {
                        debug_assert!(!vs.is_empty(), "empty Multiple in mask-image");
                        vs.first().unwrap_or(value)
                    },
                    _ => value,
                };
                if as_keyword(first) == Some("none") {
                    self.mask_image = BackgroundImage::None;
                } else if let CssValue::Url(ref url) = *first {
                    self.mask_image = BackgroundImage::Url(url.clone());
                } else if let CssValue::Gradient(ref grad) = *first {
                    self.mask_image = BackgroundImage::Gradient(grad.clone());
                } else if let CssValue::RadialGradient(ref grad) = *first {
                    self.mask_image = BackgroundImage::RadialGradient(grad.clone());
                }
            },
            "mask-mode" | "-webkit-mask-mode" => {
                if let Some(kw) = as_keyword(value) {
                    self.mask_mode = match kw {
                        "alpha" => crate::css::values::types::MaskMode::Alpha,
                        "luminance" => crate::css::values::types::MaskMode::Luminance,
                        _ => crate::css::values::types::MaskMode::MatchSource,
                    };
                }
            },
            "mask-composite" | "-webkit-mask-composite" => {
                if let Some(kw) = as_keyword(value) {
                    self.mask_composite = match kw {
                        "subtract" => crate::css::values::types::MaskComposite::Subtract,
                        "intersect" => crate::css::values::types::MaskComposite::Intersect,
                        "exclude" => crate::css::values::types::MaskComposite::Exclude,
                        _ => crate::css::values::types::MaskComposite::Add,
                    };
                }
            },
            "mask-clip" | "-webkit-mask-clip" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(b) = parse_background_box(kw)
                {
                    self.mask_clip = b;
                }
            },
            "mask-origin" | "-webkit-mask-origin" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(b) = parse_background_box(kw)
                    && b != BackgroundBox::Text
                {
                    self.mask_origin = b;
                }
            },
            "mask-position" | "-webkit-mask-position" => {
                self.mask_position = Self::resolve_bg_position(value, parent_font_size);
            },
            "mask-size" | "-webkit-mask-size" => {
                if let Some(kw) = as_keyword(value) {
                    self.mask_size = match kw {
                        "cover" => BackgroundSize::Cover,
                        "contain" => BackgroundSize::Contain,
                        _ => BackgroundSize::Auto,
                    };
                }
            },
            "mask-repeat" | "-webkit-mask-repeat" => {
                if let Some(kw) = as_keyword(value) {
                    self.mask_repeat = match kw {
                        "repeat" => BackgroundRepeat::Repeat,
                        "no-repeat" => BackgroundRepeat::NoRepeat,
                        "repeat-x" => BackgroundRepeat::RepeatX,
                        "repeat-y" => BackgroundRepeat::RepeatY,
                        _ => return,
                    };
                }
            },
            "content-visibility" => {
                if let Some(kw) = as_keyword(value) {
                    self.content_visibility = match kw {
                        "auto" => ContentVisibility::Auto,
                        "hidden" => ContentVisibility::Hidden,
                        _ => ContentVisibility::Visible,
                    };
                }
            },

            // -- Font extensions --------------------------------------
            "font-variant" => {
                if let Some(kw) = as_keyword(value) {
                    self.font_variant = match kw {
                        "small-caps" => FontVariant::SmallCaps,
                        _ => FontVariant::Normal,
                    };
                }
            },
            "font-stretch" => {
                if let Some(kw) = as_keyword(value) {
                    self.font_stretch = match kw {
                        "ultra-condensed" => FontStretch::UltraCondensed,
                        "extra-condensed" => FontStretch::ExtraCondensed,
                        "condensed" => FontStretch::Condensed,
                        "semi-condensed" => FontStretch::SemiCondensed,
                        "semi-expanded" => FontStretch::SemiExpanded,
                        "expanded" => FontStretch::Expanded,
                        "extra-expanded" => FontStretch::ExtraExpanded,
                        "ultra-expanded" => FontStretch::UltraExpanded,
                        _ => FontStretch::Normal,
                    };
                }
            },
            "font-kerning" => {
                if let Some(kw) = as_keyword(value) {
                    self.font_kerning = match kw {
                        "none" => FontKerning::None,
                        "normal" => FontKerning::Normal,
                        _ => FontKerning::Auto,
                    };
                }
            },
            "font-feature-settings" => {
                self.font_feature_settings = string_or_keyword(value);
            },

            // -- Text extensions --------------------------------------
            "hyphens" => {
                if let Some(kw) = as_keyword(value) {
                    self.hyphens = match kw {
                        "none" => Hyphens::None,
                        "auto" => Hyphens::Auto,
                        _ => Hyphens::Manual,
                    };
                }
            },
            "text-align-last" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_align_last = match kw {
                        "left" => TextAlignLast::Left,
                        "right" => TextAlignLast::Right,
                        "center" => TextAlignLast::Center,
                        "justify" => TextAlignLast::Justify,
                        "start" => TextAlignLast::Start,
                        "end" => TextAlignLast::End,
                        _ => TextAlignLast::Auto,
                    };
                }
            },
            "text-justify" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_justify = match kw {
                        "inter-word" => TextJustify::InterWord,
                        "inter-character" | "distribute" => TextJustify::InterCharacter,
                        "none" => TextJustify::None,
                        _ => TextJustify::Auto,
                    };
                }
            },
            "text-underline-position" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_underline_position = match kw {
                        "under" => TextUnderlinePosition::Under,
                        "left" => TextUnderlinePosition::Left,
                        "right" => TextUnderlinePosition::Right,
                        _ => TextUnderlinePosition::Auto,
                    };
                }
            },
            "text-decoration-thickness" => {
                if as_keyword(value) == Some("auto") || as_keyword(value) == Some("from-font") {
                    self.text_decoration_thickness = None;
                } else if matches!(
                    value,
                    CssValue::Length(..) | CssValue::Number(_) | CssValue::Calc(_)
                ) {
                    self.text_decoration_thickness = Some(resolve_length(value, parent_font_size));
                }
            },
            "text-rendering" => {
                if let Some(kw) = as_keyword(value) {
                    self.text_rendering = match kw {
                        "optimizespeed" | "optimize-speed" => TextRendering::OptimizeSpeed,
                        "optimizelegibility" | "optimize-legibility" => {
                            TextRendering::OptimizeLegibility
                        },
                        "geometricprecision" | "geometric-precision" => {
                            TextRendering::GeometricPrecision
                        },
                        _ => TextRendering::Auto,
                    };
                }
            },

            // -- 3D / clipping ----------------------------------------
            "clip-path" | "-webkit-clip-path" => {
                if as_keyword(value) == Some("none") {
                    self.clip_path = None;
                } else {
                    self.clip_path = parse_clip_path(value, parent_font_size);
                }
            },
            "perspective" => {
                if as_keyword(value) == Some("none") {
                    self.perspective = None;
                } else if matches!(
                    value,
                    CssValue::Length(..) | CssValue::Number(_) | CssValue::Calc(_)
                ) {
                    self.perspective = Some(resolve_length(value, parent_font_size));
                }
            },
            "perspective-origin" => {
                self.perspective_origin = Some(parse_perspective_origin(value, parent_font_size));
            },
            "backface-visibility" => {
                if let Some(kw) = as_keyword(value) {
                    self.backface_visibility = match kw {
                        "hidden" => BackfaceVisibility::Hidden,
                        _ => BackfaceVisibility::Visible,
                    };
                }
            },
            "transform-style" => {
                if let Some(kw) = as_keyword(value) {
                    self.transform_style = match kw {
                        "preserve-3d" => TransformStyle::Preserve3d,
                        _ => TransformStyle::Flat,
                    };
                }
            },

            // -- Grid alignment extensions ----------------------------
            "justify-self" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(j) = parse_justify_self(kw)
                {
                    self.justify_self = j;
                }
            },
            "justify-items" => {
                if let Some(kw) = as_keyword(value)
                    && let Some(j) = parse_justify_self(kw)
                {
                    self.justify_items = j;
                }
            },

            // -- Container queries ------------------------------------
            "container-type" => {
                if let Some(kw) = as_keyword(value) {
                    self.container_type = match kw {
                        "normal" => ContainerType::Normal,
                        "inline-size" => ContainerType::InlineSize,
                        "size" => ContainerType::Size,
                        _ => return,
                    };
                }
            },
            "container-name" => {
                self.container_name = parse_container_name_list(value);
            },
            "field-sizing" => {
                if let Some(kw) = as_keyword(value) {
                    self.field_sizing = match kw {
                        "content" => FieldSizing::Content,
                        "fixed" => FieldSizing::Fixed,
                        _ => return,
                    };
                }
            },
            "container" => {
                // `container: <name> [/ <type>]` shorthand.
                let (names, ty) = parse_container_shorthand(value);
                self.container_name = names;
                if let Some(t) = ty {
                    self.container_type = t;
                }
            },

            // -- Inset shorthand --------------------------------------
            "inset" => {
                let dim = resolve_dimension(value, parent_font_size);
                self.top = dim;
                self.right = dim;
                self.bottom = dim;
                self.left = dim;
            },

            // Unknown properties are silently ignored (per CSS spec).
            _ => {},
        }
    }
}
