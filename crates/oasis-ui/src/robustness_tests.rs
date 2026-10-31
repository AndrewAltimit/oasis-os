//! Cross-widget robustness sweep.
//!
//! Every `Widget` impl is measured and drawn at degenerate sizes (zero,
//! one and a few pixels, plus one huge size) and with overflowing
//! content, under every test palette. Debug builds panic on `u32`
//! underflow, so this is the regression net for "tiny rect" arithmetic
//! (`w - 2 * pad` and friends) across the whole catalogue.

use crate::accordion::{Accordion, AccordionMode};
use crate::badge::Badge;
use crate::button::Button;
use crate::card::Card;
use crate::checkbox::Checkbox;
use crate::color_picker::ColorPicker;
use crate::context::DrawContext;
use crate::context_menu::{ContextMenu, MenuItem};
use crate::date_picker::DatePicker;
use crate::divider::Divider;
use crate::dropdown::Dropdown;
use crate::input_field::InputField;
use crate::list_view::ListView;
use crate::modal::Modal;
use crate::panel::Panel;
use crate::progress_bar::ProgressBar;
use crate::radio::RadioGroup;
use crate::rich_text::{RichText, Span};
use crate::scroll_view::ScrollView;
use crate::slider::{Orientation, Slider};
use crate::spin_box::SpinBox;
use crate::spinner::Spinner;
use crate::split_pane::{SplitOrientation, SplitPane};
use crate::tab_bar::TabBar;
use crate::table::{Column, Table};
use crate::test_utils::{RecordingBackend, all_themes};
use crate::text_block::TextBlock;
use crate::toggle::Toggle;
use crate::tooltip::Tooltip;
use crate::tree_view::TreeView;
use crate::widget::Widget;

const LONG: &str = "An extremely long label that cannot possibly fit in a tiny widget rect";

// `ListView<String>` hands its renderer a `&String`.
#[allow(clippy::ptr_arg)]
fn list_item(
    item: &String,
    ctx: &mut DrawContext<'_>,
    x: i32,
    y: i32,
    _w: u32,
    _h: u32,
    _selected: bool,
) -> oasis_types::error::Result<()> {
    let color = ctx.theme.text_primary;
    ctx.backend.draw_text(item, x, y, 8, color)
}

