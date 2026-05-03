//! Integration tests for the paint pipeline.
//!
//! These tests live alongside the paint module so they can reach
//! private helpers (`has_text_content`) via `super::*`.

use super::*;
use crate::css::values::{BorderStyle, ComputedStyle, TransformFunction};
use crate::layout::box_model::{EdgeSizes, ListMarker, ListMarkerStyle, Rect, ReplacedContent};
use crate::test_utils::{DrawCall, MockBackend};
use oasis_types::backend::Color;

/// Default test viewport (480x272 at origin, no scroll).
fn test_vp() -> PaintViewport {
    PaintViewport {
        scroll_y: 0.0,
        scroll_x: 0.0,
        x: 0,
        y: 0,
        width: 480.0,
        height: 272.0,
        visible_height: 272.0,
        focused_node: None,
        counter_styles: Vec::new(),
    }
}

fn make_block(x: f32, y: f32, w: f32, h: f32, style: ComputedStyle) -> LayoutBox {
    let mut lb = LayoutBox::new(BoxType::Block, style, Some(0));
    lb.dimensions.content = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    lb
}

#[test]
fn transparent_background_skipped() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();
    assert_eq!(style.background_color.a, 0);

    let lb = make_block(0.0, 0.0, 100.0, 50.0, style);
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert_eq!(backend.fill_rect_count(), 0);
}

#[test]
fn opaque_background_painted() {
    let mut backend = MockBackend::new();
    let mut style = ComputedStyle::default();
    style.background_color = Color::rgb(255, 0, 0);

    let lb = make_block(10.0, 20.0, 100.0, 50.0, style);
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert!(backend.fill_rect_count() > 0);
    assert!(
        matches!(&&backend.calls[0], DrawCall::FillRect { .. }),
        "expected FillRect for background"
    );
    let DrawCall::FillRect { color, .. } = &&backend.calls[0] else {
        unreachable!()
    };
    assert_eq!(*color, Color::rgb(255, 0, 0));
}

#[test]
fn zero_width_borders_skipped() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();

    let lb = make_block(0.0, 0.0, 100.0, 50.0, style);
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert_eq!(backend.fill_rect_count(), 0);
}

#[test]
fn nonzero_borders_painted() {
    let mut backend = MockBackend::new();
    let mut style = ComputedStyle::default();
    style.border_top_width = 2.0;
    style.border_top_style = BorderStyle::Solid;
    style.border_top_color = Color::BLACK;

    let mut lb = make_block(10.0, 10.0, 100.0, 50.0, style);
    lb.dimensions.border = EdgeSizes {
        top: 2.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert_eq!(backend.fill_rect_count(), 1);
    assert!(
        matches!(&&backend.calls[0], DrawCall::FillRect { .. }),
        "expected border FillRect"
    );
    let DrawCall::FillRect { h, color, .. } = &&backend.calls[0] else {
        unreachable!()
    };
    assert_eq!(*h, 2);
    assert_eq!(*color, Color::BLACK);
}

#[test]
fn link_regions_recorded() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();

    let mut link_box = make_block(10.0, 10.0, 80.0, 16.0, style.clone());
    link_box.node = Some(5);

    let inline_child = LayoutBox::new(BoxType::Inline, style.clone(), None);
    link_box.children.push(inline_child);

    let mut root = make_block(0.0, 0.0, 480.0, 272.0, style);
    root.children.push(link_box);

    let mut link_map = HashMap::new();
    link_map.insert(5_usize, "https://example.com".to_string());

    let result = paint(&root, &mut backend, test_vp(), &link_map).unwrap();

    assert!(!result.links.is_empty());
    assert_eq!(result.links[0].href, "https://example.com");
    assert_eq!(result.links[0].node, 5);
}

#[test]
fn offscreen_above_viewport_culled() {
    let mut backend = MockBackend::new();
    let mut style = ComputedStyle::default();
    style.background_color = Color::rgb(255, 0, 0);

    let lb = make_block(0.0, -100.0, 100.0, 50.0, style);
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert_eq!(
        backend.calls.len(),
        0,
        "offscreen box above viewport should be culled"
    );
}

#[test]
fn offscreen_below_viewport_culled() {
    let mut backend = MockBackend::new();
    let mut style = ComputedStyle::default();
    style.background_color = Color::rgb(0, 255, 0);

    let lb = make_block(0.0, 500.0, 100.0, 50.0, style);
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert_eq!(
        backend.calls.len(),
        0,
        "offscreen box below viewport should be culled"
    );
}

