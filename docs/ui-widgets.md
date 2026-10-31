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
  hand-written form whenever the text fits. Text wider than the rect is
  truncated with `...` from the left edge (`SdiText::draw_text_ellipsis`)
  and dropped when not even the ellipsis fits, so a label never spills
  out of its widget.
- `ctx.frame((x, y, w, h), bg, border, radius)` draws a rounded fill plus a
  1-pixel rounded border.

Supporting types:

| Type | File | Purpose |
|------|------|---------|
| `DrawContext` / `Region` | `context.rs` | Backend + `Theme` bundle passed to `measure` / `draw`; sub-regions for nested drawing; `text_in` / `frame` drawing helpers (below) |
| `Theme` | `theme.rs` | Colors, font sizes, spacing and radii. Shipping themes are derived from the active skin (`SkinTheme::to_ui_theme`, with light / high-contrast variants via `oasis_skin::SkinVariant`); `Theme::dark()` is the built-in fallback |
| `WidgetState` / `WidgetStateColors` | `states.rs` | Uniform normal / hover / pressed / disabled color resolution, shared by every interactive widget (`Button::state` included); keyboard focus is a separate `focused` flag |
| `widget::Widget` | `widget.rs` | The trait above |

## Widgets implementing `Widget` (27)

**Adoption.** The toolkit is a library catalogue: only a few of these are
used outside `oasis-ui` today -- `Button` (Calculator), `ProgressBar`
(Radio, System Monitor), `Slider` and `Toggle` (Settings), plus the
non-`Widget` `MenuBar` (File Manager, Paint, Text Editor), `flex` and the
`animation::easing` curves. The other 23 widgets, `ListView`, `NinePatch`
and the focus managers have no in-tree callers yet.

They are kept on purpose: they are the general-purpose controls any app
toolkit needs (text input, selection, lists, tables, trees, dialogs,
menus), they already follow one convention (below), and the robustness
sweep keeps them honest. They stay compiled by default rather than behind
a cargo feature: they cost only build time inside this one crate, and a
`widgets-extra` feature would put a `cfg` on two dozen modules and on the
`oasis_core::ui` re-export while making nothing downstream smaller. Treat
an unadopted widget as working but unproven in a real app: expect to
adjust its API when the first app picks it up.

Pruned (removed rather than kept as dead weight): `ToastStack` (the shell
has its own skin-themed, SDI-based `oasis_core::toast::ToastManager`),
`Avatar` (no user-account concept anywhere in the OS), `IconAtlas` / `Icon`
(no atlas texture ships; icons are vector, see
[vector-graphics.md](vector-graphics.md)) together with the never-drawn
`Button::icon` field, and `accessibility` (its WCAG contrast helpers
duplicated `oasis_types::color::{contrast_ratio, relative_luminance,
meets_wcag_aa}`, and `AccessibilityLabel` had no assistive-technology
consumer).

### Widget conventions

Every widget follows the same shape, so adopting one looks like adopting
any other:

- **Construction**: `Widget::new(..)` with the content (label, options,
  range), then plain `pub` fields or `with_*` builders for the rest.
- **Interaction state**: `hovered` / `pressed` / `disabled` flags resolve
  through `WidgetState` + `WidgetStateColors` (`state()` /
  `part_state()` / `option_state()`), plus a separate `focused` flag that
  draws the theme's focus ring (`FocusStyle::from_theme`).
- **Input**: the app owns events. Widgets expose state transitions
  (`toggle`, `select_next`, `navigate_down`, `scroll_by`, `handle_key`)
  and local-offset hit tests (`option_at`, `tab_at`, `part_at`,
  `index_at`, `item_at`) that the app calls from its own handlers.
- **Styling**: all colors, font sizes, radii and shadows come from the
  `Theme` in the `DrawContext` (derived from the active skin); widgets
  hard-code geometry only.
- **Degenerate rects**: any widget can be drawn at any size, including
  0x0 -- `robustness_tests.rs` sweeps every widget at zero, one-pixel,
  few-pixel and full-screen sizes with overflowing content under every
  test palette.

