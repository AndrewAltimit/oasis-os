//! Shared test utilities for oasis-ui widget tests.
//!
//! Widget tests draw into oasis-test-backend's [`RecordingBackend`] (which
//! keeps every default-method decomposition, so `fill_rect_count` counts
//! the primitive rects a rounded rect or a gradient expands into) and
//! assert on its command log.

use oasis_types::backend::Color;

pub use oasis_test_backend::RecordingBackend;

/// Draw a widget once per [`WidgetState`](crate::states::WidgetState)
/// and assert that every state produced a distinct sequence of fill
/// colors (i.e. each interaction state is visually distinguishable).
///
/// The closure receives the state to render and a fresh context.
pub fn assert_states_distinct(
    mut f: impl FnMut(crate::states::WidgetState, &mut crate::context::DrawContext<'_>),
) {
    use crate::states::WidgetState;
    let theme = crate::theme::Theme::dark();
    let states = [
        WidgetState::Normal,
        WidgetState::Hover,
        WidgetState::Pressed,
        WidgetState::Disabled,
    ];
    let fills: Vec<Vec<Color>> = states
        .iter()
        .map(|&st| {
            let mut backend = RecordingBackend::default();
            {
                let mut ctx = crate::context::DrawContext::new(&mut backend, &theme);
                f(st, &mut ctx);
            }
            backend.fill_colors()
        })
        .collect();
    for i in 0..states.len() {
        for j in (i + 1)..states.len() {
            assert_ne!(
                fills[i], fills[j],
                "{:?} and {:?} rendered identical fills",
                states[i], states[j]
            );
        }
    }
}

/// Run `f` against a fresh dark-theme context and return the fill colors
/// it emitted. Helper for "resting rendering unchanged" assertions.
pub fn fill_colors_of(f: impl FnOnce(&mut crate::context::DrawContext<'_>)) -> Vec<Color> {
    let theme = crate::theme::Theme::dark();
    let mut backend = RecordingBackend::default();
    {
        let mut ctx = crate::context::DrawContext::new(&mut backend, &theme);
        f(&mut ctx);
    }
    backend.fill_colors()
}

/// A light palette (light surfaces, dark text) for exercising widgets
/// against something other than [`Theme::dark`](crate::theme::Theme::dark).
/// Shipping themes come from oasis-skin (`SkinVariant::Light`).
pub fn light_test_theme() -> crate::theme::Theme {
    let mut t = crate::theme::Theme::dark();
    t.background = Color::rgb(245, 245, 250);
    t.surface = Color::rgb(255, 255, 255);
    t.surface_variant = Color::rgb(235, 235, 242);
    t.text_primary = Color::rgb(20, 20, 30);
    t.text_secondary = Color::rgb(90, 90, 110);
    t.text_disabled = Color::rgb(170, 170, 185);
    t.button_bg = Color::rgb(230, 230, 238);
    t.button_bg_hover = Color::rgb(215, 215, 228);
    t.button_bg_pressed = Color::rgb(195, 195, 212);
    t.input_bg = Color::rgb(255, 255, 255);
    t.border = Color::rgb(200, 200, 212);
    t
}

/// A high-contrast palette with no rounding and no shadows (the shape a
/// `SkinVariant::HighContrast` skin produces), for edge-case geometry.
pub fn high_contrast_test_theme() -> crate::theme::Theme {
    let mut t = crate::theme::Theme::dark();
    t.background = Color::rgb(0, 0, 0);
    t.surface = Color::rgb(0, 0, 0);
    t.text_primary = Color::rgb(255, 255, 255);
    t.border = Color::rgb(255, 255, 255);
    t.border_radius_sm = 0;
    t.border_radius_md = 0;
    t.border_radius_lg = 0;
    t.border_radius_xl = 0;
    t.shadow_card = crate::shadow::Shadow::elevation(0);
    t.shadow_dropdown = crate::shadow::Shadow::elevation(0);
    t.shadow_modal = crate::shadow::Shadow::elevation(0);
    t.shadow_tooltip = crate::shadow::Shadow::elevation(0);
    t
}

/// The palettes widget tests sweep: dark plus the two fixtures above.
pub fn all_themes() -> [crate::theme::Theme; 3] {
    [
        crate::theme::Theme::dark(),
        light_test_theme(),
        high_contrast_test_theme(),
    ]
}

/// Test that a widget draws without panicking across [`all_themes`].
///
/// The closure receives a `&mut DrawContext` and should construct + draw the
/// widget under test:
///
/// ```ignore
/// test_draw_all_themes(|ctx| {
///     let c = Checkbox::new("Test", true);
///     c.draw(ctx, 0, 0, 200, 20).unwrap();
/// });
/// ```
pub fn test_draw_all_themes(mut f: impl FnMut(&mut crate::context::DrawContext<'_>)) {
    for theme in all_themes() {
        let mut backend = RecordingBackend::default();
        let mut ctx = crate::context::DrawContext::new(&mut backend, &theme);
        f(&mut ctx);
    }
}
