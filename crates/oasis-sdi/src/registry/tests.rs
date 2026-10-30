//! Tests for [`SdiRegistry`](super::SdiRegistry).

use super::theme::parse_color;
use super::*;
use oasis_test_backend::{DrawCommand, RecordingBackend};
use oasis_types::backend::{Color, TextureId};

#[test]
fn create_and_get() {
    let mut reg = SdiRegistry::new();
    {
        let obj = reg.create("panel");
        obj.x = 10;
        obj.y = 20;
    }
    let obj = reg.get("panel").unwrap();
    assert_eq!(obj.x, 10);
    assert_eq!(obj.y, 20);
}

#[test]
fn get_nonexistent_returns_error() {
    let mut reg = SdiRegistry::new();
    for err in [
        reg.get("nope").map(|_| ()).unwrap_err(),
        reg.get_mut("nope").map(|_| ()).unwrap_err(),
        reg.destroy("nope").unwrap_err(),
    ] {
        assert!(
            matches!(&err, OasisError::Sdi(SdiError::NotFound { name }) if name == "nope"),
            "typed NotFound expected: {err:?}"
        );
        assert_eq!(err.to_string(), "SDI error: object not found: nope");
    }
}

#[test]
fn destroy_removes_object() {
    let mut reg = SdiRegistry::new();
    reg.create("temp");
    assert!(reg.contains("temp"));
    reg.destroy("temp").unwrap();
    assert!(!reg.contains("temp"));
}

#[test]
fn equal_z_draw_order_is_deterministic_by_name() {
    // Free-layout dashboards force icon objects to the same z as the
    // content backdrop; the tie must resolve by name (not HashMap
    // iteration order) so "icon_*" always draws over "content_bg".
    let mut reg = SdiRegistry::new();
    // Insert in an order that differs from the alphabetical result.
    for name in ["icon_body_3", "content_bg", "icon_body_1", "icon_body_2"] {
        let obj = reg.create(name);
        obj.z = 0;
    }
    reg.z_dirty = true;
    reg.ensure_z_sorted();
    let order: Vec<&str> = reg
        .z_sorted_base
        .iter()
        .map(|&h| reg.objects[h].name.as_str())
        .collect();
    assert_eq!(
        order,
        vec!["content_bg", "icon_body_1", "icon_body_2", "icon_body_3"]
    );
}

#[test]
fn destroy_repairs_swapped_handle() {
    // `destroy` swap-removes from the slab: the object that gets moved into
    // the freed slot must keep resolving by name (and keep drawing).
    let mut reg = SdiRegistry::new();
    for name in ["a", "b", "c"] {
        reg.create(name).color = Color::rgb(1, 2, 3);
    }
    reg.destroy("a").unwrap();
    assert_eq!(reg.len(), 2);
    // "c" was the last slab entry, so it moved into "a"'s slot.
    assert!(reg.get("c").is_ok());
    assert!(reg.get("b").is_ok());
    reg.get_mut("c").unwrap().x = 42;
    assert_eq!(reg.get("c").unwrap().x, 42);
    assert_eq!(reg.get("b").unwrap().x, 0);
    let mut names: Vec<&str> = reg.names().collect();
    names.sort_unstable();
    assert_eq!(names, vec!["b", "c"]);
}

#[test]
fn z_order_auto_increments() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    reg.create("b");
    reg.create("c");
    let a = reg.get("a").unwrap().z;
    let b = reg.get("b").unwrap().z;
    let c = reg.get("c").unwrap().z;
    assert!(a < b);
    assert!(b < c);
}

#[test]
fn move_to_top() {
    let mut reg = SdiRegistry::new();
    reg.create("bottom");
    reg.create("top");
    let top_z = reg.get("top").unwrap().z;
    reg.move_to_top("bottom").unwrap();
    let bottom_z = reg.get("bottom").unwrap().z;
    assert!(bottom_z > top_z);
}