#[test]
fn onscreen_box_not_culled() {
    let mut backend = MockBackend::new();
    let mut style = ComputedStyle::default();
    style.background_color = Color::rgb(0, 0, 255);

    let lb = make_block(0.0, 100.0, 100.0, 50.0, style);
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert!(!backend.calls.is_empty(), "onscreen box should be painted");
}

#[test]
fn list_marker_disc() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();

    let lb = LayoutBox::new(
        BoxType::ListItem {
            marker: ListMarker::Disc,
        },
        style,
        Some(0),
    );
    let link_map = HashMap::new();
    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert!(backend.draw_text_count() > 0);
    assert!(
        matches!(&&backend.calls[0], DrawCall::DrawText { .. }),
        "expected DrawText for disc marker"
    );
    let DrawCall::DrawText { text, .. } = &&backend.calls[0] else {
        unreachable!()
    };
    assert_eq!(text, "\u{2022}");
}

#[test]
fn list_marker_decimal() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();

    let lb = LayoutBox::new(
        BoxType::ListItem {
            marker: ListMarker::Ordered(ListMarkerStyle::Decimal, 3),
        },
        style,
        Some(0),
    );
    let link_map = HashMap::new();
    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert!(backend.draw_text_count() > 0);
    assert!(
        matches!(&&backend.calls[0], DrawCall::DrawText { .. }),
        "expected DrawText for decimal marker"
    );
    let DrawCall::DrawText { text, .. } = &&backend.calls[0] else {
        unreachable!()
    };
    assert_eq!(text, "3.");
}

#[test]
fn broken_image_placeholder() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();

    let mut lb = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::Image {
            width: 0,
            height: 0,
            texture: None,
            alt: String::new(),
            atlas_region: None,
        }),
        style,
        Some(0),
    );
    lb.dimensions.content = Rect {
        x: 10.0,
        y: 10.0,
        width: 8.0,
        height: 8.0,
    };
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    let fill_count = backend.fill_rect_count();
    let text_count = backend.draw_text_count();
    assert_eq!(fill_count, 4, "expected 4 border lines for placeholder");
    assert_eq!(text_count, 1, "expected 1 draw_text for placeholder symbol");

    if let DrawCall::FillRect { w, h, .. } = &backend.calls[0] {
        assert!(
            *w >= 16 || *h >= 1,
            "placeholder should enforce minimum size"
        );
    }
}

#[test]
fn broken_image_with_alt_text() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();

    let mut lb = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::Image {
            width: 0,
            height: 0,
            texture: None,
            alt: "Photo".to_string(),
            atlas_region: None,
        }),
        style,
        Some(0),
    );
    lb.dimensions.content = Rect {
        x: 10.0,
        y: 10.0,
        width: 32.0,
        height: 32.0,
    };
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    let text_call = backend
        .calls
        .iter()
        .find(|c| matches!(c, DrawCall::DrawText { .. }));
    assert!(text_call.is_some());
    if let DrawCall::DrawText { text, .. } = text_call.unwrap() {
        assert_eq!(text, "Photo");
    }
}

#[test]
fn content_height_reported() {
    let mut backend = MockBackend::new();
    let style = ComputedStyle::default();

    let mut lb = make_block(0.0, 0.0, 480.0, 500.0, style);
    lb.dimensions.margin = EdgeSizes {
        top: 10.0,
        right: 0.0,
        bottom: 10.0,
        left: 0.0,
    };
    let link_map = HashMap::new();

    let result = paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert!((result.content_height - 520.0).abs() < f32::EPSILON);
}

#[test]
fn link_highlight_draws_border() {
    let mut backend = MockBackend::new();
    let link = LinkRegion {
        rect: Rect {
            x: 50.0,
            y: 100.0,
            width: 80.0,
            height: 16.0,
        },
        href: "https://example.com".to_string(),
        node: 1,
    };

    paint_link_highlight(&link, &mut backend, Color::rgb(255, 255, 0)).unwrap();

    assert_eq!(backend.fill_rect_count(), 4);
}

