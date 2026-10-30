# UI Widgets (`oasis-ui`)

`oasis-ui` is the reusable widget toolkit apps and shell chrome build on.
Everything draws through a theme-aware `DrawContext` over any
`SdiBackend`, so the same widget renders on SDL, WASM, UE5 and PSP.

All paths below are relative to `crates/oasis-ui/src/`.

## The `Widget` trait

```rust,ignore
pub trait Widget {
    /// Desired size given the available space.
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, available_h: u32) -> (u32, u32);
    /// Draw at the given position and size.
    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, h: u32) -> Result<()>;
}
```

Widgets are plain structs: the caller owns layout (via the helpers
below or its own math) and input (widgets expose state methods such as
`Checkbox::toggle`, `Dropdown::select_next`, `DatePicker::select_day`
that the app calls from its own input handlers). There is no retained widget tree or
event loop.

### Drawing helpers

Widgets share two `DrawContext` helpers instead of repeating the
measure / center / draw sequence by hand:

- `ctx.text_in((x, y, w, h), text, font_size, color, HAlign)` draws `text`
  vertically centered in the rect and placed horizontally by
  `layout::HAlign` (`Left`, `Center`, `Right`), using exactly
  `layout::align_x` / `layout::center`, so it is pixel-identical to the
  hand-written form.
- `ctx.frame((x, y, w, h), bg, border, radius)` draws a rounded fill plus a
  1-pixel rounded border.

Supporting types:

| Type | File | Purpose |
|------|------|---------|
| `DrawContext` / `Region` | `context.rs` | Backend + `Theme` bundle passed to `measure` / `draw`; sub-regions for nested drawing; `text_in` / `frame` drawing helpers (below) |
| `Theme` | `theme.rs` | Colors, font sizes, spacing and radii. Shipping themes are derived from the active skin (`SkinTheme::to_ui_theme`, with light / high-contrast variants via `oasis_skin::SkinVariant`); `Theme::dark()` is the built-in fallback |
| `WidgetState` / `WidgetStateColors` | `states.rs` | Uniform normal / hover / pressed / disabled color resolution, shared by every interactive widget (`Button::state` included); keyboard focus is a separate `focused` flag |
| `widget::Widget` | `widget.rs` | The trait above |

## Widgets implementing `Widget` (29)

**Adoption.** The toolkit is a library catalogue: only a few of these are
used outside `oasis-ui` today -- `Button` (Calculator), `ProgressBar`
(Radio, System Monitor), `Slider` and `Toggle` (Settings), plus the
non-`Widget` `MenuBar` (File Manager, Paint, Text Editor), `flex` and the
`animation::easing` curves. The other 25 widgets, `ListView`, `NinePatch`,
`IconAtlas`, `FocusManager` and `accessibility` have no in-tree callers.
They are kept on purpose (complete, tested, and earmarked for the desktop
metaphor in [theming-desktop-plan.md](theming-desktop-plan.md)) and stay
compiled by default rather than behind a cargo feature: they cost only
build time inside this one crate, a `widgets-extra` feature would put a
`cfg` on two dozen modules and their tests and on the
`oasis_core::ui` re-export, and nothing downstream would get smaller. Treat
an unadopted widget as working but unproven in a real app: expect to
adjust its API when the first app picks it up.

