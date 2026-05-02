//! Stacking-context and compositing-layer predicates used by the
//! paint pass.
//!
//! Pure predicates over [`LayoutBox`] / [`ComputedStyle`] that decide
//! which boxes start a new stacking context, which need a real
//! offscreen render target (vs. the cheap `PushLayer` opacity fast
//! path), and which are positioned at all. The recursive walker uses
//! these to drive paint ordering and layer promotion.

use crate::css::values::Position;
use crate::layout::box_model::LayoutBox;

/// Returns `true` if a layout box creates a new stacking context.
///
/// Per CSS 2.1, a stacking context is created by:
/// - Positioned elements (non-static) with an explicit z-index (not `auto`)
/// - Elements with opacity < 1.0
/// - Elements with CSS transforms
/// - Elements with CSS filters
/// - Elements with will-change: transform
///
/// Note: positioned elements with `z-index: auto` do NOT create a stacking
/// context -- they participate in the parent's stacking context at level 0.
pub(crate) fn creates_stacking_context(layout_box: &LayoutBox) -> bool {
    let style = &layout_box.style;

    // Positioned + explicit z-index (not auto).
    if style.position != Position::Static && !style.z_index_auto {
        return true;
    }

    // Opacity < 1.0 creates a stacking context.
    if style.opacity < 1.0 {
        return true;
    }

    // Non-empty transforms create a stacking context.
    if !style.transforms.is_empty() {
        return true;
    }

    // Filters create a stacking context.
    if !style.filters.is_empty() {
        return true;
    }

    // will-change: transform/opacity/filter creates a stacking context.
    if style.will_change_promotes_layer {
        return true;
    }

    // `isolation: isolate` forces a stacking context so descendant
    // `mix-blend-mode` is contained (CSS Compositing and Blending L1).
    if matches!(
        style.isolation,
        crate::css::values::types::Isolation::Isolate
    ) {
        return true;
    }

    // `mix-blend-mode` on a non-Normal value implies a stacking context
    // so the blend applies to the entire painted subtree.
    if !matches!(
        style.mix_blend_mode,
        crate::css::values::types::BlendMode::Normal
    ) {
        return true;
    }

    // Non-empty backdrop-filter chain implies a stacking context
    // (the backdrop is sampled at the layer's own boundary).
    if !style.backdrop_filters.is_empty() {
        return true;
    }

    // `mask-image: <image>` forces a stacking context so the mask can
    // be applied to the entire painted subtree in one destination-in
    // pass instead of per-box.
    if !matches!(style.mask_image, crate::css::values::BackgroundImage::None) {
        return true;
    }

    false
}

/// Whether this layout box needs a *real* compositing layer — i.e. an
/// offscreen render target — rather than just the cheap `PushLayer`
/// opacity fast path.
///
/// True when any of `mix-blend-mode`, `backdrop-filter`, `filter`,
/// `isolation: isolate`, or `will-change: transform/opacity/filter` is
/// active. Plain `opacity < 1.0` stays on the fast path.
pub(crate) fn creates_compositing_layer(layout_box: &LayoutBox) -> bool {
    let style = &layout_box.style;

    if !matches!(
        style.mix_blend_mode,
        crate::css::values::types::BlendMode::Normal
    ) {
        return true;
    }

    if !style.backdrop_filters.is_empty() {
        return true;
    }

    if !style.filters.is_empty() {
        return true;
    }

    if matches!(
        style.isolation,
        crate::css::values::types::Isolation::Isolate
    ) {
        return true;
    }

    if style.will_change_promotes_layer {
        return true;
    }

    // `mask-image` needs a real offscreen layer so we can apply a
    // destination-in pass to the full painted subtree. Without this
    // promotion, the mask would have nothing to bite into at
    // `PopCompositingLayer` time.
    if !matches!(style.mask_image, crate::css::values::BackgroundImage::None) {
        return true;
    }

    false
}

/// Returns `true` if a layout box is positioned (position != static).
pub(crate) fn is_positioned(layout_box: &LayoutBox) -> bool {
    !matches!(layout_box.style.position, Position::Static)
}