#[test]
fn perspective_ancestor_routes_3d_child_through_polygon_path() {
    // The existing paint pipeline applies an element's transform
    // to its DESCENDANTS (the element's own background paints
    // before child_matrix is composed). So to exercise the
    // perspective projection path we need 3 levels:
    //   grandparent  – perspective: 800px (no own transform)
    //   parent       – rotateY(45deg)     (no background)
    //   child        – background: red    (no transform)
    // The parent's rotation under the grandparent's perspective
    // composes into ctx.transform, so child's paint_background
    // sees a non-trivial transform and goes through fill_polygon.
    let mut backend = MockBackend::new();

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(255, 0, 0);
    let child = make_block(60.0, 60.0, 80.0, 80.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::RotateY(45.0)];
    let mut parent = make_block(50.0, 50.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    let mut grandparent_style = ComputedStyle::default();
    grandparent_style.perspective = Some(800.0);
    let mut grandparent = make_block(0.0, 0.0, 200.0, 200.0, grandparent_style);
    grandparent.children.push(parent);

    paint(&grandparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let polygon = backend
        .polygon_calls()
        .into_iter()
        .find_map(|c| {
            if let DrawCall::FillPolygon { points, color } = c
                && *color == Color::rgb(255, 0, 0)
            {
                Some(points)
            } else {
                None
            }
        })
        .expect("expected red fill_polygon from perspective path");
    assert_eq!(polygon.len(), 4);
    let max_x = polygon.iter().map(|p| p.0).max().unwrap();
    assert!(
        max_x < 140,
        "expected perspective shrink past x=140, got max_x={max_x}",
    );
}

#[test]
fn flat_3d_child_without_perspective_uses_orthographic_path() {
    let mut backend = MockBackend::new();

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(0, 255, 0);
    let child = make_block(10.0, 10.0, 80.0, 80.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::RotateY(60.0)];
    let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    assert!(
        backend.fill_polygon_count() > 0,
        "expected fill_polygon for orthographically flattened rotateY",
    );
}

#[test]
fn backface_visibility_hidden_culls_rotated_subtree() {
    let mut backend = MockBackend::new();

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(0, 0, 255);
    let child = make_block(0.0, 0.0, 100.0, 100.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::RotateY(180.0)];
    parent_style.backface_visibility = BackfaceVisibility::Hidden;
    let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    assert_eq!(backend.fill_rect_count(), 0);
    assert_eq!(backend.fill_polygon_count(), 0);
}

#[test]
fn backface_hidden_child_culled_by_inherited_preserve_3d() {
    let mut backend = MockBackend::new();

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(255, 0, 0);
    child_style.backface_visibility = BackfaceVisibility::Hidden;
    let child = make_block(0.0, 0.0, 80.0, 80.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::RotateY(180.0)];
    parent_style.transform_style = TransformStyle::Preserve3d;
    let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    let mut gparent_style = ComputedStyle::default();
    gparent_style.perspective = Some(800.0);
    let mut gparent = make_block(0.0, 0.0, 200.0, 200.0, gparent_style);
    gparent.children.push(parent);

    paint(&gparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    assert_eq!(backend.fill_rect_count(), 0);
    assert_eq!(backend.fill_polygon_count(), 0);
}

#[test]
fn preserve_3d_propagates_parent_matrix_to_children() {
    let mut backend = MockBackend::new();

    let mut inner_style = ComputedStyle::default();
    inner_style.background_color = Color::rgb(255, 255, 0);
    let inner = make_block(10.0, 10.0, 40.0, 40.0, inner_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::TranslateZ(50.0)];
    let mut parent = make_block(20.0, 20.0, 60.0, 60.0, parent_style);
    parent.children.push(inner);

    let mut gparent_style = ComputedStyle::default();
    gparent_style.transforms = vec![TransformFunction::RotateY(30.0)];
    gparent_style.transform_style = TransformStyle::Preserve3d;
    let mut gparent = make_block(50.0, 50.0, 100.0, 100.0, gparent_style);
    gparent.children.push(parent);

    let mut ggparent_style = ComputedStyle::default();
    ggparent_style.perspective = Some(800.0);
    let mut ggparent = make_block(0.0, 0.0, 200.0, 200.0, ggparent_style);
    ggparent.children.push(gparent);

    paint(&ggparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let yellow_polygons: Vec<_> = backend
        .polygon_calls()
        .into_iter()
        .filter(|c| {
            matches!(
                c,
                DrawCall::FillPolygon { color, .. } if *color == Color::rgb(255, 255, 0)
            )
        })
        .collect();
    assert!(
        !yellow_polygons.is_empty(),
        "expected the inner child to be projected via fill_polygon under preserve-3d",
    );
}

#[test]
fn preserve_3d_propagates_without_ancestor_perspective() {
    let mut backend = MockBackend::new();

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(255, 0, 0);
    let child = make_block(10.0, 10.0, 80.0, 80.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::RotateY(60.0)];
    parent_style.transform_style = TransformStyle::Preserve3d;
    let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let red_polygons: Vec<_> = backend
        .polygon_calls()
        .into_iter()
        .filter(|c| {
            matches!(
                c,
                DrawCall::FillPolygon { color, .. } if *color == Color::rgb(255, 0, 0)
            )
        })
        .collect();
    assert!(
        !red_polygons.is_empty(),
        "preserve-3d parent without ancestor perspective should still propagate \
         its 3D matrix to descendants — regression guard",
    );
}

#[test]
fn preserve_3d_with_2d_only_transform_still_propagates() {
    let mut backend = MockBackend::new();

    let mut grandchild_style = ComputedStyle::default();
    grandchild_style.background_color = Color::rgb(0, 255, 0);
    let grandchild = make_block(5.0, 5.0, 30.0, 30.0, grandchild_style);

    let mut child_style = ComputedStyle::default();
    child_style.transforms = vec![TransformFunction::RotateY(60.0)];
    let mut child = make_block(20.0, 20.0, 40.0, 40.0, child_style);
    child.children.push(grandchild);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::Rotate(45.0)];
    parent_style.transform_style = TransformStyle::Preserve3d;
    let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let green_polygons: Vec<_> = backend
        .polygon_calls()
        .into_iter()
        .filter(|c| {
            matches!(
                c,
                DrawCall::FillPolygon { color, .. } if *color == Color::rgb(0, 255, 0)
            )
        })
        .collect();
    assert!(
        !green_polygons.is_empty(),
        "preserve-3d with a 2D-only parent transform should still establish a 3D \
         rendering context per CSS Transforms 2 §6",
    );
}

#[test]
fn near_camera_plane_background_skipped_not_saturated() {
    // Regression guard for the non-finite / overflow cast in
    // `paint_background`'s 3D projection path. When a point's
    // homogeneous `w` lands just above `apply_point_3d`'s
    // `1e-6` divide-by-zero threshold, the perspective divide
    // produces finite-but-astronomical coordinates that
    // saturate on the `as i32` cast to `i32::MAX`, painting a
    // screen-spanning garbage polygon.
    let mut backend = MockBackend::new();

    #[rustfmt::skip]
    let pathological = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        10000.0, 0.0, 0.0, 2.0e-6_f32,
    ];

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(77, 88, 99);
    let child = make_block(50.0, 50.0, 100.0, 100.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transform_style = TransformStyle::Preserve3d;
    parent_style.transforms = vec![TransformFunction::Matrix3d(pathological)];
    let mut parent = make_block(0.0, 0.0, 300.0, 300.0, parent_style);
    parent.children.push(child);

    paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let offending = backend.polygon_calls().into_iter().find(|c| {
        matches!(
            c,
            DrawCall::FillPolygon { color, .. } if *color == Color::rgb(77, 88, 99)
        )
    });
    assert!(
        offending.is_none(),
        "element near the camera plane must skip `fill_polygon` \
         instead of painting a saturated-cast garbage quad",
    );
}

#[test]
fn flat_2d_child_between_3d_ancestor_and_grandchild_composes_into_ambient() {
    let mut backend = MockBackend::new();

    let mut gchild_style = ComputedStyle::default();
    gchild_style.background_color = Color::rgb(200, 150, 100);
    let gchild = make_block(100.0, 100.0, 20.0, 20.0, gchild_style);

    let mut child_style = ComputedStyle::default();
    child_style.transforms = vec![TransformFunction::Rotate(90.0)];
    let mut child = make_block(100.0, 100.0, 100.0, 100.0, child_style);
    child.children.push(gchild);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::RotateY(0.0)];
    let mut parent = make_block(50.0, 50.0, 300.0, 300.0, parent_style);
    parent.children.push(child);

    let mut gp_style = ComputedStyle::default();
    gp_style.perspective = Some(1_000_000.0);
    let mut gp = make_block(0.0, 0.0, 400.0, 400.0, gp_style);
    gp.children.push(parent);

    paint(&gp, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let quad = backend
        .polygon_calls()
        .into_iter()
        .find_map(|c| {
            if let DrawCall::FillPolygon { points, color } = c
                && *color == Color::rgb(200, 150, 100)
            {
                Some(points)
            } else {
                None
            }
        })
        .expect("expected grandchild polygon");
    let cx = quad.iter().map(|p| p.0).sum::<i32>() as f32 / 4.0;
    let cy = quad.iter().map(|p| p.1).sum::<i32>() as f32 / 4.0;
    assert!(
        (cx - 190.0).abs() < 3.0,
        "grandchild centroid x={cx} (expected ≈190) — \
         intervening child rotate(90) should compose into ambient",
    );
    assert!(
        (cy - 110.0).abs() < 3.0,
        "grandchild centroid y={cy} (expected ≈110) — \
         intervening child rotate(90) should compose into ambient",
    );
}

#[test]
fn steep_perspective_produces_trapezoidal_quad() {
    let mut backend = MockBackend::new();

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(200, 100, 50);
    let child = make_block(0.0, 0.0, 100.0, 100.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::RotateY(75.0)];
    let mut parent = make_block(0.0, 0.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    let mut gparent_style = ComputedStyle::default();
    gparent_style.perspective = Some(200.0);
    let mut gparent = make_block(0.0, 0.0, 300.0, 300.0, gparent_style);
    gparent.children.push(parent);

    paint(&gparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let quad = backend
        .polygon_calls()
        .into_iter()
        .find_map(|c| {
            if let DrawCall::FillPolygon { points, color } = c
                && *color == Color::rgb(200, 100, 50)
            {
                Some(points)
            } else {
                None
            }
        })
        .expect("expected rotated child polygon");
    assert_eq!(quad.len(), 4);
    let top_len_sq = {
        let dx = (quad[1].0 - quad[0].0) as f32;
        let dy = (quad[1].1 - quad[0].1) as f32;
        dx * dx + dy * dy
    };
    let bottom_len_sq = {
        let dx = (quad[2].0 - quad[3].0) as f32;
        let dy = (quad[2].1 - quad[3].1) as f32;
        dx * dx + dy * dy
    };
    let ratio = top_len_sq.max(bottom_len_sq) / top_len_sq.min(bottom_len_sq);
    assert!(
        ratio > 1.02,
        "expected top/bottom edges to differ under perspective (ratio={ratio})",
    );
}

#[test]
fn preserve_3d_children_z_sorted_back_to_front() {
    let mut backend = MockBackend::new();

    let mut front_style = ComputedStyle::default();
    front_style.background_color = Color::rgb(0, 0, 255);
    front_style.transforms = vec![TransformFunction::TranslateZ(100.0)];
    let front = make_block(10.0, 10.0, 50.0, 50.0, front_style);

    let mut back_style = ComputedStyle::default();
    back_style.background_color = Color::rgb(255, 0, 0);
    back_style.transforms = vec![TransformFunction::TranslateZ(-100.0)];
    let back = make_block(10.0, 10.0, 50.0, 50.0, back_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transform_style = TransformStyle::Preserve3d;
    let mut parent = make_block(0.0, 0.0, 200.0, 200.0, parent_style);
    parent.children.push(front);
    parent.children.push(back);

    let mut gparent_style = ComputedStyle::default();
    gparent_style.perspective = Some(800.0);
    let mut gparent = make_block(0.0, 0.0, 400.0, 400.0, gparent_style);
    gparent.children.push(parent);

    paint(&gparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let fills: Vec<_> = backend
        .calls
        .iter()
        .filter_map(|c| {
            if let DrawCall::FillPolygon { color, .. } = c {
                Some(*color)
            } else {
                None
            }
        })
        .collect();
    let red_idx = fills
        .iter()
        .position(|c| *c == Color::rgb(255, 0, 0))
        .expect("red (back) should paint");
    let blue_idx = fills
        .iter()
        .position(|c| *c == Color::rgb(0, 0, 255))
        .expect("blue (front) should paint");
    assert!(
        red_idx < blue_idx,
        "preserve-3d: back child (red, translateZ(-100)) must paint \
         before front child (blue, translateZ(+100)); red_idx={red_idx}, \
         blue_idx={blue_idx}",
    );
}

#[test]
fn rotate_around_box_center_produces_symmetric_quad() {
    let mut backend = MockBackend::new();

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::Rotate(45.0)];
    let mut parent = make_block(100.0, 50.0, 80.0, 40.0, parent_style);

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(10, 20, 30);
    let child = make_block(100.0, 50.0, 80.0, 40.0, child_style);
    parent.children.push(child);

    paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let polygon = backend
        .polygon_calls()
        .into_iter()
        .find_map(|c| {
            if let DrawCall::FillPolygon { points, color } = c
                && *color == Color::rgb(10, 20, 30)
            {
                Some(points)
            } else {
                None
            }
        })
        .expect("expected fill_polygon for rotated box");
    assert_eq!(polygon.len(), 4);
    let cx = polygon.iter().map(|p| p.0).sum::<i32>() as f32 / 4.0;
    let cy = polygon.iter().map(|p| p.1).sum::<i32>() as f32 / 4.0;
    assert!(
        (cx - 140.0).abs() < 2.0,
        "rotated quad centroid x={cx} (expected ≈140)",
    );
    assert!(
        (cy - 70.0).abs() < 2.0,
        "rotated quad centroid y={cy} (expected ≈70)",
    );
}

#[test]
fn rotated_parent_does_not_shift_child_offset() {
    let mut backend = MockBackend::new();

    let mut child_style = ComputedStyle::default();
    child_style.background_color = Color::rgb(240, 240, 240);
    let child = make_block(110.0, 110.0, 20.0, 20.0, child_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transforms = vec![TransformFunction::Rotate(180.0)];
    let mut parent = make_block(100.0, 100.0, 100.0, 100.0, parent_style);
    parent.children.push(child);

    paint(&parent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let polygon = backend
        .polygon_calls()
        .into_iter()
        .find_map(|c| {
            if let DrawCall::FillPolygon { points, color } = c
                && *color == Color::rgb(240, 240, 240)
            {
                Some(points)
            } else {
                None
            }
        })
        .expect("expected fill_polygon for rotated child");
    let cx = polygon.iter().map(|p| p.0).sum::<i32>() as f32 / 4.0;
    let cy = polygon.iter().map(|p| p.1).sum::<i32>() as f32 / 4.0;
    assert!(
        (cx - 180.0).abs() < 2.0,
        "rotated child centroid x={cx} (expected ≈180)",
    );
    assert!(
        (cy - 180.0).abs() < 2.0,
        "rotated child centroid y={cy} (expected ≈180)",
    );
}

#[test]
fn has_text_content_inline() {
    let style = ComputedStyle::default();
    let lb = LayoutBox::new(BoxType::Inline, style, None);
    assert!(has_text_content(&lb));
}

#[test]
fn has_text_content_nested() {
    let style = ComputedStyle::default();
    let inner = LayoutBox::new(BoxType::Inline, style.clone(), None);
    let mut outer = LayoutBox::new(BoxType::Block, style, None);
    outer.children.push(inner);
    assert!(has_text_content(&outer));
}

#[test]
fn has_text_content_empty_block() {
    let style = ComputedStyle::default();
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(!has_text_content(&lb));
}

#[test]
fn horizontal_rule_painted() {
    let mut backend = MockBackend::new();
    let mut style = ComputedStyle::default();
    style.border_top_color = Color::rgb(128, 128, 128);

    let mut lb = LayoutBox::new(
        BoxType::Replaced(ReplacedContent::HorizontalRule),
        style,
        Some(0),
    );
    lb.dimensions.content = Rect {
        x: 0.0,
        y: 50.0,
        width: 480.0,
        height: 1.0,
    };
    let link_map = HashMap::new();

    paint(&lb, &mut backend, test_vp(), &link_map).unwrap();

    assert_eq!(backend.fill_rect_count(), 1);
    if let DrawCall::FillRect { w, h, color, .. } = &backend.calls[0] {
        assert_eq!(*w, 480);
        assert_eq!(*h, 1);
        assert_eq!(*color, Color::rgb(128, 128, 128));
    }
}

#[test]
fn static_position_no_stacking_context() {
    let style = ComputedStyle::default();
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(!creates_stacking_context(&lb));
}

#[test]
fn positioned_with_z_index_creates_stacking_context() {
    let mut style = ComputedStyle::default();
    style.position = Position::Relative;
    style.z_index = 1;
    style.z_index_auto = false;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
}

#[test]
fn opacity_creates_stacking_context() {
    let mut style = ComputedStyle::default();
    style.opacity = 0.5;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
}

#[test]
fn transform_creates_stacking_context() {
    let mut style = ComputedStyle::default();
    style.transforms = vec![TransformFunction::Translate(10.0, 0.0)];
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
}

#[test]
fn positioned_z_index_auto_no_stacking_context() {
    let mut style = ComputedStyle::default();
    style.position = Position::Relative;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(!creates_stacking_context(&lb));
}

#[test]
fn mix_blend_mode_triggers_compositing_layer() {
    let mut style = ComputedStyle::default();
    style.mix_blend_mode = crate::css::values::types::BlendMode::Multiply;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
    assert!(creates_compositing_layer(&lb));
}

#[test]
fn backdrop_filter_triggers_compositing_layer() {
    let mut style = ComputedStyle::default();
    style.backdrop_filters = vec![crate::css::values::FilterFunction::Blur(4.0)];
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
    assert!(creates_compositing_layer(&lb));
}

#[test]
fn box_level_filter_triggers_compositing_layer() {
    let mut style = ComputedStyle::default();
    style.filters = vec![crate::css::values::FilterFunction::Grayscale(1.0)];
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
    assert!(creates_compositing_layer(&lb));
}

#[test]
fn isolation_isolate_triggers_compositing_layer() {
    let mut style = ComputedStyle::default();
    style.isolation = crate::css::values::types::Isolation::Isolate;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
    assert!(creates_compositing_layer(&lb));
}

#[test]
fn will_change_triggers_compositing_layer() {
    let mut style = ComputedStyle::default();
    style.will_change_promotes_layer = true;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
    assert!(creates_compositing_layer(&lb));
}

#[test]
fn mask_image_triggers_compositing_layer() {
    use crate::css::values::types::{GradientDirection, GradientStop, LinearGradient};
    let mut style = ComputedStyle::default();
    style.mask_image = crate::css::values::BackgroundImage::Gradient(LinearGradient {
        direction: GradientDirection::ToBottom,
        repeating: false,
        stops: vec![
            GradientStop {
                color: Color::rgba(255, 255, 255, 255),
                position: 0.0,
            },
            GradientStop {
                color: Color::rgba(255, 255, 255, 0),
                position: 1.0,
            },
        ],
    });
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(
        creates_stacking_context(&lb),
        "mask-image must force a stacking context",
    );
    assert!(
        creates_compositing_layer(&lb),
        "mask-image must force an offscreen compositing layer",
    );
}

#[test]
fn plain_opacity_stays_on_fast_path() {
    let mut style = ComputedStyle::default();
    style.opacity = 0.5;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
    assert!(
        !creates_compositing_layer(&lb),
        "plain opacity must not allocate a render target",
    );
}

#[test]
fn positioned_z_index_zero_explicit_creates_stacking_context() {
    let mut style = ComputedStyle::default();
    style.position = Position::Relative;
    style.z_index = 0;
    style.z_index_auto = false;
    let lb = LayoutBox::new(BoxType::Block, style, None);
    assert!(creates_stacking_context(&lb));
}

#[test]
fn stacking_context_z_order() {
    let mut backend = MockBackend::new();
    let link_map = HashMap::new();

    let mut root_style = ComputedStyle::default();
    root_style.background_color = Color::rgba(0, 0, 0, 0);
    let mut root = make_block(0.0, 0.0, 480.0, 272.0, root_style);

    let mut style_a = ComputedStyle::default();
    style_a.background_color = Color::rgb(255, 0, 0);
    style_a.position = Position::Relative;
    style_a.z_index = 2;
    style_a.z_index_auto = false;
    let child_a = make_block(10.0, 10.0, 50.0, 50.0, style_a);

    let mut style_b = ComputedStyle::default();
    style_b.background_color = Color::rgb(0, 255, 0);
    style_b.position = Position::Relative;
    style_b.z_index = 1;
    style_b.z_index_auto = false;
    let child_b = make_block(10.0, 70.0, 50.0, 50.0, style_b);

    root.children.push(child_a);
    root.children.push(child_b);

    paint(&root, &mut backend, test_vp(), &link_map).unwrap();

    let fill_calls: Vec<_> = backend
        .calls
        .iter()
        .filter_map(|c| {
            if let DrawCall::FillRect { color, .. } = c {
                Some(*color)
            } else {
                None
            }
        })
        .collect();

    assert!(fill_calls.len() >= 2, "should have at least 2 fill rects");
    let green_idx = fill_calls.iter().position(|c| *c == Color::rgb(0, 255, 0));
    let red_idx = fill_calls.iter().position(|c| *c == Color::rgb(255, 0, 0));
    assert!(
        green_idx.is_some() && red_idx.is_some(),
        "both colors should be painted",
    );
    assert!(
        green_idx.expect("green") < red_idx.expect("red"),
        "z-index=1 (green) should be painted before z-index=2 (red)",
    );
}

#[test]
fn css21_painting_order_negative_normal_positioned_positive() {
    let mut backend = MockBackend::new();
    let link_map = HashMap::new();

    let mut root_style = ComputedStyle::default();
    root_style.background_color = Color::rgba(0, 0, 0, 0);
    let mut root = make_block(0.0, 0.0, 480.0, 272.0, root_style);

    let mut style_neg = ComputedStyle::default();
    style_neg.background_color = Color::rgb(0, 0, 255);
    style_neg.position = Position::Relative;
    style_neg.z_index = -1;
    style_neg.z_index_auto = false;
    let child_neg = make_block(10.0, 10.0, 50.0, 50.0, style_neg);

    let mut style_normal = ComputedStyle::default();
    style_normal.background_color = Color::rgb(255, 255, 255);
    let child_normal = make_block(10.0, 70.0, 50.0, 50.0, style_normal);

    let mut style_auto = ComputedStyle::default();
    style_auto.background_color = Color::rgb(255, 255, 0);
    style_auto.position = Position::Relative;
    let child_auto = make_block(10.0, 130.0, 50.0, 50.0, style_auto);

    let mut style_pos = ComputedStyle::default();
    style_pos.background_color = Color::rgb(255, 0, 0);
    style_pos.position = Position::Relative;
    style_pos.z_index = 1;
    style_pos.z_index_auto = false;
    let child_pos = make_block(10.0, 190.0, 50.0, 50.0, style_pos);

    root.children.push(child_pos);
    root.children.push(child_normal);
    root.children.push(child_neg);
    root.children.push(child_auto);

    paint(&root, &mut backend, test_vp(), &link_map).unwrap();

    let fill_calls: Vec<_> = backend
        .calls
        .iter()
        .filter_map(|c| {
            if let DrawCall::FillRect { color, .. } = c {
                Some(*color)
            } else {
                None
            }
        })
        .collect();

    let blue_idx = fill_calls
        .iter()
        .position(|c| *c == Color::rgb(0, 0, 255))
        .expect("blue (z=-1) should be painted");
    let white_idx = fill_calls
        .iter()
        .position(|c| *c == Color::rgb(255, 255, 255))
        .expect("white (normal) should be painted");
    let yellow_idx = fill_calls
        .iter()
        .position(|c| *c == Color::rgb(255, 255, 0))
        .expect("yellow (auto) should be painted");
    let red_idx = fill_calls
        .iter()
        .position(|c| *c == Color::rgb(255, 0, 0))
        .expect("red (z=1) should be painted");

    assert!(
        blue_idx < white_idx,
        "negative z-index (blue) should paint before normal flow (white)",
    );
    assert!(
        white_idx < yellow_idx,
        "normal flow (white) should paint before positioned-auto (yellow)",
    );
    assert!(
        yellow_idx < red_idx,
        "positioned-auto (yellow) should paint before positive z-index (red)",
    );
}

#[test]
fn preserve_3d_explicit_z_index_opts_out_of_z_sort() {
    // Inside a preserve-3d parent, a child with an explicit
    // z-index opts out of the 3D Z-sort and participates in
    // the regular CSS 2.1 stacking tiers instead.
    let mut backend = MockBackend::new();

    let mut a_style = ComputedStyle::default();
    a_style.background_color = Color::rgb(0, 255, 0);
    a_style.transforms = vec![TransformFunction::TranslateZ(100.0)];
    let child_a = make_block(10.0, 10.0, 50.0, 50.0, a_style);

    let mut b_style = ComputedStyle::default();
    b_style.background_color = Color::rgb(255, 0, 0);
    b_style.transforms = vec![TransformFunction::TranslateZ(-100.0)];
    let child_b = make_block(10.0, 70.0, 50.0, 50.0, b_style);

    let mut c_style = ComputedStyle::default();
    c_style.background_color = Color::rgb(0, 0, 255);
    c_style.position = Position::Relative;
    c_style.z_index = 5;
    c_style.z_index_auto = false;
    let child_c = make_block(10.0, 130.0, 50.0, 50.0, c_style);

    let mut parent_style = ComputedStyle::default();
    parent_style.transform_style = TransformStyle::Preserve3d;
    let mut parent = make_block(0.0, 0.0, 200.0, 200.0, parent_style);
    parent.children.push(child_a);
    parent.children.push(child_b);
    parent.children.push(child_c);

    let mut gparent_style = ComputedStyle::default();
    gparent_style.perspective = Some(800.0);
    let mut gparent = make_block(0.0, 0.0, 400.0, 400.0, gparent_style);
    gparent.children.push(parent);

    paint(&gparent, &mut backend, test_vp(), &HashMap::new()).unwrap();

    let fills: Vec<Color> = backend
        .calls
        .iter()
        .filter_map(|c| match c {
            DrawCall::FillRect { color, .. } => Some(*color),
            DrawCall::FillPolygon { color, .. } => Some(*color),
            _ => None,
        })
        .collect();

    let blue_idx = fills
        .iter()
        .position(|c| *c == Color::rgb(0, 0, 255))
        .expect("blue (z-index:5) should paint");
    let red_idx = fills
        .iter()
        .position(|c| *c == Color::rgb(255, 0, 0))
        .expect("red (back) should paint");
    let green_idx = fills
        .iter()
        .position(|c| *c == Color::rgb(0, 255, 0))
        .expect("green (front) should paint");

    assert!(
        red_idx < green_idx,
        "back child (red, translateZ(-100)) must paint before \
         front child (green, translateZ(+100)): red={red_idx}, green={green_idx}",
    );
    assert!(
        blue_idx > green_idx,
        "explicit z-index child (blue, z-index:5) must paint after \
         Z-sorted children: blue={blue_idx}, green={green_idx}",
    );
}
