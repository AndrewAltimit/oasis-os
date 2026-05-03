//! `apply_initial` reset, transition/animation parsing, and inline-logical resolution.
//!
//! These methods live on `ComputedStyle` but are factored out from the
//! main `apply_declaration` dispatch so each concern (reset, shorthand
//! parsing, logical-property mapping) is browseable on its own.

use super::super::computed::ComputedStyle;
use super::super::resolve::resolve_length;
use super::super::types::{
    Animation, AnimationDirection, AnimationFillMode, AnimationPlayState, Appearance,
    BackgroundPosition, BackgroundRepeat, BackgroundSize, ColorScheme, Cursor, Isolation,
    ObjectFit, ObjectPosition, Overflow, PointerEvents, Resize, TextDecorationStyle, TextDirection,
    TimingFunction, TouchAction, Transition, UserSelect,
};
use crate::css::parser::CssValue;

use super::parsers::{
    parse_animation_direction, parse_animation_fill_mode, parse_animation_play_state, parse_time,
    parse_timing_function,
};

impl ComputedStyle {
    /// Parse a `transition` shorthand value into a [`Transition`].
    ///
    /// Format: `<property> <duration> [<timing>] [<delay>]`
    /// Example: `all 0.3s ease`, `color 200ms linear 50ms`
    pub(super) fn parse_transition(value: &CssValue) -> Option<Transition> {
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
    pub(super) fn parse_animation(value: &CssValue) -> Option<Animation> {
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

    /// Map an inline-axis logical property name to its physical
    /// equivalent based on `self.direction`. Returns `None` for
    /// non-logical properties (i.e. most properties — fast path).
    ///
    /// In LTR: `*-inline-start` → left, `*-inline-end` → right.
    /// In RTL: `*-inline-start` → right, `*-inline-end` → left.
    pub(super) fn resolve_inline_logical(&self, property: &str) -> Option<&'static str> {
        // Fast path: skip the table scan for properties that can't be
        // logical. The vast majority of properties don't contain "inline".
        if !property.contains("inline") {
            return None;
        }
        let is_rtl = self.direction == TextDirection::Rtl;

        // (logical_name, physical_ltr, physical_rtl)
        const MAP: &[(&str, &str, &str)] = &[
            ("margin-inline-start", "margin-left", "margin-right"),
            ("margin-inline-end", "margin-right", "margin-left"),
            ("padding-inline-start", "padding-left", "padding-right"),
            ("padding-inline-end", "padding-right", "padding-left"),
            ("inset-inline-start", "left", "right"),
            ("inset-inline-end", "right", "left"),
            (
                "border-inline-start-width",
                "border-left-width",
                "border-right-width",
            ),
            (
                "border-inline-end-width",
                "border-right-width",
                "border-left-width",
            ),
            (
                "border-inline-start-color",
                "border-left-color",
                "border-right-color",
            ),
            (
                "border-inline-end-color",
                "border-right-color",
                "border-left-color",
            ),
            (
                "border-inline-start-style",
                "border-left-style",
                "border-right-style",
            ),
            (
                "border-inline-end-style",
                "border-right-style",
                "border-left-style",
            ),
        ];
        for &(logical, ltr, rtl) in MAP {
            if property == logical {
                return Some(if is_rtl { rtl } else { ltr });
            }
        }
        None
    }

    /// Reset a single property to its CSS initial value.
    pub(super) fn apply_initial(&mut self, property: &str) {
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
    pub(super) fn resolve_bg_size_component(
        value: &CssValue,
        parent_font_size: f32,
    ) -> Option<f32> {
        match value {
            CssValue::Keyword(kw) if kw == "auto" => None,
            CssValue::Percentage(p) => Some(-*p), // negative = percentage
            _ => Some(resolve_length(value, parent_font_size)),
        }
    }

    /// Resolve a background-position value.
    pub(super) fn resolve_bg_position(
        value: &CssValue,
        parent_font_size: f32,
    ) -> BackgroundPosition {
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
    pub(super) fn resolve_obj_position(value: &CssValue, parent_font_size: f32) -> ObjectPosition {
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
    pub(super) fn more_restrictive(a: Overflow, b: Overflow) -> Overflow {
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