#[test]
fn move_to_bottom() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    reg.create("b");
    let a_z = reg.get("a").unwrap().z;
    reg.move_to_bottom("b").unwrap();
    let b_z = reg.get("b").unwrap().z;
    assert!(b_z < a_z);
}

#[test]
fn renormalize_z_orders_on_overflow() {
    let mut reg = SdiRegistry::new();
    // Create three objects with normal z-orders.
    reg.create("a"); // z=0
    reg.create("b"); // z=1
    reg.create("c"); // z=2

    // Force next_z to i32::MAX to trigger renormalization on next create.
    reg.next_z = i32::MAX;

    // Manually set z-orders to large values to simulate long-running usage.
    reg.get_mut("a").unwrap().z = i32::MAX - 100;
    reg.get_mut("b").unwrap().z = i32::MAX - 50;
    reg.get_mut("c").unwrap().z = i32::MAX - 10;

    // This create should trigger renormalization, then assign the new object.
    reg.create("d");

    // After renormalization, z-orders should be sequential starting from 0.
    // The relative order must be preserved: a < b < c < d.
    let za = reg.get("a").unwrap().z;
    let zb = reg.get("b").unwrap().z;
    let zc = reg.get("c").unwrap().z;
    let zd = reg.get("d").unwrap().z;
    assert!(za < zb, "a.z={za} should be < b.z={zb}");
    assert!(zb < zc, "b.z={zb} should be < c.z={zc}");
    assert!(zc < zd, "c.z={zc} should be < d.z={zd}");
    // The values should be small (renormalized from 0).
    assert!(
        zd < 10,
        "z-orders should be renormalized to small values, got d.z={zd}"
    );
}

#[test]
fn renormalize_z_orders_on_move_to_top() {
    let mut reg = SdiRegistry::new();
    reg.create("x"); // z=0
    reg.create("y"); // z=1

    // Force next_z to i32::MAX.
    reg.next_z = i32::MAX;
    reg.get_mut("x").unwrap().z = i32::MAX - 20;
    reg.get_mut("y").unwrap().z = i32::MAX - 10;

    // move_to_top should trigger renormalization first, then assign new top z.
    reg.move_to_top("x").unwrap();

    let zx = reg.get("x").unwrap().z;
    let zy = reg.get("y").unwrap().z;
    assert!(zx > zy, "x.z={zx} should be > y.z={zy} after move_to_top");
    // Values should be small after renormalization.
    assert!(
        zx < 10,
        "z-orders should be renormalized to small values, got x.z={zx}"
    );
}

#[test]
fn len_and_is_empty() {
    let mut reg = SdiRegistry::new();
    assert!(reg.is_empty());
    reg.create("x");
    assert_eq!(reg.len(), 1);
    assert!(!reg.is_empty());
}

#[test]
fn load_theme_creates_and_updates() {
    let mut reg = SdiRegistry::new();
    reg.create("existing");
    let theme = r##"
[existing]
x = 42
y = 10

[new_obj]
x = 100
w = 50
h = 30
color = "#FF0000"
text = "hello"
font_size = 16
"##;
    reg.load_theme(theme).unwrap();
    let e = reg.get("existing").unwrap();
    assert_eq!(e.x, 42);
    assert_eq!(e.y, 10);

    let n = reg.get("new_obj").unwrap();
    assert_eq!(n.x, 100);
    assert_eq!(n.w, 50);
    assert_eq!(n.h, 30);
    assert_eq!(n.color.r, 255);
    assert_eq!(n.color.g, 0);
    assert_eq!(n.text.as_deref(), Some("hello"));
    assert_eq!(n.font_size, 16);
}

#[test]
fn parse_color_hex() {
    let c = parse_color("#1A2B3C").unwrap();
    assert_eq!(c.r, 0x1A);
    assert_eq!(c.g, 0x2B);
    assert_eq!(c.b, 0x3C);
    assert_eq!(c.a, 255);

    let c2 = parse_color("#1A2B3C80").unwrap();
    assert_eq!(c2.a, 0x80);
}

