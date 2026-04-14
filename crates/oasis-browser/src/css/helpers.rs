//! CSS helper functions for colour parsing, unit parsing, media-query
//! evaluation, and CSS text reconstruction.
//!
//! Extracted from [`super::parser`] to keep the main parser module focused
//! on token-stream consumption and AST construction.

use super::parser::{CssColor, CssValue, LengthUnit};
use super::tokenizer::CssToken;

// Re-import `parse_value_list` so `parse_font_weight` can call it.
use super::parser::parse_value_list;

// -------------------------------------------------------------------
// Font-weight helper
// -------------------------------------------------------------------

pub(crate) fn parse_font_weight(tokens: &[CssToken]) -> CssValue {
    let non_ws: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t, CssToken::Whitespace))
        .collect();
    if non_ws.len() == 1 {
        match &non_ws[0] {
            CssToken::Ident(s) => {
                let lower = s.to_ascii_lowercase();
                return match lower.as_str() {
                    "bold" => CssValue::Number(700.0),
                    "normal" => CssValue::Number(400.0),
                    "lighter" => CssValue::Number(100.0),
                    "bolder" => CssValue::Number(900.0),
                    _ => CssValue::Keyword(s.clone()),
                };
            },
            CssToken::Number(n) => return CssValue::Number(*n),
            _ => {},
        }
    }
    let values = parse_value_list(tokens);
    match values.len() {
        1 => match values.into_iter().next() {
            Some(v) => v,
            None => CssValue::Keyword(String::new()),
        },
        _ => CssValue::Multiple(values),
    }
}

// -------------------------------------------------------------------
// Property classification
// -------------------------------------------------------------------

pub(crate) fn is_color_property(prop: &str) -> bool {
    matches!(
        prop,
        "color"
            | "background-color"
            | "border-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "outline-color"
    )
}

// -------------------------------------------------------------------
// Unit parsing
// -------------------------------------------------------------------

pub(crate) fn parse_unit(unit: &str) -> Option<LengthUnit> {
    match unit.to_ascii_lowercase().as_str() {
        "px" => Some(LengthUnit::Px),
        "em" => Some(LengthUnit::Em),
        "rem" => Some(LengthUnit::Rem),
        "pt" => Some(LengthUnit::Pt),
        _ => None,
    }
}

// -------------------------------------------------------------------
// Colour parsing
// -------------------------------------------------------------------

pub(crate) fn try_parse_color(tokens: &[CssToken]) -> Option<CssColor> {
    let non_ws: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t, CssToken::Whitespace))
        .collect();
    if non_ws.is_empty() {
        return None;
    }

    // Single hash: #rgb / #rrggbb / #rgba / #rrggbbaa.
    if non_ws.len() == 1 {
        if let CssToken::Hash(h) = non_ws[0] {
            return parse_hex_color(h);
        }
        if let CssToken::Ident(name) = non_ws[0] {
            return named_color(name);
        }
    }

    // Functional color notations.
    if let CssToken::Function(name) = non_ws[0] {
        let lower = name.to_ascii_lowercase();
        let body = function_body(&non_ws[1..]);
        return match lower.as_str() {
            "rgb" | "rgba" => parse_rgb_function(&body),
            "hsl" | "hsla" => parse_hsl_function(&body),
            "oklch" => parse_oklch_function(&body),
            "oklab" => parse_oklab_function(&body),
            "color" => parse_color_function(&body),
            "color-mix" => parse_color_mix_function(&body),
            "light-dark" => parse_light_dark_function(&body),
            _ => None,
        };
    }

    None
}

/// Extract the token slice up to (but not including) the top-level
/// `)` that closes the current function. Nested parens are respected.
fn function_body<'a>(tokens: &[&'a CssToken]) -> Vec<&'a CssToken> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    for &t in tokens {
        match t {
            CssToken::OpenParen | CssToken::Function(_) => {
                depth += 1;
                out.push(t);
            },
            CssToken::CloseParen => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                out.push(t);
            },
            _ => out.push(t),
        }
    }
    out
}

/// Split a function body by top-level commas, respecting nested parens.
fn split_top_level_commas<'a>(tokens: &[&'a CssToken]) -> Vec<Vec<&'a CssToken>> {
    let mut out: Vec<Vec<&CssToken>> = vec![Vec::new()];
    let mut depth: i32 = 0;
    for &t in tokens {
        match t {
            CssToken::OpenParen | CssToken::Function(_) => {
                depth += 1;
                out.last_mut().expect("at least one").push(t);
            },
            CssToken::CloseParen => {
                depth -= 1;
                out.last_mut().expect("at least one").push(t);
            },
            CssToken::Comma if depth == 0 => {
                out.push(Vec::new());
            },
            _ => out.last_mut().expect("at least one").push(t),
        }
    }
    out
}

/// Split a function body by the top-level `/` (alpha separator used
/// by modern CSS color syntax like `rgb(255 0 0 / 50%)`).
fn split_top_level_slash<'a>(
    tokens: &[&'a CssToken],
) -> (Vec<&'a CssToken>, Option<Vec<&'a CssToken>>) {
    let mut depth: i32 = 0;
    for (i, &t) in tokens.iter().enumerate() {
        match t {
            CssToken::OpenParen | CssToken::Function(_) => depth += 1,
            CssToken::CloseParen => depth -= 1,
            CssToken::Slash if depth == 0 => {
                let before: Vec<&CssToken> = tokens[..i].to_vec();
                let after: Vec<&CssToken> = tokens[i + 1..].to_vec();
                return (before, Some(after));
            },
            _ => {},
        }
    }
    (tokens.to_vec(), None)
}

/// Parse an alpha token group (the RHS of a `/`). Accepts a single
/// number (0–1) or a percentage.
fn parse_alpha_component(tokens: &[&CssToken]) -> Option<u8> {
    for t in tokens {
        match t {
            CssToken::Number(n) => return Some((n.clamp(0.0, 1.0) * 255.0).round() as u8),
            CssToken::Percentage(p) => {
                return Some(((p / 100.0).clamp(0.0, 1.0) * 255.0).round() as u8);
            },
            _ => {},
        }
    }
    None
}

/// Parse an HSL / HSLA function. Accepts both modern
/// (`hsl(120 50% 50%)`) and legacy (`hsl(120, 50%, 50%)`) syntax,
/// with optional alpha via `/` or a trailing comma value.
fn parse_hsl_function(body: &[&CssToken]) -> Option<CssColor> {
    let (main, alpha_tokens) = split_top_level_slash(body);
    let groups = split_top_level_commas(&main);
    // Extract legacy trailing alpha (4th comma group) before flattening.
    let legacy_alpha = if groups.len() >= 4 {
        parse_alpha_component(&groups[3])
    } else {
        None
    };
    // Flatten the first 3 groups into a sequence of non-comma components.
    let components: Vec<&CssToken> = if groups.len() > 1 {
        groups.into_iter().take(3).flatten().collect()
    } else {
        main
    };
    let mut nums = Vec::new();
    for t in components {
        match t {
            CssToken::Number(n) => nums.push(*n),
            CssToken::Percentage(p) => nums.push(*p),
            CssToken::Dimension(n, unit) => {
                nums.push(hue_to_degrees(*n, unit));
            },
            _ => {},
        }
    }
    if nums.len() < 3 {
        return None;
    }
    let alpha = if let Some(ref tokens) = alpha_tokens {
        parse_alpha_component(tokens).unwrap_or(255)
    } else {
        legacy_alpha.unwrap_or(255)
    };
    let (r, g, b) = hsl_to_rgb(nums[0], nums[1], nums[2]);
    Some(CssColor::new(r, g, b, alpha))
}

