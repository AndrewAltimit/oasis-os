//! Site-compatibility shims for inline `onclick` helpers.

/// Install compatibility shims for common site-specific helpers that
/// inline `onclick="..."` handlers call. Registered after `engine.eval_all()`
/// so that a page's own `togglecomment`/`hidecomment` definitions (if they
/// parse successfully) take precedence. When the page's script bundle
/// doesn't load — old.reddit.com ships a ~1 MB bundle built around feature
/// detection and jQuery — these shims let the page stay interactive.
///
/// The reddit shims all walk from the clicked element up to the nearest
/// `.comment` ancestor and toggle a `collapsed` class; the fixture's CSS
/// (and the real site's sheet) already hides `.comment.collapsed .child`
/// and friends, so toggling one class is enough to collapse/expand a
/// thread and its replies. Returning `false` from the onclick suppresses
/// the default link navigation.
pub(crate) fn install_site_compat_shims(engine: &oasis_js::JsEngine) {
    // Keep the JS small; each helper is a one-liner wrapped in
    // `if typeof ... === 'undefined'` so a real site script wins.
    let _ = engine.eval(COMPAT_SHIMS_JS);
}

/// JavaScript source of the site-compat shims (see
/// [`install_site_compat_shims`]).
const COMPAT_SHIMS_JS: &str = include_str!("compat_shims.js");