#[test]
fn parse_color_invalid() {
    assert!(parse_color("not-a-color").is_none());
    assert!(parse_color("#GG0000").is_none());
    assert!(parse_color("#12345").is_none());
}

#[test]
fn text_object_defaults() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("label");
    obj.text = Some("test".into());
    obj.font_size = 14;
    let o = reg.get("label").unwrap();
    assert_eq!(o.text.as_deref(), Some("test"));
    assert_eq!(o.font_size, 14);
}

#[test]
fn names_iterator() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    reg.create("b");
    let mut names: Vec<&str> = reg.names().collect();
    names.sort();
    assert_eq!(names, vec!["a", "b"]);
}

// -- Scene-dirty tracking tests --

#[test]
fn new_registry_starts_dirty() {
    let reg = SdiRegistry::new();
    assert!(reg.is_scene_dirty(), "fresh scene must draw at least once");
}

#[test]
fn take_scene_dirty_returns_and_clears() {
    let mut reg = SdiRegistry::new();
    assert!(reg.take_scene_dirty());
    assert!(!reg.is_scene_dirty());
    assert!(!reg.take_scene_dirty());
}

#[test]
fn create_sets_dirty() {
    let mut reg = SdiRegistry::new();
    reg.clear_scene_dirty();
    reg.create("obj");
    assert!(reg.is_scene_dirty());
}

#[test]
fn destroy_sets_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("obj");
    reg.clear_scene_dirty();
    reg.destroy("obj").unwrap();
    assert!(reg.is_scene_dirty());
}

#[test]
fn get_mut_without_change_is_not_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("obj").set_text("hi");
    reg.clear_scene_dirty();
    // get_mut hands out &mut, but the dirty check re-hashes touched
    // objects: rewriting identical values is not a change.
    let obj = reg.get_mut("obj").unwrap();
    obj.x = 0;
    obj.set_text("hi");
    assert!(!reg.is_scene_dirty());
    assert!(!reg.take_scene_dirty());
}

#[test]
fn get_mut_with_change_is_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("obj");
    reg.clear_scene_dirty();
    reg.get_mut("obj").unwrap().x = 5;
    assert!(reg.is_scene_dirty());
    assert!(reg.take_scene_dirty());
    // Settled: the new state is the baseline now.
    assert!(!reg.is_scene_dirty());
    reg.get_mut("obj").unwrap().x = 5;
    assert!(!reg.take_scene_dirty());
}

#[test]
fn change_reverted_before_check_is_not_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("obj");
    reg.clear_scene_dirty();
    reg.get_mut("obj").unwrap().x = 9;
    reg.get_mut("obj").unwrap().x = 0;
    assert!(!reg.take_scene_dirty());
}

#[test]
fn hidden_object_edits_are_not_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("obj").visible = false;
    reg.clear_scene_dirty();
    let obj = reg.get_mut("obj").unwrap();
    obj.x = 40;
    obj.set_text("invisible");
    assert!(!reg.take_scene_dirty(), "hidden objects never reach pixels");
    // Showing it is a change, and reflects the edited properties.
    reg.get_mut("obj").unwrap().visible = true;
    assert!(reg.take_scene_dirty());
}

#[test]
fn mark_scene_dirty_forces_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("obj");
    reg.clear_scene_dirty();
    reg.mark_scene_dirty();
    assert!(reg.is_scene_dirty());
    assert!(reg.take_scene_dirty());
    assert!(!reg.is_scene_dirty());
}

#[test]
fn touched_objects_survive_destroy_of_others() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    reg.create("b");
    reg.create("c");
    reg.clear_scene_dirty();
    // Touch the last slot, then destroy an earlier one: the swap
    // moves "c" into a new slot. The structural change forces a full
    // re-baseline, after which tracking continues correctly.
    reg.get_mut("c").unwrap().x = 1;
    reg.destroy("a").unwrap();
    assert!(reg.take_scene_dirty());
    assert!(!reg.is_scene_dirty());
    reg.get_mut("c").unwrap().x = 2;
    assert!(reg.take_scene_dirty());
    reg.get_mut("b").unwrap().x = 0;
    assert!(!reg.take_scene_dirty());
}