/// Parse an `oklch(L C H [/ A])` function.
fn parse_oklch_function(body: &[&CssToken]) -> Option<CssColor> {
    let (main, alpha_tokens) = split_top_level_slash(body);
    let mut nums: [f32; 3] = [0.0; 3];
    let mut idx = 0;
    for t in &main {
        if idx >= 3 {
            break;
        }
        match t {
            CssToken::Number(n) => {
                nums[idx] = *n;
                idx += 1;
            },
            CssToken::Percentage(p) => {
                // L: 0% = 0.0, 100% = 1.0. C: 0% = 0.0, 100% = 0.4.
                nums[idx] = if idx == 0 { p / 100.0 } else { p / 100.0 * 0.4 };
                idx += 1;
            },
            CssToken::Dimension(n, unit) => {
                if idx == 2 {
                    nums[idx] = hue_to_degrees(*n, unit);
                    idx += 1;
                }
            },
            _ => {},
        }
    }
    if idx < 3 {
        return None;
    }
    let alpha = alpha_tokens
        .as_ref()
        .and_then(|t| parse_alpha_component(t))
        .unwrap_or(255);
    let (r, g, b) = oklch_to_srgb(nums[0], nums[1], nums[2]);
    Some(CssColor::new(r, g, b, alpha))
}

/// Parse an `oklab(L a b [/ A])` function.
fn parse_oklab_function(body: &[&CssToken]) -> Option<CssColor> {
    let (main, alpha_tokens) = split_top_level_slash(body);
    let mut nums: [f32; 3] = [0.0; 3];
    let mut idx = 0;
    for t in &main {
        if idx >= 3 {
            break;
        }
        match t {
            CssToken::Number(n) => {
                nums[idx] = *n;
                idx += 1;
            },
            CssToken::Percentage(p) => {
                // L: 0-100% maps to 0-1. a,b: ±100% maps to ±0.4.
                nums[idx] = if idx == 0 { p / 100.0 } else { p / 100.0 * 0.4 };
                idx += 1;
            },
            _ => {},
        }
    }
    if idx < 3 {
        return None;
    }
    let alpha = alpha_tokens
        .as_ref()
        .and_then(|t| parse_alpha_component(t))
        .unwrap_or(255);
    let (r, g, b) = oklab_to_srgb(nums[0], nums[1], nums[2]);
    Some(CssColor::new(r, g, b, alpha))
}

