//! `ComputedStyle::apply_declaration` and `apply_initial` implementations.

use oasis_types::backend::Color;

use super::computed::ComputedStyle;
use super::resolve::{
    as_keyword, parse_grid_template, resolve_border_style, resolve_color, resolve_color_or_current,
    resolve_dimension, resolve_font_size, resolve_font_weight, resolve_length, resolve_line_height,
};
use super::types::{
    AlignContent, AlignItems, AlignSelf, Animation, AnimationDirection, AnimationFillMode,
    AnimationPlayState, Appearance, BackfaceVisibility, BackgroundBox, BackgroundImage,
    BackgroundPosition, BackgroundRepeat, BackgroundSize, BlendMode, BorderCollapse, BorderRadius,
    BorderStyle, BoxSizing, Clear, ColorScheme, ContainerType, ContentVisibility, Cursor, Display,
    FieldSizing, FlexDirection, FlexWrap, Float, FontFamily, FontKerning, FontStretch, FontStyle,
    FontVariant, Hyphens, ImageRendering, Isolation, JustifyContent, JustifySelf,
    ListStylePosition, ListStyleType, ObjectFit, ObjectPosition, Overflow, OverflowWrap,
    OverscrollBehavior, PointerEvents, Position, Resize, ScrollBehavior, ScrollSnapAlign,
    ScrollSnapStop, TextAlign, TextAlignLast, TextDecorationLine, TextDecorationStyle,
    TextDirection, TextJustify, TextOverflow, TextRendering, TextShadow, TextTransform,
    TextUnderlinePosition, TextWrap, TimingFunction, TouchAction, TransformStyle, Transition,
    UserSelect, VerticalAlign, Visibility, WhiteSpace, WordBreak,
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
                                    if let Some(c) = resolve_color(v) {
                                        self.text_decoration.color = Some(c);
                                    }
                                },
                            }
                        } else if let Some(c) = resolve_color(v) {
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
                if let Some(c) = resolve_color_or_current(value, self.color) {
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
                let first = match value {
                    CssValue::Multiple(vs) => {
                        debug_assert!(!vs.is_empty(), "empty Multiple in background-image");
                        vs.first().unwrap_or(value)
                    },
                    _ => value,
                };
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
                } else if let Some(c) = resolve_color(value) {
                    self.accent_color = Some(c);
                }
            },

            // -- Caret color ----------------------------------------------
            "caret-color" => {
                if let Some("auto") = as_keyword(value) {
                    self.caret_color = None;
                } else if let Some(c) = resolve_color_or_current(value, self.color) {
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

/// True if a `will-change` value names any property that benefits
/// from layer promotion in our pipeline.
///
/// Recognised hints are `transform`, `opacity`, `filter`,
/// `scroll-position`, and `contents`. All other identifiers (and
/// `auto`) leave the flag false.
pub(crate) fn will_change_promotes(value: &CssValue) -> bool {
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
pub(crate) fn parse_container_name_list(value: &CssValue) -> Vec<String> {
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
pub(crate) fn parse_container_shorthand(value: &CssValue) -> (Vec<String>, Option<ContainerType>) {
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

impl ComputedStyle {
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
            "text-wrap" => self.text_wrap = initial.text_wrap,
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
            "will-change" => self.will_change_promotes_layer = false,
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
            "background-size" => self.background_size = BackgroundSize::Auto,
            "background-position" => self.background_position = BackgroundPosition::default(),
            "background-repeat" => self.background_repeat = BackgroundRepeat::Repeat,
            "text-decoration-color" => self.text_decoration.color = None,
            "text-decoration-style" => self.text_decoration.style = TextDecorationStyle::Solid,
            "border-top-left-radius" => self.border_radius.top_left = 0.0,
            "border-top-right-radius" => self.border_radius.top_right = 0.0,
            "border-bottom-right-radius" => self.border_radius.bottom_right = 0.0,
            "border-bottom-left-radius" => self.border_radius.bottom_left = 0.0,
            "overflow-x" => self.overflow_x = Overflow::Visible,
            "overflow-y" => self.overflow_y = Overflow::Visible,
            "cursor" => self.cursor = Cursor::Auto,
            "pointer-events" => self.pointer_events = PointerEvents::Auto,
            "user-select" => self.user_select = UserSelect::Auto,
            "aspect-ratio" => self.aspect_ratio = None,
            "text-underline-offset" => self.text_underline_offset = 0.0,
            "direction" => self.direction = TextDirection::Ltr,
            "object-position" => self.object_position = ObjectPosition::default(),
            "appearance" | "-webkit-appearance" | "-moz-appearance" => {
                self.appearance = Appearance::Auto;
            },
            "-webkit-line-clamp" | "line-clamp" => self.line_clamp = 0,
            "accent-color" => self.accent_color = None,
            "caret-color" => self.caret_color = None,
            "color-scheme" => self.color_scheme = ColorScheme::Normal,
            "isolation" => self.isolation = Isolation::Auto,
            "resize" => self.resize = Resize::None,
            "touch-action" => self.touch_action = TouchAction::Auto,
            _ => {},
        }
    }

    /// Resolve a single background-size component (width or height).
    fn resolve_bg_size_component(value: &CssValue, parent_font_size: f32) -> Option<f32> {
        match value {
            CssValue::Keyword(kw) if kw == "auto" => None,
            CssValue::Percentage(p) => Some(-*p), // negative = percentage
            _ => Some(resolve_length(value, parent_font_size)),
        }
    }

    /// Resolve a background-position value.
    fn resolve_bg_position(value: &CssValue, parent_font_size: f32) -> BackgroundPosition {
        fn keyword_to_frac(kw: &str) -> Option<(f32, bool)> {
            match kw {
                "left" | "top" => Some((0.0, false)),
                "center" => Some((0.5, false)),
                "right" | "bottom" => Some((1.0, false)),
                _ => None,
            }
        }

        match value {
            CssValue::Keyword(kw) => {
                if let Some((frac, _)) = keyword_to_frac(kw) {
                    // Single keyword: horizontal position, vertical defaults to center.
                    match kw.as_str() {
                        "top" | "bottom" => BackgroundPosition {
                            x: 0.5,
                            y: frac,
                            x_is_px: false,
                            y_is_px: false,
                        },
                        _ => BackgroundPosition {
                            x: frac,
                            y: 0.5,
                            x_is_px: false,
                            y_is_px: false,
                        },
                    }
                } else {
                    BackgroundPosition::default()
                }
            },
            CssValue::Percentage(p) => BackgroundPosition {
                x: *p / 100.0,
                y: 0.5,
                x_is_px: false,
                y_is_px: false,
            },
            CssValue::Multiple(vs) if vs.len() >= 2 => {
                let (x, x_is_px) = match &vs[0] {
                    CssValue::Keyword(kw) => keyword_to_frac(kw).unwrap_or((0.0, false)),
                    CssValue::Percentage(p) => (*p / 100.0, false),
                    other => (resolve_length(other, parent_font_size), true),
                };
                let (y, y_is_px) = match &vs[1] {
                    CssValue::Keyword(kw) => keyword_to_frac(kw).unwrap_or((0.0, false)),
                    CssValue::Percentage(p) => (*p / 100.0, false),
                    other => (resolve_length(other, parent_font_size), true),
                };
                BackgroundPosition {
                    x,
                    y,
                    x_is_px,
                    y_is_px,
                }
            },
            other => {
                let px = resolve_length(other, parent_font_size);
                BackgroundPosition {
                    x: px,
                    y: 0.5,
                    x_is_px: true,
                    y_is_px: false,
                }
            },
        }
    }

    /// Resolve an `object-position` value (same logic as background-position).
    fn resolve_obj_position(value: &CssValue, parent_font_size: f32) -> ObjectPosition {
        let bp = Self::resolve_bg_position(value, parent_font_size);
        ObjectPosition {
            x: bp.x,
            y: bp.y,
            x_is_px: bp.x_is_px,
            y_is_px: bp.y_is_px,
        }
    }

    /// Return the more restrictive of two overflow values.
    ///
    /// Used to promote `self.overflow` when `overflow-x`/`overflow-y` are set
    /// independently, so scroll container detection (which checks `self.overflow`)
    /// still works.
    fn more_restrictive(a: Overflow, b: Overflow) -> Overflow {
        fn rank(o: Overflow) -> u8 {
            match o {
                Overflow::Visible => 0,
                Overflow::Auto => 1,
                Overflow::Scroll => 2,
                Overflow::Hidden => 3,
            }
        }
        if rank(a) >= rank(b) { a } else { b }
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

fn string_or_keyword(value: &CssValue) -> Option<String> {
    match value {
        CssValue::String(s) => Some(s.clone()),
        CssValue::Keyword(k) => Some(k.clone()),
        _ => None,
    }
}

fn parse_overscroll(value: &CssValue) -> Option<OverscrollBehavior> {
    match as_keyword(value)? {
        "contain" => Some(OverscrollBehavior::Contain),
        "none" => Some(OverscrollBehavior::None),
        "auto" => Some(OverscrollBehavior::Auto),
        _ => None,
    }
}

fn parse_blend_mode(s: &str) -> Option<BlendMode> {
    Some(match s {
        "normal" => BlendMode::Normal,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        _ => return None,
    })
}

fn parse_background_box(s: &str) -> Option<BackgroundBox> {
    Some(match s {
        "border-box" => BackgroundBox::BorderBox,
        "padding-box" => BackgroundBox::PaddingBox,
        "content-box" => BackgroundBox::ContentBox,
        "text" => BackgroundBox::Text,
        _ => return None,
    })
}

fn parse_justify_self(s: &str) -> Option<JustifySelf> {
    Some(match s {
        "auto" => JustifySelf::Auto,
        "start" => JustifySelf::Start,
        "end" => JustifySelf::End,
        "center" => JustifySelf::Center,
        "stretch" => JustifySelf::Stretch,
        "flex-start" => JustifySelf::FlexStart,
        "flex-end" => JustifySelf::FlexEnd,
        _ => return None,
    })
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
            "translate3d" => {
                let x =
                    parse_transform_length(args.first().copied().unwrap_or("0"), parent_font_size);
                let y =
                    parse_transform_length(args.get(1).copied().unwrap_or("0"), parent_font_size);
                let z =
                    parse_transform_length(args.get(2).copied().unwrap_or("0"), parent_font_size);
                result.push(TransformFunction::Translate3d(x, y, z));
            },
            "translateZ" | "translatez" => {
                let z =
                    parse_transform_length(args.first().copied().unwrap_or("0"), parent_font_size);
                result.push(TransformFunction::TranslateZ(z));
            },
            "scale3d" => {
                let sx = args
                    .first()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                let sy = args
                    .get(1)
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                let sz = args
                    .get(2)
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                result.push(TransformFunction::Scale3d(sx, sy, sz));
            },
            "scaleZ" | "scalez" => {
                let sz = args
                    .first()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(1.0);
                result.push(TransformFunction::ScaleZ(sz));
            },
            "rotateX" | "rotatex" => {
                let angle = parse_angle(args.first().copied().unwrap_or("0"));
                result.push(TransformFunction::RotateX(angle));
            },
            "rotateY" | "rotatey" => {
                let angle = parse_angle(args.first().copied().unwrap_or("0"));
                result.push(TransformFunction::RotateY(angle));
            },
            "rotateZ" | "rotatez" => {
                let angle = parse_angle(args.first().copied().unwrap_or("0"));
                result.push(TransformFunction::RotateZ(angle));
            },
            "rotate3d" => {
                if args.len() >= 4 {
                    let x = args[0].parse::<f32>().unwrap_or(0.0);
                    let y = args[1].parse::<f32>().unwrap_or(0.0);
                    let z = args[2].parse::<f32>().unwrap_or(0.0);
                    let angle = parse_angle(args[3]);
                    result.push(TransformFunction::Rotate3d(x, y, z, angle));
                }
            },
            "matrix3d" => {
                if args.len() >= 16 {
                    let mut m = [0.0f32; 16];
                    for (i, slot) in m.iter_mut().enumerate() {
                        *slot = args[i].parse::<f32>().unwrap_or(0.0);
                    }
                    result.push(TransformFunction::Matrix3d(m));
                }
            },
            "perspective" => {
                let d =
                    parse_transform_length(args.first().copied().unwrap_or("0"), parent_font_size);
                if d > 0.0 {
                    result.push(TransformFunction::Perspective(d));
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
                z: 0.0,
                x_pct: None,
                y_pct: None,
            };
        },
        CssValue::Percentage(p) => {
            return TransformOrigin {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                x_pct: Some(*p / 100.0),
                y_pct: Some(0.5),
            };
        },
        _ => {
            return TransformOrigin {
                x: 0.0,
                y: 0.0,
                z: 0.0,
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
    let mut z: f32 = 0.0;

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

    // Optional third token is the Z origin in pixels (no percentage form).
    if let Some(p2) = parts.get(2) {
        z = parse_origin_length(p2, parent_font_size);
    }

    TransformOrigin {
        x,
        y,
        z,
        x_pct,
        y_pct,
    }
}

/// Parse a CSS length used in transform-origin Z position. Accepts
/// `px`, `em`, `rem`, and bare numbers (treated as px).
fn parse_origin_length(s: &str, parent_font_size: f32) -> f32 {
    let s = s.trim();
    if let Some(px) = s.strip_suffix("px") {
        px.trim().parse::<f32>().unwrap_or(0.0)
    } else if let Some(em) = s.strip_suffix("em") {
        em.trim().parse::<f32>().unwrap_or(0.0) * parent_font_size
    } else if let Some(rem) = s.strip_suffix("rem") {
        rem.trim().parse::<f32>().unwrap_or(0.0) * super::types::ROOT_FONT_SIZE
    } else {
        s.parse::<f32>().unwrap_or(0.0)
    }
}

/// Parse a CSS `perspective-origin` value into a structured
/// [`super::types::PerspectiveOrigin`]. Supports the same `keyword`,
/// `<percentage>`, `<length>`, and one/two-token forms as
/// `transform-origin`, but without a Z component.
fn parse_perspective_origin(
    value: &CssValue,
    parent_font_size: f32,
) -> super::types::PerspectiveOrigin {
    use super::types::PerspectiveOrigin;

    let raw = match value {
        CssValue::Keyword(s) | CssValue::String(s) => s.clone(),
        CssValue::Length(_, _) => {
            let px = resolve_length(value, parent_font_size);
            return PerspectiveOrigin {
                x: px,
                y: 0.0,
                x_pct: None,
                y_pct: Some(0.5),
            };
        },
        CssValue::Percentage(p) => {
            return PerspectiveOrigin {
                x: 0.0,
                y: 0.0,
                x_pct: Some(*p / 100.0),
                y_pct: Some(0.5),
            };
        },
        _ => {
            return PerspectiveOrigin {
                x: 0.0,
                y: 0.0,
                x_pct: Some(0.5),
                y_pct: Some(0.5),
            };
        },
    };

    let parts: Vec<&str> = raw.split_whitespace().collect();

    let resolve_part = |s: &str| -> (f32, Option<f32>) {
        match s {
            "left" | "top" => (0.0, Some(0.0)),
            "center" => (0.0, Some(0.5)),
            "right" | "bottom" => (0.0, Some(1.0)),
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

    let mut x_pct: Option<f32> = None;
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    let y_pct: Option<f32>;

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
        y_pct = Some(0.5);
    }

    PerspectiveOrigin { x, y, x_pct, y_pct }
}

/// Parse a CSS `clip-path` value into a structured [`ClipPath`].
///
/// Accepts: `inset(top [right [bottom [left]]])`, `rect(t, r, b, l)`,
/// `circle(r [at cx cy])`, `ellipse(rx ry [at cx cy])`. Length units are
/// resolved against `parent_font_size` for em values; percentages become
/// fractions (0..=1) resolved against the border box at paint time.
///
/// Unsupported forms (e.g. `polygon()`, SVG `url(#id)`) return `None`.
fn parse_clip_path(value: &CssValue, parent_font_size: f32) -> Option<super::types::ClipPath> {
    use super::types::{ClipLength, ClipPath};

    let raw = match value {
        CssValue::Keyword(s) if s == "none" => return None,
        CssValue::Keyword(s) | CssValue::String(s) => s.trim(),
        _ => return None,
    };

    let paren = raw.find('(')?;
    let func = raw[..paren].trim();
    let close = raw.rfind(')')?;
    if close <= paren {
        return None;
    }
    let args_str = raw[paren + 1..close].trim();

    // Split on `at` to separate shape args from position args.
    let (shape_args, pos_args) = match args_str.split_once(" at ") {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (args_str, None),
    };

    // Tokenize shape args on whitespace (commas treated as whitespace).
    let shape_tokens: Vec<&str> = shape_args
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .collect();

    let parse_len = |tok: &str| -> Option<ClipLength> {
        if let Some(pct) = tok.strip_suffix('%') {
            pct.trim()
                .parse::<f32>()
                .ok()
                .filter(|v| *v >= 0.0)
                .map(|v| ClipLength::Frac(v / 100.0))
        } else if let Some(px) = tok.strip_suffix("px") {
            px.trim()
                .parse::<f32>()
                .ok()
                .filter(|v| *v >= 0.0)
                .map(ClipLength::Px)
        } else if let Some(em) = tok.strip_suffix("em") {
            em.trim()
                .parse::<f32>()
                .ok()
                .filter(|v| *v >= 0.0)
                .map(|v| ClipLength::Px(v * parent_font_size))
        } else if let Some(rem) = tok.strip_suffix("rem") {
            rem.trim()
                .parse::<f32>()
                .ok()
                .filter(|v| *v >= 0.0)
                .map(|v| ClipLength::Px(v * super::types::ROOT_FONT_SIZE))
        } else if tok == "0" {
            Some(ClipLength::Px(0.0))
        } else {
            tok.parse::<f32>()
                .ok()
                .filter(|v| *v >= 0.0)
                .map(ClipLength::Px)
        }
    };

    // `at <x> <y>` → (cx, cy). Defaults to 50% 50% (center).
    let parse_at = |s: Option<&str>| -> (ClipLength, ClipLength) {
        let default = (ClipLength::Frac(0.5), ClipLength::Frac(0.5));
        let Some(s) = s else {
            return default;
        };
        let toks: Vec<&str> = s.split_whitespace().collect();
        let cx = toks.first().and_then(|t| parse_len(t)).unwrap_or(default.0);
        let cy = toks.get(1).and_then(|t| parse_len(t)).unwrap_or(default.1);
        (cx, cy)
    };

    match func {
        "inset" => {
            // CSS shorthand: 1-4 values like margin/padding.
            let t = parse_len(shape_tokens.first()?)?;
            let r = shape_tokens.get(1).and_then(|s| parse_len(s)).unwrap_or(t);
            let b = shape_tokens.get(2).and_then(|s| parse_len(s)).unwrap_or(t);
            let l = shape_tokens.get(3).and_then(|s| parse_len(s)).unwrap_or(r);
            Some(ClipPath::Inset {
                top: t,
                right: r,
                bottom: b,
                left: l,
            })
        },
        "rect" => {
            // Legacy `rect(top, right, bottom, left)`. All values must be px
            // lengths or `auto`. Fractions not allowed here per CSS 2.1.
            let to_px = |tok: &str| -> Option<Option<f32>> {
                if tok == "auto" {
                    return Some(None);
                }
                match parse_len(tok)? {
                    ClipLength::Px(v) => Some(Some(v)),
                    ClipLength::Frac(_) => None,
                }
            };
            let t = to_px(shape_tokens.first()?)?;
            let r = to_px(shape_tokens.get(1)?)?;
            let b = to_px(shape_tokens.get(2)?)?;
            let l = to_px(shape_tokens.get(3)?)?;
            Some(ClipPath::Rect {
                top: t,
                right: r,
                bottom: b,
                left: l,
            })
        },
        "circle" => {
            let r = shape_tokens
                .first()
                .and_then(|s| parse_len(s))
                .unwrap_or(ClipLength::Frac(0.5));
            let (cx, cy) = parse_at(pos_args);
            Some(ClipPath::Circle { cx, cy, r })
        },
        "ellipse" => {
            let rx = shape_tokens
                .first()
                .and_then(|s| parse_len(s))
                .unwrap_or(ClipLength::Frac(0.5));
            let ry = shape_tokens.get(1).and_then(|s| parse_len(s)).unwrap_or(rx);
            let (cx, cy) = parse_at(pos_args);
            Some(ClipPath::Ellipse { cx, cy, rx, ry })
        },
        _ => None,
    }
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
        assert_eq!(s.font_weight, FontWeight::BOLD);
    }

    #[test]
    fn apply_font_weight_bold_number() {
        // The CSS parser normalises "bold" to Number(700.0).
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-weight", &CssValue::Number(700.0), 16.0);
        assert_eq!(s.font_weight, FontWeight::BOLD);
    }

    #[test]
    fn apply_font_weight_normal_number() {
        let mut s = ComputedStyle::default();
        s.font_weight = FontWeight::BOLD;
        s.apply_declaration("font-weight", &CssValue::Number(400.0), 16.0);
        assert_eq!(s.font_weight, FontWeight::NORMAL);
    }

    #[test]
    fn apply_font_weight_numeric() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-weight", &CssValue::Number(300.0), 16.0);
        assert_eq!(s.font_weight, FontWeight(300));
        assert!(!s.font_weight.is_bold());
        s.apply_declaration("font-weight", &CssValue::Number(600.0), 16.0);
        assert_eq!(s.font_weight, FontWeight(600));
        assert!(s.font_weight.is_bold());
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
    fn multi_layer_background_image_takes_first_layer() {
        // `background-image: url(a), url(b)` parses as
        // `CssValue::Multiple([Url(a), Url(b)])`. The engine only
        // supports single-layer semantics, so the first layer should
        // win instead of the whole declaration being dropped.
        let mut s = ComputedStyle::default();
        let value = CssValue::Multiple(vec![
            CssValue::Url("a.png".into()),
            CssValue::Url("b.png".into()),
        ]);
        s.apply_declaration("background-image", &value, 16.0);
        assert_eq!(s.background_image, BackgroundImage::Url("a.png".into()));
    }

    #[test]
    fn multi_layer_mask_image_takes_first_layer() {
        // Same behaviour for mask-image: without the `Multiple` arm
        // the fallthrough left `mask_image = None`, silently
        // removing the mask on any page using the multi-layer form.
        let mut s = ComputedStyle::default();
        let value = CssValue::Multiple(vec![
            CssValue::Url("mask-a.png".into()),
            CssValue::Url("mask-b.png".into()),
        ]);
        s.apply_declaration("mask-image", &value, 16.0);
        assert_eq!(s.mask_image, BackgroundImage::Url("mask-a.png".into()));
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
                    | "white-space" | "text-wrap" | "line-height"
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

    // -- Extended property coverage tests --------------------------------

    #[test]
    fn parse_scroll_behavior_smooth() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("scroll-behavior", &CssValue::Keyword("smooth".into()), 16.0);
        assert_eq!(s.scroll_behavior, ScrollBehavior::Smooth);
    }

    #[test]
    fn parse_clip_path_inset_four_values() {
        use super::super::types::{ClipLength, ClipPath};
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "clip-path",
            &CssValue::Keyword("inset(10px 20px 30px 40px)".into()),
            16.0,
        );
        match s.clip_path {
            Some(ClipPath::Inset {
                top,
                right,
                bottom,
                left,
            }) => {
                assert_eq!(top, ClipLength::Px(10.0));
                assert_eq!(right, ClipLength::Px(20.0));
                assert_eq!(bottom, ClipLength::Px(30.0));
                assert_eq!(left, ClipLength::Px(40.0));
            },
            other => panic!("expected Inset, got {other:?}"),
        }
    }

    #[test]
    fn parse_clip_path_inset_shorthand_one_value() {
        use super::super::types::{ClipLength, ClipPath};
        let mut s = ComputedStyle::default();
        s.apply_declaration("clip-path", &CssValue::Keyword("inset(5%)".into()), 16.0);
        match s.clip_path {
            Some(ClipPath::Inset {
                top,
                right,
                bottom,
                left,
            }) => {
                assert_eq!(top, ClipLength::Frac(0.05));
                assert_eq!(right, ClipLength::Frac(0.05));
                assert_eq!(bottom, ClipLength::Frac(0.05));
                assert_eq!(left, ClipLength::Frac(0.05));
            },
            other => panic!("expected Inset, got {other:?}"),
        }
    }

    #[test]
    fn parse_clip_path_circle_with_at() {
        use super::super::types::{ClipLength, ClipPath};
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "clip-path",
            &CssValue::Keyword("circle(50% at 25% 75%)".into()),
            16.0,
        );
        match s.clip_path {
            Some(ClipPath::Circle { cx, cy, r }) => {
                assert_eq!(r, ClipLength::Frac(0.5));
                assert_eq!(cx, ClipLength::Frac(0.25));
                assert_eq!(cy, ClipLength::Frac(0.75));
            },
            other => panic!("expected Circle, got {other:?}"),
        }
    }

    #[test]
    fn parse_clip_path_circle_single_coordinate_at() {
        use super::super::types::{ClipLength, ClipPath};
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "clip-path",
            &CssValue::Keyword("circle(50% at 25%)".into()),
            16.0,
        );
        match s.clip_path {
            Some(ClipPath::Circle { cx, cy, r }) => {
                assert_eq!(r, ClipLength::Frac(0.5));
                assert_eq!(cx, ClipLength::Frac(0.25));
                assert_eq!(cy, ClipLength::Frac(0.5));
            },
            other => panic!("expected Circle, got {other:?}"),
        }
    }

    #[test]
    fn parse_clip_path_ellipse_default_center() {
        use super::super::types::{ClipLength, ClipPath};
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "clip-path",
            &CssValue::Keyword("ellipse(40px 20px)".into()),
            16.0,
        );
        match s.clip_path {
            Some(ClipPath::Ellipse { cx, cy, rx, ry }) => {
                assert_eq!(rx, ClipLength::Px(40.0));
                assert_eq!(ry, ClipLength::Px(20.0));
                assert_eq!(cx, ClipLength::Frac(0.5));
                assert_eq!(cy, ClipLength::Frac(0.5));
            },
            other => panic!("expected Ellipse, got {other:?}"),
        }
    }

    #[test]
    fn parse_clip_path_none_clears() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("clip-path", &CssValue::Keyword("inset(10px)".into()), 16.0);
        assert!(s.clip_path.is_some());
        s.apply_declaration("clip-path", &CssValue::Keyword("none".into()), 16.0);
        assert!(s.clip_path.is_none());
    }

    #[test]
    fn parse_mix_blend_mode() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "mix-blend-mode",
            &CssValue::Keyword("multiply".into()),
            16.0,
        );
        assert_eq!(s.mix_blend_mode, BlendMode::Multiply);
    }

    #[test]
    fn parse_background_clip_text() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("background-clip", &CssValue::Keyword("text".into()), 16.0);
        assert_eq!(s.background_clip, BackgroundBox::Text);
    }

    #[test]
    fn parse_image_rendering_pixelated() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "image-rendering",
            &CssValue::Keyword("pixelated".into()),
            16.0,
        );
        assert_eq!(s.image_rendering, ImageRendering::Pixelated);
    }

    #[test]
    fn parse_font_stretch_condensed() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("font-stretch", &CssValue::Keyword("condensed".into()), 16.0);
        assert_eq!(s.font_stretch, FontStretch::Condensed);
    }

    #[test]
    fn parse_hyphens_auto() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("hyphens", &CssValue::Keyword("auto".into()), 16.0);
        assert_eq!(s.hyphens, Hyphens::Auto);
    }

    #[test]
    fn parse_text_align_last() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("text-align-last", &CssValue::Keyword("center".into()), 16.0);
        assert_eq!(s.text_align_last, TextAlignLast::Center);
    }

    #[test]
    fn parse_text_decoration_thickness_px() {
        use crate::css::parser::LengthUnit;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "text-decoration-thickness",
            &CssValue::Length(2.0, LengthUnit::Px),
            16.0,
        );
        assert_eq!(s.text_decoration_thickness, Some(2.0));
    }

    #[test]
    fn parse_text_decoration_thickness_auto() {
        let mut s = ComputedStyle::default();
        s.text_decoration_thickness = Some(3.0);
        s.apply_declaration(
            "text-decoration-thickness",
            &CssValue::Keyword("auto".into()),
            16.0,
        );
        assert_eq!(s.text_decoration_thickness, None);
    }

    #[test]
    fn parse_perspective_length() {
        use crate::css::parser::LengthUnit;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "perspective",
            &CssValue::Length(500.0, LengthUnit::Px),
            16.0,
        );
        assert_eq!(s.perspective, Some(500.0));
    }

    #[test]
    fn parse_backface_visibility_hidden() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "backface-visibility",
            &CssValue::Keyword("hidden".into()),
            16.0,
        );
        assert_eq!(s.backface_visibility, BackfaceVisibility::Hidden);
    }

    #[test]
    fn parse_transform_origin_three_value_includes_z() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "transform-origin",
            &CssValue::String("25% 75% 40px".into()),
            16.0,
        );
        let origin = s.transform_origin.expect("transform-origin parsed");
        assert_eq!(origin.x_pct, Some(0.25));
        assert_eq!(origin.y_pct, Some(0.75));
        assert!((origin.z - 40.0).abs() < 1e-4);
    }

    #[test]
    fn parse_perspective_origin_to_structured_value() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "perspective-origin",
            &CssValue::String("left center".into()),
            16.0,
        );
        let origin = s.perspective_origin.expect("perspective-origin parsed");
        assert_eq!(origin.x_pct, Some(0.0));
        assert_eq!(origin.y_pct, Some(0.5));
    }

    #[test]
    fn parse_transform_3d_functions() {
        use super::super::types::TransformFunction;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "transform",
            &CssValue::String(
                "translate3d(10px, 20px, 30px) rotateX(45deg) rotateY(60deg) scale3d(1, 2, 3) \
                 perspective(500px)"
                    .into(),
            ),
            16.0,
        );
        assert_eq!(s.transforms.len(), 5);
        assert!(matches!(
            s.transforms[0],
            TransformFunction::Translate3d(10.0, 20.0, 30.0)
        ));
        assert!(
            matches!(s.transforms[1], TransformFunction::RotateX(d) if (d - 45.0).abs() < 1e-4)
        );
        assert!(
            matches!(s.transforms[2], TransformFunction::RotateY(d) if (d - 60.0).abs() < 1e-4)
        );
        assert!(matches!(
            s.transforms[3],
            TransformFunction::Scale3d(1.0, 2.0, 3.0)
        ));
        assert!(
            matches!(s.transforms[4], TransformFunction::Perspective(d) if (d - 500.0).abs() < 1e-4)
        );
    }

    #[test]
    fn parse_transform_rotate3d_and_matrix3d() {
        use super::super::types::TransformFunction;
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "transform",
            &CssValue::String(
                "rotate3d(0, 1, 0, 90deg) matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 5,6,7,1)".into(),
            ),
            16.0,
        );
        assert_eq!(s.transforms.len(), 2);
        assert!(matches!(
            s.transforms[0],
            TransformFunction::Rotate3d(0.0, 1.0, 0.0, d) if (d - 90.0).abs() < 1e-4
        ));
        if let TransformFunction::Matrix3d(values) = &s.transforms[1] {
            assert_eq!(values[12], 5.0);
            assert_eq!(values[13], 6.0);
            assert_eq!(values[14], 7.0);
            assert_eq!(values[15], 1.0);
        } else {
            panic!("expected Matrix3d, got {:?}", s.transforms[1]);
        }
    }

    #[test]
    fn parse_transform_style_preserve_3d() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "transform-style",
            &CssValue::Keyword("preserve-3d".into()),
            16.0,
        );
        assert_eq!(s.transform_style, TransformStyle::Preserve3d);
    }

    #[test]
    fn parse_overscroll_behavior_shorthand_sets_both_axes() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "overscroll-behavior",
            &CssValue::Keyword("contain".into()),
            16.0,
        );
        assert_eq!(s.overscroll_behavior_x, OverscrollBehavior::Contain);
        assert_eq!(s.overscroll_behavior_y, OverscrollBehavior::Contain);
    }

    #[test]
    fn parse_content_visibility_auto() {
        let mut s = ComputedStyle::default();
        s.apply_declaration(
            "content-visibility",
            &CssValue::Keyword("auto".into()),
            16.0,
        );
        assert_eq!(s.content_visibility, ContentVisibility::Auto);
    }

    #[test]
    fn parse_justify_self_center() {
        let mut s = ComputedStyle::default();
        s.apply_declaration("justify-self", &CssValue::Keyword("center".into()), 16.0);
        assert_eq!(s.justify_self, JustifySelf::Center);
    }

    #[test]
    fn parse_inset_shorthand_sets_all_four_sides() {
        use crate::css::parser::LengthUnit;
        let mut s = ComputedStyle::default();
        s.apply_declaration("inset", &CssValue::Length(10.0, LengthUnit::Px), 16.0);
        assert_eq!(s.top, Dimension::Px(10.0));
        assert_eq!(s.right, Dimension::Px(10.0));
        assert_eq!(s.bottom, Dimension::Px(10.0));
        assert_eq!(s.left, Dimension::Px(10.0));
    }
}