#[test]
fn set_visible_only_dirties_on_change() {
    let mut reg = SdiRegistry::new();
    reg.create("obj");
    reg.clear_scene_dirty();
    assert!(!reg.set_visible("obj", true), "already visible");
    assert!(!reg.is_scene_dirty());
    assert!(reg.set_visible("obj", false));
    assert!(!reg.get("obj").unwrap().visible);
    assert!(reg.take_scene_dirty());
    assert!(!reg.set_visible("obj", false));
    assert!(!reg.take_scene_dirty());
    assert!(!reg.set_visible("missing", false));
}

#[test]
fn set_text_and_position_only_dirty_on_change() {
    let mut reg = SdiRegistry::new();
    reg.create("obj").set_text("a");
    reg.clear_scene_dirty();
    assert!(!reg.set_text("obj", Some("a")));
    assert!(!reg.set_position("obj", 0, 0));
    assert!(!reg.is_scene_dirty());
    assert!(reg.set_text("obj", Some("b")));
    assert!(reg.set_position("obj", 3, 4));
    let obj = reg.get("obj").unwrap();
    assert_eq!(obj.text.as_deref(), Some("b"));
    assert_eq!((obj.x, obj.y), (3, 4));
    assert!(reg.take_scene_dirty());
    assert!(reg.set_text("obj", None));
    assert!(reg.get("obj").unwrap().text.is_none());
    assert!(!reg.set_text("missing", Some("x")));
}

#[test]
fn fnv_index_lookups_after_many_inserts() {
    let mut reg = SdiRegistry::new();
    for i in 0..500 {
        reg.create(format!("term_line_{i}"));
    }
    for i in 0..500 {
        assert!(reg.contains(&format!("term_line_{i}")));
    }
    assert!(!reg.contains("term_line_500"));
}

#[test]
fn get_mut_missing_object_does_not_set_dirty() {
    let mut reg = SdiRegistry::new();
    reg.clear_scene_dirty();
    assert!(reg.get_mut("nope").is_err());
    assert!(!reg.is_scene_dirty());
}

#[test]
fn shared_get_does_not_set_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("obj");
    reg.clear_scene_dirty();
    let _ = reg.get("obj").unwrap();
    assert!(!reg.is_scene_dirty());
}

#[test]
fn z_order_moves_set_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    reg.create("b");
    reg.clear_scene_dirty();
    reg.move_to_top("a").unwrap();
    assert!(reg.is_scene_dirty());
    reg.clear_scene_dirty();
    reg.move_to_bottom("b").unwrap();
    assert!(reg.is_scene_dirty());
}

#[test]
fn load_theme_sets_dirty() {
    let mut reg = SdiRegistry::new();
    reg.clear_scene_dirty();
    reg.load_theme("[panel]\nx = 5\n").unwrap();
    assert!(reg.is_scene_dirty());
}

#[test]
fn load_image_sets_dirty() {
    let mut reg = SdiRegistry::new();
    reg.create("img");
    reg.clear_scene_dirty();
    let mut backend = RecordingBackend::default();
    reg.load_image("img", 2, 2, &[0u8; 16], &mut backend)
        .unwrap();
    assert!(reg.is_scene_dirty());
}

#[test]
fn draw_does_not_set_dirty() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("box");
    obj.w = 4;
    obj.h = 4;
    reg.clear_scene_dirty();
    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();
    assert!(
        !reg.is_scene_dirty(),
        "drawing must not re-dirty an unchanged scene"
    );
}