/// Parse `color(<colorspace> r g b [/ a])`. Supported spaces: `srgb`,
/// `srgb-linear`, `display-p3`. Other spaces fall back to `srgb`
/// interpretation.
fn parse_color_function(body: &[&CssToken]) -> Option<CssColor> {
    // First token should be a color space ident.
    let mut iter = body.iter();
    let space = loop {
        match iter.next()? {
            CssToken::Ident(s) => break s.to_ascii_lowercase(),
            CssToken::Whitespace => continue,
            _ => return None,
        }
    };
    // The rest of the tokens are the components + optional alpha.
    let rest: Vec<&CssToken> = iter.copied().collect();
    let (main, alpha_tokens) = split_top_level_slash(&rest);
    let mut nums: [f32; 3] = [0.0; 3];
    let mut idx = 0;
    for t in &main {
        if idx >= 3 {
            break;
        }
        match t {
            CssToken::Number(n) => {
                nums[idx] = *n;
                idx += 1;
            },
            CssToken::Percentage(p) => {
                nums[idx] = p / 100.0;
                idx += 1;
            },
            _ => {},
        }
    }
    if idx < 3 {
        return None;
    }
    let alpha = alpha_tokens
        .as_ref()
        .and_then(|t| parse_alpha_component(t))
        .unwrap_or(255);
    let (r, g, b) = match space.as_str() {
        "srgb" => (
            (nums[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (nums[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (nums[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        ),
        "srgb-linear" => (
            (linear_to_gamma_srgb(nums[0]).clamp(0.0, 1.0) * 255.0).round() as u8,
            (linear_to_gamma_srgb(nums[1]).clamp(0.0, 1.0) * 255.0).round() as u8,
            (linear_to_gamma_srgb(nums[2]).clamp(0.0, 1.0) * 255.0).round() as u8,
        ),
        "display-p3" => display_p3_to_srgb8(nums[0], nums[1], nums[2]),
        _ => (
            (nums[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (nums[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (nums[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        ),
    };
    Some(CssColor::new(r, g, b, alpha))
}

/// Parse `color-mix(in <space>, c1 [<pct>]?, c2 [<pct>]?)`. Only sRGB
/// interpolation is implemented; other color spaces fall back to sRGB
/// linear interpolation.
fn parse_color_mix_function(body: &[&CssToken]) -> Option<CssColor> {
    let args = split_top_level_commas(body);
    if args.len() < 3 {
        return None;
    }
    // First arg: `in <space>`.
    let space_arg = &args[0];
    let mut seen_in = false;
    let mut space = "srgb".to_string();
    for t in space_arg {
        match t {
            CssToken::Ident(s) if s.eq_ignore_ascii_case("in") => seen_in = true,
            CssToken::Ident(s) if seen_in => {
                space = s.to_ascii_lowercase();
                break;
            },
            _ => {},
        }
    }
    if !seen_in {
        return None;
    }
    let (c1, p1) = parse_color_mix_arg(&args[1])?;
    let (c2, p2) = parse_color_mix_arg(&args[2])?;
    // Normalise percentages. If neither has a percentage, 50/50.
    // If only one is given, the other is `100 - p`. If both are given,
    // normalise so they sum to 100 (spec says the result is scaled by
    // the sum when it's < 100, but we ignore that for simplicity).
    let (w1, w2) = match (p1, p2) {
        (None, None) => (0.5, 0.5),
        (Some(p), None) => {
            let w = (p / 100.0).clamp(0.0, 1.0);
            (w, 1.0 - w)
        },
        (None, Some(p)) => {
            let w = (p / 100.0).clamp(0.0, 1.0);
            (1.0 - w, w)
        },
        (Some(a), Some(b)) => {
            let sum = a + b;
            if sum == 0.0 {
                (0.5, 0.5)
            } else {
                (a / sum, b / sum)
            }
        },
    };
    // Interpolate in linear-sRGB for correctness (applies to both
    // `in srgb` and as a sensible fallback for `in oklch` etc.).
    let _ = space; // recorded but not yet used for space-specific mixing
    let (r, g, b, a) = mix_srgb_linear(c1, c2, w1, w2);
    Some(CssColor::new(r, g, b, a))
}

/// Parse a single color argument inside `color-mix()`, which is a
/// color possibly followed by a percentage.
fn parse_color_mix_arg(tokens: &[&CssToken]) -> Option<(CssColor, Option<f32>)> {
    // Only strip a trailing top-level percentage (not one nested inside
    // a function like `hsl(120, 50%, 50%)`).
    let mut pct: Option<f32> = None;
    let last_non_ws = tokens
        .iter()
        .enumerate()
        .rev()
        .find(|(_, t)| !matches!(t, CssToken::Whitespace));
    let end = if let Some((i, CssToken::Percentage(p))) = last_non_ws {
        pct = Some(*p);
        i
    } else {
        tokens.len()
    };
    let owned: Vec<CssToken> = tokens[..end].iter().copied().cloned().collect();
    let color = try_parse_color(&owned)?;
    Some((color, pct))
}

/// Parse `light-dark(light-color, dark-color)`. We always resolve to
/// the light-mode color since we don't yet track a color-scheme
/// context at parse time.
fn parse_light_dark_function(body: &[&CssToken]) -> Option<CssColor> {
    let args = split_top_level_commas(body);
    let first = args.first()?;
    let owned: Vec<CssToken> = first.iter().copied().cloned().collect();
    try_parse_color(&owned)
}

// -------------------------------------------------------------------
// Color-space conversions
// -------------------------------------------------------------------

/// Convert a hue expressed in the given unit to degrees in [0, 360).
fn hue_to_degrees(n: f32, unit: &str) -> f32 {
    let d = match unit.to_ascii_lowercase().as_str() {
        "deg" => n,
        "rad" => n.to_degrees(),
        "grad" => n * 0.9,
        "turn" => n * 360.0,
        _ => n,
    };
    let m = d % 360.0;
    if m < 0.0 { m + 360.0 } else { m }
}

/// Convert HSL (h in degrees, s/l in percent 0..100) to sRGB bytes.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;
    let s = (s / 100.0).clamp(0.0, 1.0);
    let l = (l / 100.0).clamp(0.0, 1.0);

    fn hue_to_channel(p: f32, q: f32, mut t: f32) -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    }

    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue_to_channel(p, q, h + 1.0 / 3.0),
            hue_to_channel(p, q, h),
            hue_to_channel(p, q, h - 1.0 / 3.0),
        )
    };
    (
        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Convert OKLCH (L in 0..1, C >= 0, H in degrees) to gamma-encoded
/// sRGB bytes. Out-of-gamut results are clamped per-channel.
fn oklch_to_srgb(l: f32, c: f32, h_deg: f32) -> (u8, u8, u8) {
    let h = h_deg.to_radians();
    let a = c * h.cos();
    let b = c * h.sin();
    oklab_to_srgb(l, a, b)
}

/// Convert OKLab (L in 0..1, a,b around 0) to gamma-encoded sRGB bytes.
/// See <https://bottosson.github.io/posts/oklab/>.
fn oklab_to_srgb(l: f32, a: f32, b: f32) -> (u8, u8, u8) {
    // OKLab -> LMS'. Coefficients are truncated to f32 precision
    // (see <https://bottosson.github.io/posts/oklab/> for the full
    // double-precision values).
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    // LMS' -> LMS (cube).
    let lms_l = l_ * l_ * l_;
    let lms_m = m_ * m_ * m_;
    let lms_s = s_ * s_ * s_;
    // LMS -> linear sRGB.
    let r_lin = 4.076_742 * lms_l - 3.307_711_6 * lms_m + 0.230_969_94 * lms_s;
    let g_lin = -1.268_438 * lms_l + 2.609_757_4 * lms_m - 0.341_319_38 * lms_s;
    let b_lin = -0.004_196_086_5 * lms_l - 0.703_418_6 * lms_m + 1.707_614_7 * lms_s;
    (
        (linear_to_gamma_srgb(r_lin).clamp(0.0, 1.0) * 255.0).round() as u8,
        (linear_to_gamma_srgb(g_lin).clamp(0.0, 1.0) * 255.0).round() as u8,
        (linear_to_gamma_srgb(b_lin).clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Convert linear-light sRGB in [0, 1] to gamma-encoded sRGB in [0, 1].
fn linear_to_gamma_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.abs().powf(1.0 / 2.4) * c.signum() - 0.055 * c.signum()
    }
}

/// Convert gamma-encoded sRGB in [0, 1] to linear-light sRGB.
fn gamma_to_linear_srgb(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert `color(display-p3 r g b)` to sRGB bytes. Inputs are gamma-encoded
/// (same transfer curve as sRGB per CSS Color 4), so we linearize first,
/// apply the Display-P3-to-linear-sRGB matrix, then gamma-encode back.
fn display_p3_to_srgb8(r: f32, g: f32, b: f32) -> (u8, u8, u8) {
    let lr = gamma_to_linear_srgb(r);
    let lg = gamma_to_linear_srgb(g);
    let lb = gamma_to_linear_srgb(b);
    // Display-P3 → linear sRGB matrix (D65).
    let sr = 1.2249401 * lr - 0.2249404 * lg + 0.0000000 * lb;
    let sg = -0.0420569 * lr + 1.0420571 * lg + 0.0000000 * lb;
    let sb = -0.0196376 * lr - 0.0786361 * lg + 1.0982737 * lb;
    (
        (linear_to_gamma_srgb(sr).clamp(0.0, 1.0) * 255.0).round() as u8,
        (linear_to_gamma_srgb(sg).clamp(0.0, 1.0) * 255.0).round() as u8,
        (linear_to_gamma_srgb(sb).clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Interpolate two CssColors in linear sRGB with the given weights,
/// then gamma-encode back.
fn mix_srgb_linear(c1: CssColor, c2: CssColor, w1: f32, w2: f32) -> (u8, u8, u8, u8) {
    let lin = |v: u8| gamma_to_linear_srgb(v as f32 / 255.0);
    let r = lin(c1.r) * w1 + lin(c2.r) * w2;
    let g = lin(c1.g) * w1 + lin(c2.g) * w2;
    let b = lin(c1.b) * w1 + lin(c2.b) * w2;
    let a = (c1.a as f32 / 255.0) * w1 + (c2.a as f32 / 255.0) * w2;
    (
        (linear_to_gamma_srgb(r).clamp(0.0, 1.0) * 255.0).round() as u8,
        (linear_to_gamma_srgb(g).clamp(0.0, 1.0) * 255.0).round() as u8,
        (linear_to_gamma_srgb(b).clamp(0.0, 1.0) * 255.0).round() as u8,
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

pub(crate) fn parse_hex_color(hex: &str) -> Option<CssColor> {
    let hex = hex.trim_start_matches('#');
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        3 => {
            let r = hex_digit(hex.as_bytes()[0])?;
            let g = hex_digit(hex.as_bytes()[1])?;
            let b = hex_digit(hex.as_bytes()[2])?;
            Some(CssColor::new(r << 4 | r, g << 4 | g, b << 4 | b, 255))
        },
        4 => {
            let r = hex_digit(hex.as_bytes()[0])?;
            let g = hex_digit(hex.as_bytes()[1])?;
            let b = hex_digit(hex.as_bytes()[2])?;
            let a = hex_digit(hex.as_bytes()[3])?;
            Some(CssColor::new(
                r << 4 | r,
                g << 4 | g,
                b << 4 | b,
                a << 4 | a,
            ))
        },
        6 => {
            let r = hex_byte(&hex[0..2])?;
            let g = hex_byte(&hex[2..4])?;
            let b = hex_byte(&hex[4..6])?;
            Some(CssColor::new(r, g, b, 255))
        },
        8 => {
            let r = hex_byte(&hex[0..2])?;
            let g = hex_byte(&hex[2..4])?;
            let b = hex_byte(&hex[4..6])?;
            let a = hex_byte(&hex[6..8])?;
            Some(CssColor::new(r, g, b, a))
        },
        _ => None,
    }
}

pub(crate) fn hex_digit(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn hex_byte(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}

pub(crate) fn parse_rgb_function(tokens: &[&CssToken]) -> Option<CssColor> {
    let numbers: Vec<f32> = tokens
        .iter()
        .filter_map(|t| match t {
            CssToken::Number(n) => Some(*n),
            _ => None,
        })
        .collect();
    if numbers.len() >= 3 {
        let r = numbers[0].clamp(0.0, 255.0) as u8;
        let g = numbers[1].clamp(0.0, 255.0) as u8;
        let b = numbers[2].clamp(0.0, 255.0) as u8;
        let a = if numbers.len() >= 4 {
            (numbers[3].clamp(0.0, 1.0) * 255.0) as u8
        } else {
            255
        };
        Some(CssColor::new(r, g, b, a))
    } else {
        None
    }
}

pub(crate) fn named_color(name: &str) -> Option<CssColor> {
    match name.to_ascii_lowercase().as_str() {
        "black" => Some(CssColor::new(0, 0, 0, 255)),
        "white" => Some(CssColor::new(255, 255, 255, 255)),
        "red" => Some(CssColor::new(255, 0, 0, 255)),
        "green" => Some(CssColor::new(0, 128, 0, 255)),
        "blue" => Some(CssColor::new(0, 0, 255, 255)),
        "yellow" => Some(CssColor::new(255, 255, 0, 255)),
        "cyan" | "aqua" => Some(CssColor::new(0, 255, 255, 255)),
        "magenta" | "fuchsia" => Some(CssColor::new(255, 0, 255, 255)),
        "orange" => Some(CssColor::new(255, 165, 0, 255)),
        "purple" => Some(CssColor::new(128, 0, 128, 255)),
        "gray" | "grey" => Some(CssColor::new(128, 128, 128, 255)),
        "lime" => Some(CssColor::new(0, 255, 0, 255)),
        "navy" => Some(CssColor::new(0, 0, 128, 255)),
        "teal" => Some(CssColor::new(0, 128, 128, 255)),
        "maroon" => Some(CssColor::new(128, 0, 0, 255)),
        "olive" => Some(CssColor::new(128, 128, 0, 255)),
        "silver" => Some(CssColor::new(192, 192, 192, 255)),
        "transparent" => Some(CssColor::new(0, 0, 0, 0)),
        "pink" => Some(CssColor::new(255, 192, 203, 255)),
        "brown" => Some(CssColor::new(165, 42, 42, 255)),
        "coral" => Some(CssColor::new(255, 127, 80, 255)),
        "gold" => Some(CssColor::new(255, 215, 0, 255)),
        _ => None,
    }
}

// -------------------------------------------------------------------
// Media query evaluation
// -------------------------------------------------------------------

/// Viewport dimensions and preferences for media query evaluation.
#[derive(Debug, Clone, Copy)]
pub struct MediaViewport {
    /// Viewport width in CSS pixels.
    pub width: f32,
    /// Viewport height in CSS pixels.
    pub height: f32,
    /// Whether the user prefers a dark color scheme.
    pub dark_mode: bool,
    /// Whether the user prefers reduced motion.
    pub prefers_reduced_motion: bool,
    /// Whether the device supports hover interactions.
    pub hover: bool,
    /// Primary pointing device type: "fine", "coarse", or "none".
    pub pointer: &'static str,
}

impl MediaViewport {
    /// Default PSP viewport (480x272, light mode).
    pub(crate) const DEFAULT: Self = Self {
        width: 480.0,
        height: 272.0,
        dark_mode: false,
        prefers_reduced_motion: false,
        hover: true,
        pointer: "fine",
    };
}

impl Default for MediaViewport {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Evaluate a simplified media query against the default OASIS viewport.
///
/// Convenience wrapper around [`eval_media_query_with_viewport`] using
/// the default 480x272 PSP viewport.
#[cfg(test)]
pub(crate) fn eval_media_query(query: &str) -> bool {
    eval_media_query_with_viewport(query, MediaViewport::DEFAULT)
}

/// Evaluate a media query against a specific viewport size.
pub(crate) fn eval_media_query_with_viewport(query: &str, viewport: MediaViewport) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }

    // Comma-separated: any match means true.
    for part in query.split(',') {
        if eval_single_media_query(part.trim(), viewport) {
            return true;
        }
    }
    false
}

pub(crate) fn eval_single_media_query(query: &str, viewport: MediaViewport) -> bool {
    let query = query.trim();
    if query.is_empty() || query == "all" || query == "screen" {
        return true;
    }
    if query == "print" || query == "not screen" {
        return false;
    }
    if let Some(rest) = query.strip_prefix("not ") {
        return !eval_single_media_query(rest, viewport);
    }
    // Handle compound conditions like "screen and (max-width: 600px)".
    // Split on " and " and evaluate each part.
    let parts: Vec<&str> = query.split(" and ").collect();
    for part in &parts {
        let p = part.trim();
        // "only" is a CSS3 modifier for backwards compat; strip it.
        let p = p.strip_prefix("only ").unwrap_or(p);
        if p == "screen" || p == "all" || p.is_empty() {
            continue;
        }
        if p == "print" {
            return false;
        }
        // Parenthesized feature: (max-width: 600px), (min-width: 320px), etc.
        let inner = p.trim_start_matches('(').trim_end_matches(')').trim();
        if let Some(rest) = inner.strip_prefix("max-width:") {
            let px = parse_px_value(rest.trim());
            if viewport.width > px {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("min-width:") {
            let px = parse_px_value(rest.trim());
            if viewport.width < px {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("max-height:") {
            let px = parse_px_value(rest.trim());
            if viewport.height > px {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("min-height:") {
            let px = parse_px_value(rest.trim());
            if viewport.height < px {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("prefers-color-scheme:") {
            let scheme = rest.trim();
            if scheme == "dark" && !viewport.dark_mode {
                return false;
            }
            if scheme == "light" && viewport.dark_mode {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("orientation:") {
            let orient = rest.trim();
            if orient == "portrait" && viewport.width >= viewport.height {
                return false;
            }
            if orient == "landscape" && viewport.height > viewport.width {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("prefers-reduced-motion:") {
            let pref = rest.trim();
            if pref == "reduce" && !viewport.prefers_reduced_motion {
                return false;
            }
            if pref == "no-preference" && viewport.prefers_reduced_motion {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("hover:") {
            let val = rest.trim();
            if val == "hover" && !viewport.hover {
                return false;
            }
            if val == "none" && viewport.hover {
                return false;
            }
        } else if let Some(rest) = inner.strip_prefix("pointer:") {
            let val = rest.trim();
            if val != viewport.pointer {
                return false;
            }
        } else {
            // Unknown features: treat as NOT matching (safe default).
            return false;
        }
    }
    true
}

/// Parse a pixel value like "600px" or "600" from a media query.
pub(crate) fn parse_px_value(s: &str) -> f32 {
    let s = s.trim().trim_end_matches("px");
    s.parse::<f32>().unwrap_or(0.0)
}

// -------------------------------------------------------------------
// CSS text reconstruction
// -------------------------------------------------------------------

/// Reconstruct CSS text from a token stream.
///
/// Used to store custom property values and `var()` fallback text as
/// raw strings that can be re-tokenized later during cascade resolution.
pub(crate) fn tokens_to_css_text(tokens: &[CssToken]) -> String {
    let mut out = String::new();
    for tok in tokens {
        match tok {
            CssToken::Ident(s) => out.push_str(s),
            CssToken::Hash(s) => {
                out.push('#');
                out.push_str(s);
            },
            CssToken::String(s) => {
                out.push('"');
                out.push_str(s);
                out.push('"');
            },
            CssToken::Number(n) => out.push_str(&format!("{n}")),
            CssToken::Percentage(n) => {
                out.push_str(&format!("{n}"));
                out.push('%');
            },
            CssToken::Dimension(n, u) => {
                out.push_str(&format!("{n}"));
                out.push_str(u);
            },
            CssToken::Colon => out.push(':'),
            CssToken::Semicolon => out.push(';'),
            CssToken::Comma => out.push(','),
            CssToken::OpenBrace => out.push('{'),
            CssToken::CloseBrace => out.push('}'),
            CssToken::OpenParen => out.push('('),
            CssToken::CloseParen => out.push(')'),
            CssToken::OpenBracket => out.push('['),
            CssToken::CloseBracket => out.push(']'),
            CssToken::Dot => out.push('.'),
            CssToken::Greater => out.push('>'),
            CssToken::Plus => out.push('+'),
            CssToken::Star => out.push('*'),
            CssToken::Slash => out.push('/'),
            CssToken::Delim(c) => out.push(*c),
            CssToken::Whitespace => out.push(' '),
            CssToken::AtKeyword(s) => {
                out.push('@');
                out.push_str(s);
            },
            CssToken::Function(s) => {
                out.push_str(s);
                out.push('(');
            },
            CssToken::Eof => {},
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // hex_digit
    // ---------------------------------------------------------------

    #[test]
    fn hex_digit_decimal() {
        for (ch, expected) in (b'0'..=b'9').zip(0u8..=9) {
            assert_eq!(hex_digit(ch), Some(expected));
        }
    }

    #[test]
    fn hex_digit_lowercase() {
        for (ch, expected) in (b'a'..=b'f').zip(10u8..=15) {
            assert_eq!(hex_digit(ch), Some(expected));
        }
    }

    #[test]
    fn hex_digit_uppercase() {
        for (ch, expected) in (b'A'..=b'F').zip(10u8..=15) {
            assert_eq!(hex_digit(ch), Some(expected));
        }
    }

    #[test]
    fn hex_digit_invalid() {
        assert_eq!(hex_digit(b'g'), None);
        assert_eq!(hex_digit(b'G'), None);
        assert_eq!(hex_digit(b' '), None);
        assert_eq!(hex_digit(b'z'), None);
    }

    // ---------------------------------------------------------------
    // hex_byte
    // ---------------------------------------------------------------

    #[test]
    fn hex_byte_valid() {
        assert_eq!(hex_byte("ff"), Some(255));
        assert_eq!(hex_byte("00"), Some(0));
        assert_eq!(hex_byte("7f"), Some(127));
        assert_eq!(hex_byte("FF"), Some(255));
        assert_eq!(hex_byte("a0"), Some(160));
    }

    #[test]
    fn hex_byte_invalid() {
        assert_eq!(hex_byte("zz"), None);
        assert_eq!(hex_byte(""), None);
        assert_eq!(hex_byte("gg"), None);
    }

    // ---------------------------------------------------------------
    // parse_hex_color
    // ---------------------------------------------------------------

    #[test]
    fn hex_color_3_digit() {
        let c = parse_hex_color("f00").unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 255));
    }

    #[test]
    fn hex_color_3_digit_with_hash() {
        let c = parse_hex_color("#abc").unwrap();
        assert_eq!(c, CssColor::new(0xaa, 0xbb, 0xcc, 255));
    }

    #[test]
    fn hex_color_4_digit_rgba() {
        let c = parse_hex_color("f008").unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 0x88));
    }

    #[test]
    fn hex_color_6_digit() {
        let c = parse_hex_color("ff8000").unwrap();
        assert_eq!(c, CssColor::new(255, 128, 0, 255));
    }

    #[test]
    fn hex_color_6_digit_with_hash() {
        let c = parse_hex_color("#336699").unwrap();
        assert_eq!(c, CssColor::new(0x33, 0x66, 0x99, 255));
    }

    #[test]
    fn hex_color_8_digit_rgba() {
        let c = parse_hex_color("ff000080").unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 128));
    }

    #[test]
    fn hex_color_invalid_length() {
        assert!(parse_hex_color("f").is_none());
        assert!(parse_hex_color("ff").is_none());
        assert!(parse_hex_color("fffff").is_none());
        assert!(parse_hex_color("fffffff").is_none());
        assert!(parse_hex_color("fffffffff").is_none());
    }

    #[test]
    fn hex_color_invalid_chars() {
        assert!(parse_hex_color("xyz").is_none());
        assert!(parse_hex_color("gggggg").is_none());
    }

    // ---------------------------------------------------------------
    // named_color
    // ---------------------------------------------------------------

    #[test]
    fn named_color_basic() {
        assert_eq!(named_color("black"), Some(CssColor::new(0, 0, 0, 255)));
        assert_eq!(
            named_color("white"),
            Some(CssColor::new(255, 255, 255, 255))
        );
        assert_eq!(named_color("red"), Some(CssColor::new(255, 0, 0, 255)));
    }

    #[test]
    fn named_color_case_insensitive() {
        assert_eq!(named_color("BLACK"), Some(CssColor::new(0, 0, 0, 255)));
        assert_eq!(named_color("Red"), Some(CssColor::new(255, 0, 0, 255)));
    }

    #[test]
    fn named_color_aliases() {
        // cyan == aqua
        assert_eq!(named_color("cyan"), named_color("aqua"));
        // magenta == fuchsia
        assert_eq!(named_color("magenta"), named_color("fuchsia"));
        // gray == grey
        assert_eq!(named_color("gray"), named_color("grey"));
    }

    #[test]
    fn named_color_transparent() {
        assert_eq!(named_color("transparent"), Some(CssColor::new(0, 0, 0, 0)));
    }

    #[test]
    fn named_color_unknown() {
        assert!(named_color("chartreuse").is_none());
        assert!(named_color("").is_none());
        assert!(named_color("notacolor").is_none());
    }

    // ---------------------------------------------------------------
    // parse_rgb_function
    // ---------------------------------------------------------------

    #[test]
    fn rgb_function_three_args() {
        let tokens = [
            CssToken::Number(100.0),
            CssToken::Comma,
            CssToken::Number(200.0),
            CssToken::Comma,
            CssToken::Number(50.0),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        let c = parse_rgb_function(&refs).unwrap();
        assert_eq!(c, CssColor::new(100, 200, 50, 255));
    }

    #[test]
    fn rgba_function_four_args() {
        let tokens = [
            CssToken::Number(255.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(128.0),
            CssToken::Comma,
            CssToken::Number(0.5),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        let c = parse_rgb_function(&refs).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 128, 127));
    }

    #[test]
    fn rgb_function_clamped() {
        let tokens = [
            CssToken::Number(300.0),
            CssToken::Comma,
            CssToken::Number(-10.0),
            CssToken::Comma,
            CssToken::Number(128.0),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        let c = parse_rgb_function(&refs).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 128, 255));
    }

    #[test]
    fn rgb_function_too_few_args() {
        let tokens = [
            CssToken::Number(100.0),
            CssToken::Comma,
            CssToken::Number(200.0),
        ];
        let refs: Vec<&CssToken> = tokens.iter().collect();
        assert!(parse_rgb_function(&refs).is_none());
    }

    #[test]
    fn rgb_function_empty() {
        let refs: Vec<&CssToken> = vec![];
        assert!(parse_rgb_function(&refs).is_none());
    }

    // ---------------------------------------------------------------
    // try_parse_color
    // ---------------------------------------------------------------

    #[test]
    fn try_parse_color_hex() {
        let tokens = [CssToken::Hash("ff0000".into())];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 255));
    }

    #[test]
    fn try_parse_color_named() {
        let tokens = [CssToken::Ident("blue".into())];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(0, 0, 255, 255));
    }

    #[test]
    fn try_parse_color_rgb_function() {
        let tokens = [
            CssToken::Function("rgb".into()),
            CssToken::Number(10.0),
            CssToken::Comma,
            CssToken::Number(20.0),
            CssToken::Comma,
            CssToken::Number(30.0),
            CssToken::CloseParen,
        ];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(10, 20, 30, 255));
    }

    #[test]
    fn try_parse_color_rgba_function() {
        let tokens = [
            CssToken::Function("RGBA".into()),
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::CloseParen,
        ];
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(0, 0, 0, 0));
    }

    #[test]
    fn try_parse_color_empty() {
        assert!(try_parse_color(&[]).is_none());
    }

    #[test]
    fn try_parse_color_whitespace_only() {
        let tokens = [CssToken::Whitespace, CssToken::Whitespace];
        assert!(try_parse_color(&tokens).is_none());
    }

    #[test]
    fn try_parse_color_unknown_function() {
        let tokens = [
            CssToken::Function("cmyk".into()),
            CssToken::Number(0.0),
            CssToken::CloseParen,
        ];
        assert!(try_parse_color(&tokens).is_none());
    }

    // ---------------------------------------------------------------
    // Color functions (hsl/oklch/oklab/color/color-mix/light-dark)
    // ---------------------------------------------------------------

    /// Lex a CSS value string to a token slice suitable for
    /// `try_parse_color`. Keeps whitespace out to match how the
    /// parser feeds us arguments.
    fn lex(css: &str) -> Vec<CssToken> {
        super::super::tokenizer::CssTokenizer::new(css).tokenize()
    }

    #[test]
    fn hsl_basic_red() {
        let tokens = lex("hsl(0, 100%, 50%)");
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 255));
    }

    #[test]
    fn hsl_modern_syntax_green() {
        let tokens = lex("hsl(120 100% 50%)");
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(0, 255, 0, 255));
    }

    #[test]
    fn hsla_with_slash_alpha() {
        let tokens = lex("hsl(240 100% 50% / 50%)");
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(0, 0, 255, 128));
    }

    #[test]
    fn oklch_red_approx() {
        // `oklch(62.8% 0.258 29.23)` is approximately CSS `red`.
        let tokens = lex("oklch(62.8% 0.258 29.23)");
        let c = try_parse_color(&tokens).expect("oklch red parses");
        // Allow a small tolerance — the conversion and rounding can
        // drift by a few least-significant bits.
        assert!(
            (c.r as i16 - 255).abs() <= 2,
            "red channel should be near 255, got {}",
            c.r
        );
        assert!(c.g <= 10, "green channel should be near 0, got {}", c.g);
        assert!(c.b <= 10, "blue channel should be near 0, got {}", c.b);
    }

    #[test]
    fn oklch_preserves_alpha() {
        let tokens = lex("oklch(0.5 0.1 180 / 0.5)");
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c.a, 128);
    }

    #[test]
    fn oklab_parses() {
        let tokens = lex("oklab(0.5 0.1 -0.1)");
        assert!(try_parse_color(&tokens).is_some());
    }

    #[test]
    fn color_srgb_function() {
        let tokens = lex("color(srgb 1 0 0)");
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 255));
    }

    #[test]
    fn color_display_p3_parses() {
        let tokens = lex("color(display-p3 1 0 0)");
        let c = try_parse_color(&tokens).expect("display-p3 parses");
        // Display-P3 red is out-of-gamut in sRGB; clamping to the
        // boundary gives pure red.
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn color_display_p3_mid_range_gamma_decodes() {
        // Mid-range values exercise the gamma-decode step before the
        // P3→sRGB matrix. Without linearizing first, 0.5 would map
        // to ~201 instead of the correct ~139.
        let tokens = lex("color(display-p3 0.5 0 0)");
        let c = try_parse_color(&tokens).expect("display-p3 mid-range");
        assert!(
            (130..=148).contains(&c.r),
            "expected ~139 for display-p3 0.5 red, got {}",
            c.r
        );
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn color_mix_srgb_50_50() {
        // Mix red and blue 50/50 in sRGB; linear-space interpolation
        // gives roughly (128, 0, 128) after gamma encoding.
        let tokens = lex("color-mix(in srgb, red, blue)");
        let c = try_parse_color(&tokens).unwrap();
        // Linear midpoint of gamma-red and gamma-blue is approximately
        // 188 (not 128 which would be a naive gamma-space midpoint).
        assert!((c.r as i16 - 188).abs() <= 5, "red ~188, got {}", c.r);
        assert_eq!(c.g, 0);
        assert!((c.b as i16 - 188).abs() <= 5, "blue ~188, got {}", c.b);
    }

    #[test]
    fn color_mix_srgb_weighted() {
        // Red 20% + blue 80% → mostly blue.
        let tokens = lex("color-mix(in srgb, red 20%, blue 80%)");
        let c = try_parse_color(&tokens).unwrap();
        assert!(c.b > c.r, "blue should dominate, got r={} b={}", c.r, c.b);
    }

    #[test]
    fn color_mix_clamps_out_of_range_percentage() {
        let tokens = lex("color-mix(in srgb, red 150%, blue)");
        let c = try_parse_color(&tokens).unwrap();
        // 150% is clamped to 100%, so result should be pure red.
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn light_dark_picks_light() {
        // We don't yet track a color-scheme context, so light-dark()
        // resolves to its first argument.
        let tokens = lex("light-dark(red, blue)");
        let c = try_parse_color(&tokens).unwrap();
        assert_eq!(c, CssColor::new(255, 0, 0, 255));
    }

    // ---------------------------------------------------------------
    // parse_unit
    // ---------------------------------------------------------------

    #[test]
    fn parse_unit_known() {
        assert_eq!(parse_unit("px"), Some(LengthUnit::Px));
        assert_eq!(parse_unit("em"), Some(LengthUnit::Em));
        assert_eq!(parse_unit("rem"), Some(LengthUnit::Rem));
        assert_eq!(parse_unit("pt"), Some(LengthUnit::Pt));
    }

    #[test]
    fn parse_unit_case_insensitive() {
        assert_eq!(parse_unit("PX"), Some(LengthUnit::Px));
        assert_eq!(parse_unit("Em"), Some(LengthUnit::Em));
        assert_eq!(parse_unit("REM"), Some(LengthUnit::Rem));
    }

    #[test]
    fn parse_unit_unknown() {
        assert_eq!(parse_unit("vh"), None);
        assert_eq!(parse_unit("vw"), None);
        assert_eq!(parse_unit(""), None);
        assert_eq!(parse_unit("cm"), None);
    }

    // ---------------------------------------------------------------
    // is_color_property
    // ---------------------------------------------------------------

    #[test]
    fn is_color_property_true() {
        assert!(is_color_property("color"));
        assert!(is_color_property("background-color"));
        assert!(is_color_property("border-color"));
        assert!(is_color_property("border-top-color"));
        assert!(is_color_property("border-right-color"));
        assert!(is_color_property("border-bottom-color"));
        assert!(is_color_property("border-left-color"));
        assert!(is_color_property("outline-color"));
    }

    #[test]
    fn is_color_property_false() {
        assert!(!is_color_property("background"));
        assert!(!is_color_property("border"));
        assert!(!is_color_property("font-size"));
        assert!(!is_color_property(""));
        assert!(!is_color_property("Color"));
    }

    // ---------------------------------------------------------------
    // parse_font_weight
    // ---------------------------------------------------------------

    #[test]
    fn font_weight_bold() {
        let tokens = [CssToken::Ident("bold".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(700.0));
    }

    #[test]
    fn font_weight_normal() {
        let tokens = [CssToken::Ident("normal".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(400.0));
    }

    #[test]
    fn font_weight_lighter() {
        let tokens = [CssToken::Ident("lighter".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(100.0));
    }

    #[test]
    fn font_weight_bolder() {
        let tokens = [CssToken::Ident("bolder".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(900.0));
    }

    #[test]
    fn font_weight_case_insensitive() {
        let tokens = [CssToken::Ident("BOLD".into())];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(700.0));
    }

    #[test]
    fn font_weight_numeric() {
        let tokens = [CssToken::Number(600.0)];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(600.0));
    }

    #[test]
    fn font_weight_unknown_keyword() {
        let tokens = [CssToken::Ident("fancy".into())];
        assert_eq!(
            parse_font_weight(&tokens),
            CssValue::Keyword("fancy".into())
        );
    }

    #[test]
    fn font_weight_with_whitespace() {
        let tokens = [
            CssToken::Whitespace,
            CssToken::Ident("bold".into()),
            CssToken::Whitespace,
        ];
        assert_eq!(parse_font_weight(&tokens), CssValue::Number(700.0));
    }

    // ---------------------------------------------------------------
    // parse_px_value
    // ---------------------------------------------------------------

    #[test]
    fn parse_px_value_with_unit() {
        assert!((parse_px_value("600px") - 600.0).abs() < f32::EPSILON);
        assert!((parse_px_value("320px") - 320.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_px_value_without_unit() {
        assert!((parse_px_value("480") - 480.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_px_value_with_whitespace() {
        assert!((parse_px_value("  100px  ") - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_px_value_invalid() {
        assert!((parse_px_value("abc") - 0.0).abs() < f32::EPSILON);
        assert!((parse_px_value("") - 0.0).abs() < f32::EPSILON);
    }

    // ---------------------------------------------------------------
    // eval_media_query / eval_single_media_query
    // ---------------------------------------------------------------

    #[test]
    fn media_query_empty_is_true() {
        assert!(eval_media_query(""));
        assert!(eval_media_query("  "));
    }

    #[test]
    fn media_query_screen_and_all() {
        assert!(eval_media_query("screen"));
        assert!(eval_media_query("all"));
    }

    #[test]
    fn media_query_print_is_false() {
        assert!(!eval_media_query("print"));
    }

    #[test]
    fn media_query_not_print() {
        assert!(eval_media_query("not print"));
    }

    #[test]
    fn media_query_not_screen() {
        assert!(!eval_media_query("not screen"));
    }

    #[test]
    fn media_query_max_width_pass() {
        // viewport = 480, max-width 600 => true
        assert!(eval_media_query("(max-width: 600px)"));
    }

    #[test]
    fn media_query_max_width_fail() {
        // viewport = 480, max-width 200 => false
        assert!(!eval_media_query("(max-width: 200px)"));
    }

    #[test]
    fn media_query_min_width_pass() {
        // viewport = 480, min-width 320 => true
        assert!(eval_media_query("(min-width: 320px)"));
    }

    #[test]
    fn media_query_min_width_fail() {
        // viewport = 480, min-width 800 => false
        assert!(!eval_media_query("(min-width: 800px)"));
    }

    #[test]
    fn media_query_compound() {
        assert!(eval_media_query("screen and (max-width: 600px)"));
        assert!(!eval_media_query("print and (max-width: 600px)"));
    }

    #[test]
    fn media_query_comma_separated() {
        // "print, screen" => print=false OR screen=true => true
        assert!(eval_media_query("print, screen"));
        // "print, not screen" => both false
        assert!(!eval_media_query("print, not screen"));
    }

    #[test]
    fn media_query_only_modifier() {
        assert!(eval_media_query("only screen"));
    }

    #[test]
    fn media_query_prefers_color_scheme() {
        assert!(eval_media_query("(prefers-color-scheme: light)"));
        assert!(!eval_media_query("(prefers-color-scheme: dark)"));
    }

    #[test]
    fn media_query_unknown_feature() {
        // `hover` is now a known feature, so use a truly unknown one.
        assert!(!eval_media_query("(scan: interlace)"));
    }

    #[test]
    fn media_query_hover_feature() {
        // Default viewport has hover: true.
        assert!(eval_media_query("(hover: hover)"));
        assert!(!eval_media_query("(hover: none)"));
    }

    #[test]
    fn media_query_pointer_feature() {
        assert!(eval_media_query("(pointer: fine)"));
        assert!(!eval_media_query("(pointer: coarse)"));
    }

    #[test]
    fn media_query_prefers_reduced_motion() {
        assert!(eval_media_query("(prefers-reduced-motion: no-preference)"));
        assert!(!eval_media_query("(prefers-reduced-motion: reduce)"));
    }

    #[test]
    fn eval_single_media_query_with_viewport() {
        let vp = MediaViewport {
            width: 800.0,
            height: 600.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query("(max-width: 1024px)", vp));
        assert!(!eval_single_media_query("(max-width: 600px)", vp));
        assert!(eval_single_media_query("(min-width: 600px)", vp));
        assert!(!eval_single_media_query("(min-width: 1024px)", vp));
    }

    #[test]
    fn media_query_min_height_pass() {
        let vp = MediaViewport {
            width: 480.0,
            height: 600.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query("(min-height: 400px)", vp));
    }

    #[test]
    fn media_query_min_height_fail() {
        let vp = MediaViewport {
            width: 480.0,
            height: 300.0,
            ..MediaViewport::DEFAULT
        };
        assert!(!eval_single_media_query("(min-height: 400px)", vp));
    }

    #[test]
    fn media_query_max_height_pass() {
        let vp = MediaViewport {
            width: 480.0,
            height: 300.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query("(max-height: 400px)", vp));
    }

    #[test]
    fn media_query_max_height_fail() {
        let vp = MediaViewport {
            width: 480.0,
            height: 600.0,
            ..MediaViewport::DEFAULT
        };
        assert!(!eval_single_media_query("(max-height: 400px)", vp));
    }

    #[test]
    fn media_query_compound_with_height() {
        let vp = MediaViewport {
            width: 800.0,
            height: 600.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query(
            "screen and (min-width: 480px) and (min-height: 400px)",
            vp
        ));
        assert!(!eval_single_media_query(
            "screen and (min-width: 480px) and (min-height: 800px)",
            vp
        ));
    }

    #[test]
    fn media_query_viewport_with_viewport_fn() {
        let vp = MediaViewport {
            width: 1024.0,
            height: 768.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_media_query_with_viewport("(min-width: 800px)", vp));
        assert!(!eval_media_query_with_viewport("(min-width: 1200px)", vp));
        assert!(eval_media_query_with_viewport("(max-height: 800px)", vp));
        assert!(!eval_media_query_with_viewport("(max-height: 700px)", vp));
    }

    #[test]
    fn media_query_prefers_color_scheme_dark_mode() {
        let vp = MediaViewport {
            width: 480.0,
            height: 272.0,
            dark_mode: true,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query("(prefers-color-scheme: dark)", vp));
        assert!(!eval_single_media_query(
            "(prefers-color-scheme: light)",
            vp
        ));
    }

    #[test]
    fn media_query_prefers_color_scheme_light_mode() {
        let vp = MediaViewport {
            width: 480.0,
            height: 272.0,
            ..MediaViewport::DEFAULT
        };
        assert!(!eval_single_media_query("(prefers-color-scheme: dark)", vp));
        assert!(eval_single_media_query("(prefers-color-scheme: light)", vp));
    }

    #[test]
    fn media_query_orientation_landscape() {
        let vp = MediaViewport {
            width: 800.0,
            height: 600.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query("(orientation: landscape)", vp));
        assert!(!eval_single_media_query("(orientation: portrait)", vp));
    }

    #[test]
    fn media_query_orientation_portrait() {
        let vp = MediaViewport {
            width: 600.0,
            height: 800.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query("(orientation: portrait)", vp));
        assert!(!eval_single_media_query("(orientation: landscape)", vp));
    }

    #[test]
    fn media_query_orientation_square_is_landscape() {
        // When width == height, orientation is landscape (width >= height).
        let vp = MediaViewport {
            width: 500.0,
            height: 500.0,
            ..MediaViewport::DEFAULT
        };
        assert!(eval_single_media_query("(orientation: landscape)", vp));
        assert!(!eval_single_media_query("(orientation: portrait)", vp));
    }

    // ---------------------------------------------------------------
    // tokens_to_css_text
    // ---------------------------------------------------------------

    #[test]
    fn css_text_empty() {
        assert_eq!(tokens_to_css_text(&[]), "");
    }

    #[test]
    fn css_text_ident() {
        let tokens = [CssToken::Ident("auto".into())];
        assert_eq!(tokens_to_css_text(&tokens), "auto");
    }

    #[test]
    fn css_text_hash() {
        let tokens = [CssToken::Hash("ff0000".into())];
        assert_eq!(tokens_to_css_text(&tokens), "#ff0000");
    }

    #[test]
    fn css_text_string() {
        let tokens = [CssToken::String("hello".into())];
        assert_eq!(tokens_to_css_text(&tokens), "\"hello\"");
    }

    #[test]
    fn css_text_number() {
        let tokens = [CssToken::Number(42.0)];
        assert_eq!(tokens_to_css_text(&tokens), "42");
    }

    #[test]
    fn css_text_percentage() {
        let tokens = [CssToken::Percentage(50.0)];
        assert_eq!(tokens_to_css_text(&tokens), "50%");
    }

    #[test]
    fn css_text_dimension() {
        let tokens = [CssToken::Dimension(10.0, "px".into())];
        assert_eq!(tokens_to_css_text(&tokens), "10px");
    }

    #[test]
    fn css_text_punctuation() {
        let tokens = [
            CssToken::OpenBrace,
            CssToken::CloseBrace,
            CssToken::OpenParen,
            CssToken::CloseParen,
            CssToken::OpenBracket,
            CssToken::CloseBracket,
            CssToken::Colon,
            CssToken::Semicolon,
            CssToken::Comma,
            CssToken::Dot,
            CssToken::Greater,
            CssToken::Plus,
            CssToken::Star,
            CssToken::Slash,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "{}()[]:;,.>+*/");
    }

    #[test]
    fn css_text_delim() {
        let tokens = [CssToken::Delim('~')];
        assert_eq!(tokens_to_css_text(&tokens), "~");
    }

    #[test]
    fn css_text_at_keyword() {
        let tokens = [CssToken::AtKeyword("media".into())];
        assert_eq!(tokens_to_css_text(&tokens), "@media");
    }

    #[test]
    fn css_text_function() {
        let tokens = [
            CssToken::Function("rgb".into()),
            CssToken::Number(255.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::Comma,
            CssToken::Number(0.0),
            CssToken::CloseParen,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "rgb(255,0,0)");
    }

    #[test]
    fn css_text_eof_ignored() {
        let tokens = [CssToken::Ident("x".into()), CssToken::Eof];
        assert_eq!(tokens_to_css_text(&tokens), "x");
    }

    #[test]
    fn css_text_whitespace_trimmed() {
        let tokens = [
            CssToken::Whitespace,
            CssToken::Ident("a".into()),
            CssToken::Whitespace,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "a");
    }

    #[test]
    fn css_text_complex_declaration() {
        let tokens = [
            CssToken::Ident("margin".into()),
            CssToken::Colon,
            CssToken::Whitespace,
            CssToken::Dimension(10.0, "px".into()),
            CssToken::Whitespace,
            CssToken::Dimension(20.0, "px".into()),
            CssToken::Semicolon,
        ];
        assert_eq!(tokens_to_css_text(&tokens), "margin: 10px 20px;");
    }
}
