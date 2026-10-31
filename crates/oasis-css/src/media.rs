//! Media-query evaluation against a [`MediaViewport`].

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
    // A leading `not`, possibly repeated, is peeled in a loop: one
    // recursion per `not` let `not not not ...` overflow the stack.
    let mut query = query.trim();
    let mut negated = false;
    while let Some(rest) = query.strip_prefix("not ") {
        query = rest.trim_start();
        negated = !negated;
    }
    eval_positive_media_query(query, viewport) != negated
}

/// [`eval_single_media_query`] without a leading `not`.
fn eval_positive_media_query(query: &str, viewport: MediaViewport) -> bool {
    let query = query.trim();
    if query.is_empty() || query == "all" || query == "screen" {
        return true;
    }
    if query == "print" {
        return false;
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
    // `em` / `rem` in a media query are relative to the initial font
    // size (Media Queries 4 §1.3), 16px. Regression: `48em` parsed as 0,
    // so every `(min-width: 48em)` breakpoint matched and every
    // `(max-width: 48em)` one never did.
    const INITIAL_FONT_SIZE: f32 = 16.0;
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let (num, scale) = if let Some(n) = lower.strip_suffix("rem") {
        (n, INITIAL_FONT_SIZE)
    } else if let Some(n) = lower.strip_suffix("em") {
        (n, INITIAL_FONT_SIZE)
    } else {
        (lower.strip_suffix("px").unwrap_or(&lower), 1.0)
    };
    num.trim().parse::<f32>().map_or(0.0, |v| v * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_px_value_em_and_rem_use_the_initial_font_size() {
        assert!((parse_px_value("48em") - 768.0).abs() < f32::EPSILON);
        assert!((parse_px_value("2.5REM") - 40.0).abs() < f32::EPSILON);
        let vp = MediaViewport::DEFAULT;
        assert!(!eval_single_media_query("(min-width: 48em)", vp));
        assert!(eval_single_media_query("(max-width: 48em)", vp));
    }

    #[test]
    fn repeated_not_is_peeled_iteratively() {
        let vp = MediaViewport::DEFAULT;
        assert!(eval_single_media_query("not not screen", vp));
        assert!(!eval_single_media_query("not screen", vp));
        // Deep enough to overflow the stack with one frame per `not`.
        let deep = format!("{}print", "not ".repeat(200_001));
        assert!(eval_single_media_query(&deep, vp));
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
}