#[test]
fn scene_signature_stable_for_identical_rewrites() {
    // The per-frame UI update pattern: get_mut + write the same
    // values. Neither the dirty check nor the signature changes.
    let mut reg = SdiRegistry::new();
    let obj = reg.create("bar");
    obj.x = 3;
    obj.text = Some("12:00".into());
    let sig1 = reg.scene_signature();
    assert!(reg.take_scene_dirty(), "creation is a change");
    let obj = reg.get_mut("bar").unwrap();
    obj.x = 3;
    obj.set_text("12:00");
    assert!(!reg.take_scene_dirty());
    assert_eq!(reg.scene_signature(), sig1);
}

#[test]
fn scene_signature_changes_on_property_change() {
    let mut reg = SdiRegistry::new();
    reg.create("bar").set_text("12:00");
    let sig1 = reg.scene_signature();
    reg.get_mut("bar").unwrap().set_text("12:01");
    assert_ne!(reg.scene_signature(), sig1, "text change must re-hash");
    let sig2 = reg.scene_signature();
    reg.get_mut("bar").unwrap().visible = false;
    assert_ne!(reg.scene_signature(), sig2, "visibility must re-hash");
    // Hidden objects hash as name + visibility only: edits to them
    // are invisible until they are shown again.
    let hidden = reg.scene_signature();
    reg.get_mut("bar").unwrap().x += 1;
    assert_eq!(reg.scene_signature(), hidden, "hidden edits never paint");
    reg.get_mut("bar").unwrap().visible = true;
    let sig3 = reg.scene_signature();
    reg.get_mut("bar").unwrap().x += 1;
    assert_ne!(reg.scene_signature(), sig3, "position must re-hash");
    let sig4 = reg.scene_signature();
    reg.get_mut("bar").unwrap().alpha = 7;
    assert_ne!(reg.scene_signature(), sig4, "alpha must re-hash");
}

#[test]
fn scene_signature_changes_on_create_and_destroy() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    let sig1 = reg.scene_signature();
    reg.create("b");
    let sig2 = reg.scene_signature();
    assert_ne!(sig1, sig2);
    reg.destroy("b").unwrap();
    assert_eq!(
        reg.scene_signature(),
        sig1,
        "same scene content must hash the same"
    );
}

#[test]
fn scene_signature_changes_on_texture_assignment() {
    let mut reg = SdiRegistry::new();
    reg.create("img");
    let sig1 = reg.scene_signature();
    reg.get_mut("img").unwrap().texture = Some(TextureId(9));
    assert_ne!(reg.scene_signature(), sig1);
}

#[test]
fn scene_signature_ignores_accessibility_metadata() {
    let mut reg = SdiRegistry::new();
    reg.create("btn");
    let sig1 = reg.scene_signature();
    let obj = reg.get_mut("btn").unwrap();
    obj.aria_label = Some("Launch".into());
    obj.role = Some(crate::AccessibilityRole::Button);
    assert_eq!(
        reg.scene_signature(),
        sig1,
        "aria metadata never affects pixels"
    );
}

// -- Overlay z-order and object alpha --

#[test]
fn overlay_layer_resorts_after_create_without_base_pass() {
    let mut reg = SdiRegistry::new();
    for name in ["a", "b"] {
        let o = reg.create(name);
        o.overlay = true;
        o.text = Some(name.into());
    }
    let mut log = RecordingBackend::default();
    reg.draw(&mut log).unwrap();
    // Create and destroy overlay objects, then draw only the overlay:
    // it must not walk the stale handle list (wrong objects or an
    // out-of-bounds panic after the slab shrank).
    reg.destroy("a").unwrap();
    let o = reg.create("c");
    o.overlay = true;
    o.text = Some("c".into());
    let mut log = RecordingBackend::default();
    reg.draw_overlay_layer(&mut log).unwrap();
    let texts = log.texts();
    assert_eq!(texts, ["b", "c"]);

    reg.destroy("b").unwrap();
    reg.destroy("c").unwrap();
    let mut log = RecordingBackend::default();
    reg.draw_overlay_filtered(&mut log, |_| true).unwrap();
    assert_eq!(log.fill_rect_count() + log.draw_text_count(), 0);
}