/// One instance of every `Widget` impl, populated so that every draw
/// path (open menus, selections, scrollbars, long text) is exercised.
fn all_widgets() -> Vec<(&'static str, Box<dyn Widget>)> {
    let mut accordion = Accordion::new(AccordionMode::MultiExpand);
    accordion.add_section(LONG, vec![LONG.into(), "b".into()]);
    accordion.add_section("Two", vec![]);
    accordion.expand_all();

    let mut card = Card::new(LONG);
    card.subtitle = Some(LONG.into());
    card.body = Some(LONG.into());

    let mut menu = ContextMenu::new(vec![
        MenuItem::action(LONG),
        MenuItem::separator(),
        MenuItem::submenu("More", vec![MenuItem::action("x")]),
        MenuItem::action_disabled("Off"),
    ]);
    menu.show(0, 0);

    let mut dropdown = Dropdown::new(vec![LONG.into(), "b".into(), "c".into()]);
    dropdown.open = true;

    let mut input = InputField::new();
    input.text = LONG.into();
    input.cursor_pos = LONG.len();
    input.focused = true;

    let mut rich = RichText::new();
    rich.add_line(vec![Span::plain(LONG), Span::bold(LONG)]);
    rich.add_plain("second");

    let mut table = Table::new(vec![Column::new(LONG, 40), Column::new("B", 0)]);
    for i in 0..20 {
        table.add_row(vec![format!("{LONG} {i}"), "x".into()]);
    }
    table.selected_row = Some(19);
    table.scroll_offset = 25;

    let mut tree = TreeView::new();
    let root = tree.add_root(LONG);
    for _ in 0..5 {
        let _ = tree.add_child(root, LONG);
    }
    tree.expand_all();

    let mut tooltip = Tooltip::new(LONG).with_delay(0);
    tooltip.on_hover_start();
    tooltip.tick(1);

    let mut scroll = ScrollView::new(10_000, 10);
    scroll.scroll_by(5_000);

    let mut list = ListView::new(
        (0..50).map(|i| format!("item {i}")).collect(),
        12,
        list_item,
    );
    list.selected = Some(49);
    list.scroll_offset = 400;

    let mut tabs = TabBar::new(vec![LONG.into(), "b".into(), "c".into()]);
    tabs.active = 2;

    let mut spin = SpinBox::new(-1e9, 1e9);
    spin.set_value(123_456_789.0);

    let mut slider_v = Slider::new(0.0, 100.0).with_orientation(Orientation::Vertical);
    slider_v.set_value(100.0);

    vec![
        ("Accordion", Box::new(accordion)),
        ("Badge", Box::new(Badge::new(LONG))),
        ("Button", Box::new(Button::new(LONG))),
        ("Card", Box::new(card)),
        ("Checkbox", Box::new(Checkbox::new(LONG, true))),
        ("ColorPicker", Box::new(ColorPicker::new())),
        ("ContextMenu", Box::new(menu)),
        ("DatePicker", Box::new(DatePicker::new(2024, 2))),
        ("Divider(h)", Box::new(Divider::horizontal())),
        ("Divider(v)", Box::new(Divider::vertical())),
        ("Dropdown", Box::new(dropdown)),
        ("InputField", Box::new(input)),
        ("ListView", Box::new(list)),
        ("Modal", Box::new(Modal::yes_no_cancel(LONG, LONG))),
        ("Panel", Box::new(Panel::default())),
        ("ProgressBar", Box::new(ProgressBar::new(0.5))),
        (
            "RadioGroup",
            Box::new(RadioGroup::new(vec![LONG.into(), "b".into()])),
        ),
        ("RichText", Box::new(rich)),
        ("ScrollView", Box::new(scroll)),
        ("Slider", Box::new(Slider::new(0.0, 1.0))),
        ("Slider(v)", Box::new(slider_v)),
        ("SpinBox", Box::new(spin)),
        ("Spinner", Box::new(Spinner::with_label(LONG))),
        (
            "SplitPane(h)",
            Box::new(SplitPane::new(SplitOrientation::Horizontal)),
        ),
        (
            "SplitPane(v)",
            Box::new(SplitPane::new(SplitOrientation::Vertical)),
        ),
        ("TabBar", Box::new(tabs)),
        ("Table", Box::new(table)),
        (
            "TextBlock",
            Box::new(TextBlock::new(format!("{LONG}\n{LONG}"))),
        ),
        ("Toggle", Box::new(Toggle::new(true))),
        ("Tooltip", Box::new(tooltip)),
        ("TreeView", Box::new(tree)),
    ]
}

const SIZES: &[(u32, u32)] = &[
    (0, 0),
    (1, 1),
    (2, 2),
    (3, 3),
    (5, 4),
    (7, 9),
    (12, 12),
    (0, 200),
    (200, 0),
    (1, 300),
    (300, 1),
    (480, 272),
];

#[test]
fn every_widget_survives_degenerate_sizes() {
    let mut failures = Vec::new();
    for theme in all_themes() {
        for (name, widget) in all_widgets() {
            for &(w, h) in SIZES {
                let mut backend = RecordingBackend::default();
                let mut ctx = DrawContext::new(&mut backend, &theme);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = widget.measure(&ctx, w, h);
                    widget.draw(&mut ctx, -3, -3, w, h)
                }));
                match result {
                    Ok(Ok(())) => {},
                    Ok(Err(e)) => failures.push(format!("{name} at {w}x{h}: draw returned {e}")),
                    Err(_) => failures.push(format!("{name} panicked at {w}x{h}")),
                }
            }
        }
    }
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
}

#[test]
fn every_widget_measure_is_finite() {
    let theme = crate::theme::Theme::dark();
    for (name, widget) in all_widgets() {
        let mut backend = RecordingBackend::default();
        let ctx = DrawContext::new(&mut backend, &theme);
        for &(w, h) in SIZES {
            let (mw, mh) = widget.measure(&ctx, w, h);
            // A widget may ask for more than it is offered, but never an
            // underflow-wrapped size.
            assert!(
                mw < 1 << 20 && mh < 1 << 20,
                "{name} at {w}x{h}: measured {mw}x{mh}"
            );
        }
    }
}