| Widget | File | Purpose |
|--------|------|---------|
| `Accordion` | `accordion.rs` | Collapsible sections, single- or multi-expand (`AccordionMode`) |
| `Badge` | `badge.rs` | Small colored tag / count indicator |
| `Button` | `button.rs` | Push button with `ButtonStyle` variants; `state: WidgetState` (normal / hover / pressed / disabled) plus a `focused` ring |
| `Card` | `card.rs` | Content card with optional image, title, subtitle and body |
| `Checkbox` | `checkbox.rs` | Labelled boolean checkbox |
| `ColorPicker` | `color_picker.rs` | HSV color selection with RGB preview |
| `ContextMenu` | `context_menu.rs` | Right-click popup with actions, separators and submenus (`MenuItem`); `show_within` opens up / left of the point near the screen edge |
| `DatePicker` | `date_picker.rs` | Calendar grid with month / year navigation |
| `Divider` | `divider.rs` | Horizontal or vertical separator line |
| `Dropdown` | `dropdown.rs` | Dropdown / combobox with an option list |
| `InputField` | `input_field.rs` | Single-line text input with cursor (`insert`, `backspace`, `delete`, `move_left` / `_right` / `_home` / `_end`); scrolls to keep the cursor visible while focused |
| `Modal` | `modal.rs` | Modal dialog with input blocking and button presets (`ModalButtons`, `ModalResult`) |
| `Panel` | `panel.rs` | Container with background, border, shadow and rounded corners |
| `ProgressBar` | `progress_bar.rs` | Determinate progress / gauge (`ProgressStyle`) |
| `RadioGroup` | `radio.rs` | Mutually exclusive option selection |
| `RichText` | `rich_text.rs` | Formatted text built from styled `Span`s |
| `ScrollView` | `scroll_view.rs` | Scrollable content region with a scrollbar (`ScrollbarStyle`); `scroll_by` saturates and clamps |
| `Slider` | `slider.rs` | Value within a range, horizontal or vertical |
| `SpinBox` | `spin_box.rs` | Numeric input with increment / decrement buttons |
| `Spinner` | `spinner.rs` | Loading indicators (`SpinnerStyle`) |
| `SplitPane` | `split_pane.rs` | Two resizable panes with a draggable divider |
| `Table` | `table.rs` | Multi-column data grid with optional header, selection and stable sort (the selection follows its row) |
| `TabBar` | `tab_bar.rs` | Row of tabs (`TabStyle`) |
| `TextBlock` | `text_block.rs` | Multi-line text with wrapping, truncation and alignment |
| `Toggle` | `toggle.rs` | On / off switch |
| `Tooltip` | `tooltip.rs` | Hover-activated text overlay with delay state machine and `TooltipAnchor` positioning (flips to the opposite side rather than covering its target at a viewport edge) |
| `TreeView` | `tree_view.rs` | Hierarchical tree with expand / collapse and keyboard navigation |

## Other components (not `Widget` impls)

| Component | File | Purpose |
|-----------|------|---------|
| `ListView<T>` | `list_view.rs` | Scrollable list with virtualized item rendering (items drawn by a caller `fn`); `scroll_to` / clamped `scroll_by` |
| `MenuBar` | `menu_bar.rs` | Windows-95-style top menu bar with drop-downs; returns `MenuHit` for clicks (used by Paint, Text Editor) |
| `NinePatch`, `NinePatchSlices` | `nine_patch.rs` | 9-slice rendering of themed borders from a texture (re-exported from `oasis_types::nine_patch`) |
| `FlexLayout`, `GridLayout` | `flex.rs` | Lightweight flex and grid layout helpers producing `ComputedRect`s; flex shares sum to exactly the free space (no stray pixel after the last child) and huge sizes saturate |
| `Padding`, `center`, `distribute`, `max_scroll`, `HAlign` / `VAlign`, `MeasureCache` | `layout.rs` | Centering, alignment, padding, distribution, scroll bounds, cached text measurement |
| `FocusRing`, `FocusManager`, `SpatialFocusManager`, `FocusStyle` | `focus/` (`ring.rs`, `manager.rs`, `spatial.rs`, `style.rs`) | Index focus cursor; Tab / Shift-Tab cycling that skips disabled items; tab-index + arrow-key spatial navigation (consumes the backends' `InputEvent::Tab` / `ShiftTab`); focus-ring styling from the theme |
| `Tween`, `ColorTween`, easing functions | `animation.rs` | Frame-driven animation primitives |
| `test_utils` | `test_utils.rs` | Test-only, crate-internal: re-exports `oasis_test_backend::RecordingBackend`, plus `all_themes()` / `test_draw_all_themes` / `assert_states_distinct` helpers |

`color` and `shadow` are re-exported from `oasis-types` (WCAG contrast
checks live there: `color::contrast_ratio`, `color::meets_wcag_aa`).

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
rounding or shadows). `robustness_tests.rs` measures and draws one
populated instance of every `Widget` impl at degenerate sizes under all
three palettes; add new widgets to its `all_widgets()` list.