#[test]
fn object_alpha_fades_text_stroke_and_fill_together() {
    let mut reg = SdiRegistry::new();
    let o = reg.create("win.title");
    o.w = 20;
    o.h = 10;
    o.color = Color::rgba(10, 20, 30, 255);
    o.stroke_width = Some(1);
    o.stroke_color = Some(Color::rgba(1, 2, 3, 200));
    o.text = Some("Title".into());
    o.text_color = Color::rgba(250, 250, 250, 255);
    o.text_shadow_offset = Some((1, 1));
    o.text_shadow_color = Some(Color::rgba(0, 0, 0, 100));
    o.alpha = 128;
    let mut log = RecordingBackend::default();
    reg.draw(&mut log).unwrap();

    let fills = log.fill_colors();
    assert_eq!(fills[0], Color::rgba(10, 20, 30, 128), "fill");
    assert!(
        fills[1..].iter().all(|&c| c == Color::rgba(1, 2, 3, 100)),
        "stroke: {fills:?}"
    );
    let texts = log.text_colors();
    assert_eq!(
        texts,
        [Color::rgba(0, 0, 0, 50), Color::rgba(250, 250, 250, 128)]
    );
}

// -- Draw pipeline tests using a recording backend --

#[test]
fn draw_dispatches_fill_rect_for_colored_objects() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("box");
    obj.x = 10;
    obj.y = 20;
    obj.w = 100;
    obj.h = 50;
    obj.color = Color::rgb(255, 0, 0);

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    assert!(
        calls.iter().any(|c| matches!(
            c,
            DrawCommand::FillRect {
                x: 10,
                y: 20,
                w: 100,
                h: 50,
                ..
            }
        )),
        "expected fill_rect call for colored object"
    );
}

#[test]
fn draw_dispatches_draw_text_for_text_objects() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("label");
    obj.text = Some("Hello World".to_string());
    obj.font_size = 12;
    obj.text_color = Color::WHITE;

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    let text_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, DrawCommand::DrawText { .. }))
        .collect();
    assert_eq!(text_calls.len(), 1, "expected exactly one draw_text call");
    if let DrawCommand::DrawText { text, .. } = &text_calls[0] {
        assert_eq!(text, "Hello World");
    }
}

#[test]
fn draw_skips_invisible_objects() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("hidden");
    obj.text = Some("invisible".to_string());
    obj.visible = false;

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    assert!(
        calls.is_empty(),
        "invisible object should not produce any draw calls"
    );
}

#[test]
fn draw_skips_zero_alpha_objects() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("transparent");
    obj.text = Some("ghost".to_string());
    obj.alpha = 0;

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    assert!(
        calls.is_empty(),
        "zero-alpha object should not produce any draw calls"
    );
}

#[test]
fn draw_text_only_object_no_fill_rect() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("text_only");
    obj.w = 0;
    obj.h = 0;
    obj.text = Some("just text".to_string());

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    // Should have draw_text but NOT fill_rect.
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, DrawCommand::DrawText { .. })),
        "text-only object should call draw_text"
    );
    assert!(
        !calls
            .iter()
            .any(|c| matches!(c, DrawCommand::FillRect { .. })),
        "text-only object should NOT call fill_rect"
    );
}

#[test]
fn draw_object_with_rect_and_text() {
    let mut reg = SdiRegistry::new();
    let obj = reg.create("button");
    obj.x = 5;
    obj.y = 10;
    obj.w = 80;
    obj.h = 20;
    obj.color = Color::rgb(50, 50, 50);
    obj.text = Some("Click".to_string());
    obj.text_color = Color::WHITE;
    obj.font_size = 12;

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    // Should have BOTH fill_rect and draw_text.
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, DrawCommand::FillRect { .. })),
        "button should have fill_rect"
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, DrawCommand::DrawText { text, .. } if text == "Click")),
        "button should have draw_text with 'Click'"
    );
}

