//! Free-function parsers for the value tokens that `apply_declaration`
//! consumes — time/timing/iteration counts, `transform`, `clip-path`,
//! `filter`, `grid-template-areas`, font families, and counter directives.
//!
//! Pure value-to-typed-enum converters; no `ComputedStyle` access.

use super::super::resolve::{as_keyword, resolve_length};
use super::super::types::{
    AnimationDirection, AnimationFillMode, AnimationPlayState, BackgroundBox, BlendMode,
    FontFamily, FontFamilyName, JustifySelf, OverscrollBehavior, TimingFunction,
};
use crate::css::parser::CssValue;

/// Parse a CSS time value (e.g. `0.3s`, `200ms`) into milliseconds.
pub(super) fn parse_time(s: &str) -> Option<f32> {
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
pub(super) fn parse_timing_function(s: &str) -> Option<TimingFunction> {
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
pub(super) fn parse_iteration_count(s: &str) -> f32 {
    if s == "infinite" {
        f32::INFINITY
    } else {
        s.parse::<f32>().unwrap_or(1.0)
    }
}

pub(super) fn string_or_keyword(value: &CssValue) -> Option<String> {
    match value {
        CssValue::String(s) => Some(s.clone()),
        CssValue::Keyword(k) => Some(k.clone()),
        _ => None,
    }
}

pub(super) fn parse_overscroll(value: &CssValue) -> Option<OverscrollBehavior> {
    match as_keyword(value)? {
        "contain" => Some(OverscrollBehavior::Contain),
        "none" => Some(OverscrollBehavior::None),
        "auto" => Some(OverscrollBehavior::Auto),
        _ => None,
    }
}

pub(super) fn parse_blend_mode(s: &str) -> Option<BlendMode> {
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

pub(super) fn parse_background_box(s: &str) -> Option<BackgroundBox> {
    Some(match s {
        "border-box" => BackgroundBox::BorderBox,
        "padding-box" => BackgroundBox::PaddingBox,
        "content-box" => BackgroundBox::ContentBox,
        "text" => BackgroundBox::Text,
        _ => return None,
    })
}

pub(super) fn parse_justify_self(s: &str) -> Option<JustifySelf> {
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
pub(super) fn parse_animation_direction(s: &str) -> Option<AnimationDirection> {
    match s {
        "normal" => Some(AnimationDirection::Normal),
        "reverse" => Some(AnimationDirection::Reverse),
        "alternate" => Some(AnimationDirection::Alternate),
        "alternate-reverse" => Some(AnimationDirection::AlternateReverse),
        _ => None,
    }
}

/// Parse a CSS `animation-fill-mode` keyword.
pub(super) fn parse_animation_fill_mode(s: &str) -> Option<AnimationFillMode> {
    match s {
        "none" => Some(AnimationFillMode::None),
        "forwards" => Some(AnimationFillMode::Forwards),
        "backwards" => Some(AnimationFillMode::Backwards),
        "both" => Some(AnimationFillMode::Both),
        _ => None,
    }
}

/// Parse a CSS `animation-play-state` keyword.
pub(super) fn parse_animation_play_state(s: &str) -> Option<AnimationPlayState> {
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
pub(super) fn parse_transform(
    value: &CssValue,
    parent_font_size: f32,
) -> Vec<super::super::types::TransformFunction> {
    use super::super::types::TransformFunction;

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
            "matrix" if args.len() >= 6 => {
                let a = args[0].parse::<f32>().unwrap_or(1.0);
                let b = args[1].parse::<f32>().unwrap_or(0.0);
                let c = args[2].parse::<f32>().unwrap_or(0.0);
                let d = args[3].parse::<f32>().unwrap_or(1.0);
                let e = args[4].parse::<f32>().unwrap_or(0.0);
                let f = args[5].parse::<f32>().unwrap_or(0.0);
                result.push(TransformFunction::Matrix(a, b, c, d, e, f));
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
            "rotate3d" if args.len() >= 4 => {
                let x = args[0].parse::<f32>().unwrap_or(0.0);
                let y = args[1].parse::<f32>().unwrap_or(0.0);
                let z = args[2].parse::<f32>().unwrap_or(0.0);
                let angle = parse_angle(args[3]);
                result.push(TransformFunction::Rotate3d(x, y, z, angle));
            },
            "matrix3d" if args.len() >= 16 => {
                let mut m = [0.0f32; 16];
                for (i, slot) in m.iter_mut().enumerate() {
                    *slot = args[i].parse::<f32>().unwrap_or(0.0);
                }
                result.push(TransformFunction::Matrix3d(m));
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
            rem.trim().parse::<f32>().unwrap_or(0.0) * super::super::types::current_root_font_size()
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
pub(super) fn parse_transform_origin(
    value: &CssValue,
    parent_font_size: f32,
) -> super::super::types::TransformOrigin {
    use super::super::types::TransformOrigin;

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
        rem.trim().parse::<f32>().unwrap_or(0.0) * super::super::types::current_root_font_size()
    } else {
        s.parse::<f32>().unwrap_or(0.0)
    }
}

/// Parse a CSS `perspective-origin` value into a structured
/// [`super::super::types::PerspectiveOrigin`]. Supports the same `keyword`,
/// `<percentage>`, `<length>`, and one/two-token forms as
/// `transform-origin`, but without a Z component.
pub(super) fn parse_perspective_origin(
    value: &CssValue,
    parent_font_size: f32,
) -> super::super::types::PerspectiveOrigin {
    use super::super::types::PerspectiveOrigin;

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
pub(super) fn parse_clip_path(
    value: &CssValue,
    parent_font_size: f32,
) -> Option<super::super::types::ClipPath> {
    use super::super::types::{ClipLength, ClipPath};

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
                .map(|v| ClipLength::Px(v * super::super::types::current_root_font_size()))
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
pub(super) fn parse_filter(value: &CssValue) -> Vec<super::super::types::FilterFunction> {
    use super::super::types::FilterFunction;

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
pub(super) fn parse_counter_directive(value: &CssValue) -> Vec<(String, i32)> {
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
pub(super) fn resolve_content_counters(
    content: &str,
    _counters: &std::collections::HashMap<String, i32>,
) -> String {
    // Placeholder implementation -- returns content unchanged.
    content.to_string()
}

/// Parse `grid-template-areas` value.
pub(super) fn parse_grid_template_areas(value: &CssValue) -> Vec<Vec<String>> {
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

/// Parse a `font-family` CSS value into a [`FontFamily`] stack.
///
/// Handles comma-separated lists of quoted names, unquoted multi-word
/// names, and generic family keywords:
///
/// ```css
/// font-family: "Open Sans", Helvetica, Arial, sans-serif;
/// ```
pub(super) fn parse_font_family_value(value: &CssValue) -> FontFamily {
    let names = match value {
        CssValue::Keyword(kw) => {
            // Single keyword — either a generic or a bare font name.
            return FontFamily::generic(keyword_to_family_name(kw));
        },
        CssValue::String(s) => {
            // Single quoted string — a named font.
            return FontFamily::stack(vec![FontFamilyName::Named(s.clone())]);
        },
        CssValue::Multiple(vs) => {
            // Comma-separated list. The parser may group tokens between
            // commas into sub-Multiple nodes, or present them flat
            // separated by Keyword(",") or simply as sequential items.
            // We collect all entries, splitting on commas.
            collect_font_family_names(vs)
        },
        _ => return FontFamily::default(),
    };
    if names.is_empty() {
        FontFamily::default()
    } else {
        FontFamily::stack(names)
    }
}

/// Map a single keyword to a [`FontFamilyName`].
pub(super) fn keyword_to_family_name(kw: &str) -> FontFamilyName {
    match kw.to_ascii_lowercase().as_str() {
        "serif" => FontFamilyName::Serif,
        "sans-serif" => FontFamilyName::SansSerif,
        "monospace" => FontFamilyName::Monospace,
        "cursive" => FontFamilyName::Cursive,
        "fantasy" => FontFamilyName::Fantasy,
        "system-ui" => FontFamilyName::SystemUi,
        other => FontFamilyName::Named(other.to_string()),
    }
}

/// Walk a flat list of [`CssValue`]s (from a comma-separated font-family
/// declaration) and produce an ordered list of [`FontFamilyName`]s.
pub(super) fn collect_font_family_names(values: &[CssValue]) -> Vec<FontFamilyName> {
    let mut result = Vec::new();
    let mut pending_idents: Vec<String> = Vec::new();

    for v in values {
        match v {
            CssValue::String(s) => {
                // Flush any accumulated bare idents as a single name.
                flush_pending_idents(&mut pending_idents, &mut result);
                result.push(FontFamilyName::Named(s.clone()));
            },
            CssValue::Keyword(kw) if kw == "," => {
                // Comma separator — flush accumulated idents.
                flush_pending_idents(&mut pending_idents, &mut result);
            },
            CssValue::Keyword(kw) => {
                // Bare ident — could be a multi-word name or a generic.
                pending_idents.push(kw.clone());
            },
            CssValue::Multiple(sub) => {
                // Nested grouping — recurse.
                flush_pending_idents(&mut pending_idents, &mut result);
                result.extend(collect_font_family_names(sub));
            },
            _ => {},
        }
    }
    flush_pending_idents(&mut pending_idents, &mut result);
    result
}

/// Flush accumulated bare ident tokens into a single font family name.
///
/// Multi-word unquoted names like `Trebuchet MS` come through as
/// separate Keyword tokens; this joins them with spaces.
pub(super) fn flush_pending_idents(idents: &mut Vec<String>, result: &mut Vec<FontFamilyName>) {
    if idents.is_empty() {
        return;
    }
    let joined = idents.join(" ");
    idents.clear();
    result.push(keyword_to_family_name(&joined));
}