| Widget | File | Purpose |
|--------|------|---------|
| `Accordion` | `accordion.rs` | Collapsible sections, single- or multi-expand (`AccordionMode`) |
| `Avatar` | `avatar.rs` | Circular image with fallback initial |
| `Badge` | `badge.rs` | Small colored tag / count indicator |
| `Button` | `button.rs` | Push button with `ButtonStyle` variants; `state: WidgetState` (normal / hover / pressed / disabled) plus a `focused` ring |
| `Card` | `card.rs` | Content card with optional image, title, subtitle and body |
| `Checkbox` | `checkbox.rs` | Labelled boolean checkbox |
| `ColorPicker` | `color_picker.rs` | HSV color selection with RGB preview |
| `ContextMenu` | `context_menu.rs` | Right-click popup with actions, separators and submenus (`MenuItem`) |
| `DatePicker` | `date_picker.rs` | Calendar grid with month / year navigation |
| `Divider` | `divider.rs` | Horizontal or vertical separator line |
| `Dropdown` | `dropdown.rs` | Dropdown / combobox with an option list |
| `InputField` | `input_field.rs` | Single-line text input with cursor |
| `Modal` | `modal.rs` | Modal dialog with input blocking and button presets (`ModalButtons`, `ModalResult`) |
| `Panel` | `panel.rs` | Container with background, border, shadow and rounded corners |
| `ProgressBar` | `progress_bar.rs` | Determinate progress / gauge (`ProgressStyle`) |
| `RadioGroup` | `radio.rs` | Mutually exclusive option selection |
| `RichText` | `rich_text.rs` | Formatted text built from styled `Span`s |
| `ScrollView` | `scroll_view.rs` | Scrollable content region with a scrollbar (`ScrollbarStyle`) |
| `Slider` | `slider.rs` | Value within a range, horizontal or vertical |
| `SpinBox` | `spin_box.rs` | Numeric input with increment / decrement buttons |
| `Spinner` | `spinner.rs` | Loading indicators (`SpinnerStyle`) |
| `SplitPane` | `split_pane.rs` | Two resizable panes with a draggable divider |
| `Table` | `table.rs` | Multi-column data grid with optional header, selection and sort direction |
| `TabBar` | `tab_bar.rs` | Row of tabs (`TabStyle`) |
| `TextBlock` | `text_block.rs` | Multi-line text with wrapping, truncation and alignment |
| `ToastStack` | `toast.rs` | Stack of ephemeral `Toast` notifications (`ToastLevel`, `ToastPosition`) |
| `Toggle` | `toggle.rs` | On / off switch |
| `Tooltip` | `tooltip.rs` | Hover-activated text overlay with delay state machine and `TooltipAnchor` positioning |
| `TreeView` | `tree_view.rs` | Hierarchical tree with expand / collapse and keyboard navigation |

## Other components (not `Widget` impls)

| Component | File | Purpose |
|-----------|------|---------|
| `ListView<T>` | `list_view.rs` | Scrollable list with virtualized item rendering (items drawn by a caller closure) |
| `MenuBar` | `menu_bar.rs` | Windows-95-style top menu bar with drop-downs; returns `MenuHit` for clicks (used by Paint, Text Editor) |
| `NinePatch`, `NinePatchSlices` | `nine_patch.rs` | 9-slice rendering of themed borders from a texture (re-exported from `oasis_types::nine_patch`) |
| `IconAtlas` / `Icon` | `icon.rs` | Icon atlas rendering |
| `FlexLayout`, `GridLayout` | `flex.rs` | Lightweight flex and grid layout helpers producing `ComputedRect`s |
| `Padding`, `center`, `distribute`, `HAlign` / `VAlign`, `MeasureCache` | `layout.rs` | Centering, alignment, padding, distribution, cached text measurement |
| `FocusRing`, `FocusManager` | `focus.rs` | Keyboard focus navigation and focus styling for widget groups |
| `Tween`, `ColorTween`, easing functions | `animation.rs` | Frame-driven animation primitives |
| `AccessibilityLabel`, `contrast_ratio`, `meets_wcag_aa` / `_aaa` | `accessibility.rs` | Semantic labels and WCAG contrast checks |
| `test_utils` | `test_utils.rs` | Test-only, crate-internal: re-exports `oasis_test_backend::RecordingBackend`, plus `all_themes()` / `test_draw_all_themes` / `assert_states_distinct` helpers |

`color` and `shadow` are re-exported from `oasis-types`.

## Using a widget

```rust,ignore
use oasis_ui::{DrawContext, Widget};
use oasis_ui::progress_bar::ProgressBar;

fn draw_gauge(ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, pct: f32) -> oasis_types::error::Result<()> {
    let bar = ProgressBar::new(pct); // value in 0.0..=1.0
    let (_, h) = bar.measure(ctx, w, 32);
    bar.draw(ctx, x, y, w, h)
}
```

Apps typically build a `DrawContext::new(backend, &theme)` inside
`App::draw_windowed`, and keep one layout struct that both
drawing and click hit-testing use (see the Calculator's `CalcLayout` or
Paint's `PaintLayout`). See [writing-apps.md](writing-apps.md).

## Tests

Each widget has in-module tests that draw into oasis-test-backend's
`RecordingBackend` (a dev-dependency) and assert on its command log
(`fill_rect_count`, `has_text`, `text_positions`, ...). The
`test_utils::test_draw_all_themes` sweep runs a widget under `Theme::dark()`
and two test palettes (a light one and a high-contrast one with no
rounding or shadows).