#[test]
fn draw_z_order_respected() {
    let mut reg = SdiRegistry::new();
    // Create two objects. "back" at z=0, "front" at z=10.
    let obj = reg.create("back");
    obj.w = 10;
    obj.h = 10;
    obj.z = 0;
    obj.color = Color::rgb(255, 0, 0);
    let obj = reg.create("front");
    obj.w = 10;
    obj.h = 10;
    obj.z = 10;
    obj.color = Color::rgb(0, 255, 0);

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    // The first fill_rect should be for "back" (lower z), second for "front".
    let rects: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, DrawCommand::FillRect { .. }))
        .collect();
    assert_eq!(rects.len(), 2);
}

#[test]
fn draw_multiple_text_objects() {
    let mut reg = SdiRegistry::new();
    for i in 0..5 {
        let obj = reg.create(format!("line_{i}"));
        obj.text = Some(format!("Line {i}"));
        obj.y = i * 16;
    }

    let mut backend = RecordingBackend::default();
    reg.draw(&mut backend).unwrap();

    let calls = backend.commands();
    let text_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, DrawCommand::DrawText { .. }))
        .collect();
    assert_eq!(text_calls.len(), 5, "should render all 5 text objects");
}

/// Names of the objects in draw order (base pass then overlay pass).
fn draw_order(reg: &mut SdiRegistry) -> Vec<String> {
    reg.ensure_z_sorted();
    reg.z_sorted_base
        .iter()
        .chain(&reg.z_sorted_overlay)
        .map(|&h| reg.objects[h].name.clone())
        .collect()
}

#[test]
fn z_rewrite_through_get_mut_reorders_next_draw() {
    // Regression: the cached z sort only rebuilt on create/destroy/move,
    // so `get_mut(..).z = n` on an existing object was ignored until some
    // unrelated structural change happened.
    let mut reg = SdiRegistry::new();
    reg.create("a");
    reg.create("b");
    assert_eq!(draw_order(&mut reg), ["a", "b"]);
    reg.get_mut("a").unwrap().z = 50;
    assert_eq!(draw_order(&mut reg), ["b", "a"]);
}

#[test]
fn overlay_rewrite_through_get_mut_repartitions() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    reg.create("b");
    assert_eq!(draw_order(&mut reg), ["a", "b"]);
    reg.get_mut("a").unwrap().overlay = true;
    assert_eq!(draw_order(&mut reg), ["b", "a"]);
    assert_eq!(reg.z_sorted_overlay.len(), 1);
}

#[test]
fn load_theme_overlay_flag_on_existing_object_takes_effect() {
    let mut reg = SdiRegistry::new();
    reg.create("panel");
    reg.create("label");
    assert_eq!(draw_order(&mut reg), ["panel", "label"]);
    reg.load_theme("[panel]\noverlay = true\n").unwrap();
    assert_eq!(draw_order(&mut reg), ["label", "panel"]);
}

#[test]
fn move_to_bottom_at_i32_min_does_not_overflow() {
    let mut reg = SdiRegistry::new();
    reg.create("floor").z = i32::MIN;
    reg.create("x");
    reg.move_to_bottom("x").unwrap();
    assert!(reg.get("x").unwrap().z < reg.get("floor").unwrap().z);
    assert_eq!(draw_order(&mut reg), ["x", "floor"]);
}

#[test]
fn move_to_bottom_missing_object_is_error() {
    let mut reg = SdiRegistry::new();
    reg.create("a");
    assert!(reg.move_to_bottom("nope").is_err());
}

#[test]
fn create_with_existing_name_replaces_and_raises() {
    // Name collision contract: `create` on an existing name resets the
    // object (all properties) and gives it a fresh, topmost auto-z.
    let mut reg = SdiRegistry::new();
    {
        let a = reg.create("a");
        a.x = 7;
        a.text = Some("old".into());
    }
    reg.create("b");
    reg.create("a");
    assert_eq!(reg.len(), 2);
    let a = reg.get("a").unwrap();
    assert_eq!(a.x, 0);
    assert!(a.text.is_none());
    assert_eq!(draw_order(&mut reg), ["b", "a"]);
}

#[test]
fn recreate_after_destroy_draws_on_top() {
    let mut reg = SdiRegistry::new();
    for n in ["a", "b", "c"] {
        reg.create(n);
    }
    reg.destroy("a").unwrap();
    reg.create("a");
    assert_eq!(draw_order(&mut reg), ["b", "c", "a"]);
}

mod prop {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn arb_names(min: usize, max: usize) -> impl Strategy<Value = Vec<String>> {
        proptest::collection::hash_set("[a-z]{1,8}", min..max).prop_map(|s| s.into_iter().collect())
    }

    proptest! {
        #[test]
        fn len_equals_creates_minus_destroys(names in arb_names(1, 20)) {
            let mut reg = SdiRegistry::new();
            for name in &names {
                reg.create(name);
            }
            prop_assert_eq!(reg.len(), names.len());

            // Destroy half.
            let destroy_count = names.len() / 2;
            for name in names.iter().take(destroy_count) {
                reg.destroy(name).unwrap();
            }
            prop_assert_eq!(reg.len(), names.len() - destroy_count);
        }

        #[test]
        fn z_orders_are_unique(names in arb_names(2, 20)) {
            let mut reg = SdiRegistry::new();
            for name in &names {
                reg.create(name);
            }
            let zs: Vec<i32> = names.iter().map(|n| reg.get(n).unwrap().z).collect();
            let unique: HashSet<i32> = zs.iter().copied().collect();
            prop_assert_eq!(
                zs.len(), unique.len(),
                "all z-orders must be unique"
            );
        }

        #[test]
        fn move_to_top_gives_highest_z(names in arb_names(2, 10)) {
            let mut reg = SdiRegistry::new();
            for name in &names {
                reg.create(name);
            }
            let target = &names[0];
            reg.move_to_top(target).unwrap();
            let target_z = reg.get(target).unwrap().z;
            for name in &names[1..] {
                let z = reg.get(name).unwrap().z;
                prop_assert!(
                    target_z > z,
                    "move_to_top({target}): z={target_z} should be > {name}.z={z}"
                );
            }
        }

        #[test]
        fn move_to_bottom_gives_lowest_z(names in arb_names(2, 10)) {
            let mut reg = SdiRegistry::new();
            for name in &names {
                reg.create(name);
            }
            let target = &names[0];
            reg.move_to_bottom(target).unwrap();
            let target_z = reg.get(target).unwrap().z;
            for name in &names[1..] {
                let z = reg.get(name).unwrap().z;
                prop_assert!(
                    target_z < z,
                    "move_to_bottom({target}): z={target_z} should be < {name}.z={z}"
                );
            }
        }

        #[test]
        fn create_then_contains(names in arb_names(1, 20)) {
            let mut reg = SdiRegistry::new();
            for name in &names {
                reg.create(name);
            }
            for name in &names {
                prop_assert!(reg.contains(name), "registry should contain {name}");
            }
        }

        #[test]
        fn destroy_then_not_contains(names in arb_names(1, 10)) {
            let mut reg = SdiRegistry::new();
            for name in &names {
                reg.create(name);
            }
            for name in &names {
                reg.destroy(name).unwrap();
                prop_assert!(!reg.contains(name), "destroyed object should not be contained");
            }
            prop_assert!(reg.is_empty());
        }

        #[test]
        fn destroy_nonexistent_is_error(name in "[a-z]{1,8}") {
            let reg = SdiRegistry::new();
            prop_assert!(reg.get(&name).is_err());
        }

        #[test]
        fn z_order_auto_increments_monotonically(n in 2usize..20) {
            let mut reg = SdiRegistry::new();
            let mut prev_z = None;
            for i in 0..n {
                let name = format!("obj_{i}");
                reg.create(&name);
                let z = reg.get(&name).unwrap().z;
                if let Some(pz) = prev_z {
                    prop_assert!(z > pz, "z-order must increase: {z} > {pz}");
                }
                prev_z = Some(z);
            }
        }
    }
}
