use super::*;

/// Shorthand for building a zero-gap, zero-margin manager.
fn no_gap_manager(layout: TilingLayout) -> TilingManager {
    let mut mgr = TilingManager::with_layout(layout);
    mgr.config_mut().gap = 0;
    mgr.config_mut().margin = 0;
    mgr.config_mut().min_width = 1;
    mgr.config_mut().min_height = 1;
    mgr
}

// ── MasterStack ─────────────────────────────────────────────────

#[test]
fn master_stack_single_window_fills_area() {
    let mgr = no_gap_manager(TilingLayout::MasterStack);
    let tiles = mgr.compute_layout(&["a"], 800, 600);
    assert_eq!(tiles.len(), 1);
    assert_eq!(tiles[0].window_id, "a");
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 800,
            h: 600,
        }
    );
}

#[test]
fn master_stack_two_windows() {
    let mut mgr = no_gap_manager(TilingLayout::MasterStack);
    mgr.config_mut().master_ratio = 0.5;
    let tiles = mgr.compute_layout(&["m", "s"], 800, 600);
    assert_eq!(tiles.len(), 2);
    // Master takes left half.
    assert_eq!(tiles[0].window_id, "m");
    assert_eq!(tiles[0].geometry.x, 0);
    assert_eq!(tiles[0].geometry.w, 400);
    assert_eq!(tiles[0].geometry.h, 600);
    // Stack takes right half.
    assert_eq!(tiles[1].window_id, "s");
    assert_eq!(tiles[1].geometry.w, 400);
    assert_eq!(tiles[1].geometry.h, 600);
}

#[test]
fn master_stack_five_windows() {
    let mut mgr = no_gap_manager(TilingLayout::MasterStack);
    mgr.config_mut().master_ratio = 0.5;
    let ids: Vec<&str> = vec!["m", "a", "b", "c", "d"];
    let tiles = mgr.compute_layout(&ids, 800, 400);
    assert_eq!(tiles.len(), 5);
    // Master.
    assert_eq!(tiles[0].window_id, "m");
    assert_eq!(tiles[0].geometry.w, 400);
    assert_eq!(tiles[0].geometry.h, 400);
    // Stack: 4 windows split 400px vertically -> 100px each.
    for t in &tiles[1..] {
        assert_eq!(t.geometry.w, 400);
        assert!(t.geometry.h >= 100);
    }
}

// ── Grid ────────────────────────────────────────────────────────

#[test]
fn grid_four_windows_2x2() {
    let mgr = no_gap_manager(TilingLayout::Grid);
    let tiles = mgr.compute_layout(&["a", "b", "c", "d"], 800, 600);
    assert_eq!(tiles.len(), 4);
    // ceil(sqrt(4)) = 2 cols, 2 rows.
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 400,
            h: 300,
        }
    );
    assert_eq!(
        tiles[1].geometry,
        Geometry {
            x: 400,
            y: 0,
            w: 400,
            h: 300,
        }
    );
    assert_eq!(
        tiles[2].geometry,
        Geometry {
            x: 0,
            y: 300,
            w: 400,
            h: 300,
        }
    );
    assert_eq!(
        tiles[3].geometry,
        Geometry {
            x: 400,
            y: 300,
            w: 400,
            h: 300,
        }
    );
}

#[test]
fn grid_three_windows() {
    let mgr = no_gap_manager(TilingLayout::Grid);
    let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
    assert_eq!(tiles.len(), 3);
    // ceil(sqrt(3)) = 2 cols, ceil(3/2) = 2 rows.
    // Row 0: a(col0), b(col1). Row 1: c(col0).
    assert_eq!(tiles[0].geometry.x, 0);
    assert_eq!(tiles[1].geometry.x, 400);
    assert_eq!(tiles[2].geometry.x, 0);
    assert_eq!(tiles[2].geometry.y, 300);
    // The lone last-row window spans the full width (it used to take one
    // cell and leave the other 400x300 empty).
    assert_eq!(tiles[2].geometry.w, 800);
}

/// Sum of tile areas, and whether any two tiles overlap.
fn coverage(tiles: &[TileGeometry]) -> (u64, bool) {
    let area = tiles
        .iter()
        .map(|t| u64::from(t.geometry.w) * u64::from(t.geometry.h))
        .sum();
    let overlaps = tiles.iter().enumerate().any(|(i, a)| {
        tiles[i + 1..].iter().any(|b| {
            let (a, b) = (a.geometry, b.geometry);
            a.x < b.x + b.w as i32
                && b.x < a.x + a.w as i32
                && a.y < b.y + b.h as i32
                && b.y < a.y + a.h as i32
        })
    });
    (area, overlaps)
}

#[test]
fn gapless_layouts_cover_odd_areas_exactly() {
    let ids = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    for layout in [
        TilingLayout::MasterStack,
        TilingLayout::Grid,
        TilingLayout::Columns,
        TilingLayout::Rows,
    ] {
        let mut mgr = no_gap_manager(layout);
        mgr.config_mut().min_width = 1;
        mgr.config_mut().min_height = 1;
        for (w, h) in [(801, 599), (479, 271), (1023, 767), (7, 5)] {
            for n in 1..=ids.len() {
                if n as u32 > w.min(h) {
                    continue; // narrower than one pixel per tile
                }
                let tiles = mgr.compute_layout(&ids[..n], w, h);
                let (area, overlaps) = coverage(&tiles);
                assert!(!overlaps, "{layout:?} n={n} {w}x{h}: overlap");
                assert_eq!(
                    area,
                    u64::from(w) * u64::from(h),
                    "{layout:?} n={n} {w}x{h}: lost pixels"
                );
            }
        }
    }
}

#[test]
fn grid_single_window_fills_area() {
    let mgr = no_gap_manager(TilingLayout::Grid);
    let tiles = mgr.compute_layout(&["x"], 480, 272);
    assert_eq!(tiles.len(), 1);
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 480,
            h: 272,
        }
    );
}

// ── Columns ─────────────────────────────────────────────────────

#[test]
fn columns_three_windows() {
    let mgr = no_gap_manager(TilingLayout::Columns);
    let tiles = mgr.compute_layout(&["a", "b", "c"], 900, 600);
    assert_eq!(tiles.len(), 3);
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 300,
            h: 600,
        }
    );
    assert_eq!(
        tiles[1].geometry,
        Geometry {
            x: 300,
            y: 0,
            w: 300,
            h: 600,
        }
    );
    assert_eq!(
        tiles[2].geometry,
        Geometry {
            x: 600,
            y: 0,
            w: 300,
            h: 600,
        }
    );
}

#[test]
fn columns_single_fills_area() {
    let mgr = no_gap_manager(TilingLayout::Columns);
    let tiles = mgr.compute_layout(&["a"], 480, 272);
    assert_eq!(tiles.len(), 1);
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 480,
            h: 272,
        }
    );
}

// ── Rows ────────────────────────────────────────────────────────

#[test]
fn rows_three_windows() {
    let mgr = no_gap_manager(TilingLayout::Rows);
    let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
    assert_eq!(tiles.len(), 3);
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 800,
            h: 200,
        }
    );
    assert_eq!(
        tiles[1].geometry,
        Geometry {
            x: 0,
            y: 200,
            w: 800,
            h: 200,
        }
    );
    assert_eq!(
        tiles[2].geometry,
        Geometry {
            x: 0,
            y: 400,
            w: 800,
            h: 200,
        }
    );
}

#[test]
fn rows_single_fills_area() {
    let mgr = no_gap_manager(TilingLayout::Rows);
    let tiles = mgr.compute_layout(&["a"], 480, 272);
    assert_eq!(tiles.len(), 1);
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 480,
            h: 272,
        }
    );
}

// ── Monocle ─────────────────────────────────────────────────────

#[test]
fn monocle_all_same_geometry() {
    let mgr = no_gap_manager(TilingLayout::Monocle);
    let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
    assert_eq!(tiles.len(), 3);
    let expected = Geometry {
        x: 0,
        y: 0,
        w: 800,
        h: 600,
    };
    for t in &tiles {
        assert_eq!(t.geometry, expected);
    }
}

#[test]
fn monocle_single_window() {
    let mgr = no_gap_manager(TilingLayout::Monocle);
    let tiles = mgr.compute_layout(&["only"], 480, 272);
    assert_eq!(tiles.len(), 1);
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 480,
            h: 272,
        }
    );
}

// ── Gaps and margins ────────────────────────────────────────────

#[test]
fn gap_spacing_columns() {
    let mut mgr = TilingManager::with_layout(TilingLayout::Columns);
    mgr.config_mut().gap = 10;
    mgr.config_mut().margin = 0;
    mgr.config_mut().min_width = 1;
    mgr.config_mut().min_height = 1;
    let tiles = mgr.compute_layout(&["a", "b"], 810, 600);
    assert_eq!(tiles.len(), 2);
    // (810 - 10 gap) / 2 = 400 each.
    assert_eq!(tiles[0].geometry.x, 0);
    assert_eq!(tiles[0].geometry.w, 400);
    // Second column starts at 400 + 10 = 410.
    assert_eq!(tiles[1].geometry.x, 410);
    assert_eq!(tiles[1].geometry.w, 400);
}

#[test]
fn margin_from_edges() {
    let mut mgr = TilingManager::with_layout(TilingLayout::Monocle);
    mgr.config_mut().gap = 0;
    mgr.config_mut().margin = 20;
    mgr.config_mut().min_width = 1;
    mgr.config_mut().min_height = 1;
    let tiles = mgr.compute_layout(&["a"], 800, 600);
    assert_eq!(tiles.len(), 1);
    // 800 - 40 margin = 760, starts at x=20.
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 20,
            y: 20,
            w: 760,
            h: 560,
        }
    );
}

#[test]
fn gap_and_margin_rows() {
    let mut mgr = TilingManager::with_layout(TilingLayout::Rows);
    mgr.config_mut().gap = 8;
    mgr.config_mut().margin = 10;
    mgr.config_mut().min_width = 1;
    mgr.config_mut().min_height = 1;
    let tiles = mgr.compute_layout(&["a", "b"], 400, 228);
    // Usable: 380 x 208. Row height = (208 - 8) / 2 = 100 each.
    assert_eq!(tiles.len(), 2);
    assert_eq!(tiles[0].geometry.x, 10);
    assert_eq!(tiles[0].geometry.y, 10);
    assert_eq!(tiles[0].geometry.w, 380);
    assert_eq!(tiles[0].geometry.h, 100);
    assert_eq!(tiles[1].geometry.x, 10);
    assert_eq!(tiles[1].geometry.y, 118); // 10 + 100 + 8
    assert_eq!(tiles[1].geometry.w, 380);
    assert_eq!(tiles[1].geometry.h, 100); // remainder absorbed
}

// ── Empty window list ───────────────────────────────────────────

#[test]
fn empty_window_list() {
    let mgr = TilingManager::new();
    let tiles = mgr.compute_layout(&[], 800, 600);
    assert!(tiles.is_empty());
}

// ── Floating overrides ──────────────────────────────────────────

#[test]
fn floating_override_excludes_window() {
    let mut mgr = no_gap_manager(TilingLayout::Columns);
    mgr.set_floating("b", true);
    let tiles = mgr.compute_layout(&["a", "b", "c"], 800, 600);
    assert_eq!(tiles.len(), 2);
    let ids: Vec<&str> = tiles.iter().map(|t| t.window_id.as_str()).collect();
    assert!(ids.contains(&"a"));
    assert!(!ids.contains(&"b"));
    assert!(ids.contains(&"c"));
}

#[test]
fn floating_override_can_be_removed() {
    let mut mgr = no_gap_manager(TilingLayout::Columns);
    mgr.set_floating("b", true);
    assert!(mgr.is_floating("b"));
    mgr.set_floating("b", false);
    assert!(!mgr.is_floating("b"));
    let tiles = mgr.compute_layout(&["a", "b"], 800, 600);
    assert_eq!(tiles.len(), 2);
}

#[test]
fn all_windows_floating_returns_empty() {
    let mut mgr = no_gap_manager(TilingLayout::Rows);
    mgr.set_floating("a", true);
    mgr.set_floating("b", true);
    let tiles = mgr.compute_layout(&["a", "b"], 800, 600);
    assert!(tiles.is_empty());
}

// ── cycle_layout ────────────────────────────────────────────────

#[test]
fn cycle_layout_full_cycle() {
    let l = TilingLayout::MasterStack;
    let l = cycle_layout(l);
    assert_eq!(l, TilingLayout::Grid);
    let l = cycle_layout(l);
    assert_eq!(l, TilingLayout::Columns);
    let l = cycle_layout(l);
    assert_eq!(l, TilingLayout::Rows);
    let l = cycle_layout(l);
    assert_eq!(l, TilingLayout::Monocle);
    let l = cycle_layout(l);
    assert_eq!(l, TilingLayout::MasterStack);
}

// ── adjust_master_ratio ─────────────────────────────────────────

#[test]
fn adjust_master_ratio_clamp_high() {
    let mut cfg = TilingConfig {
        master_ratio: 0.85,
        ..Default::default()
    };
    adjust_master_ratio(&mut cfg, 0.2);
    assert!((cfg.master_ratio - 0.9).abs() < f32::EPSILON);
}

#[test]
fn adjust_master_ratio_clamp_low() {
    let mut cfg = TilingConfig {
        master_ratio: 0.15,
        ..Default::default()
    };
    adjust_master_ratio(&mut cfg, -0.1);
    assert!((cfg.master_ratio - 0.1).abs() < 0.01);
}

#[test]
fn adjust_master_ratio_normal() {
    let mut cfg = TilingConfig {
        master_ratio: 0.5,
        ..Default::default()
    };
    adjust_master_ratio(&mut cfg, 0.05);
    assert!((cfg.master_ratio - 0.55).abs() < f32::EPSILON);
}

// ── Minimum size enforcement ────────────────────────────────────

#[test]
fn minimum_size_enforced_columns() {
    let mut mgr = TilingManager::with_layout(TilingLayout::Columns);
    mgr.config_mut().gap = 0;
    mgr.config_mut().margin = 0;
    mgr.config_mut().min_width = 200;
    mgr.config_mut().min_height = 100;
    // 10 columns in 400px -> 40px each, below min_width of 200.
    let ids: Vec<&str> = (0..10).map(|_| "w").collect();
    let tiles = mgr.compute_layout(&ids, 400, 50);
    for t in &tiles {
        assert!(t.geometry.w >= 200);
        assert!(t.geometry.h >= 100);
    }
}

#[test]
fn minimum_size_enforced_rows() {
    let mut mgr = TilingManager::with_layout(TilingLayout::Rows);
    mgr.config_mut().gap = 0;
    mgr.config_mut().margin = 0;
    mgr.config_mut().min_width = 80;
    mgr.config_mut().min_height = 60;
    // 20 rows in 100px -> 5px each, below min_height of 60.
    let ids: Vec<&str> = (0..20).map(|_| "w").collect();
    let tiles = mgr.compute_layout(&ids, 50, 100);
    for t in &tiles {
        assert!(t.geometry.w >= 80);
        assert!(t.geometry.h >= 60);
    }
}

// ── Config mutation ─────────────────────────────────────────────

#[test]
fn config_mutation() {
    let mut mgr = TilingManager::new();
    mgr.config_mut().gap = 16;
    mgr.config_mut().margin = 8;
    mgr.config_mut().master_ratio = 0.6;
    assert_eq!(mgr.config().gap, 16);
    assert_eq!(mgr.config().margin, 8);
    assert!((mgr.config().master_ratio - 0.6).abs() < f32::EPSILON);
}

// ── Default trait ───────────────────────────────────────────────

#[test]
fn default_manager_is_master_stack() {
    let mgr = TilingManager::default();
    assert_eq!(mgr.layout(), TilingLayout::MasterStack);
}

// ── with_layout constructor ─────────────────────────────────────

#[test]
fn with_layout_sets_layout() {
    let mgr = TilingManager::with_layout(TilingLayout::Grid);
    assert_eq!(mgr.layout(), TilingLayout::Grid);
}

// ── set_layout ──────────────────────────────────────────────────

#[test]
fn set_layout_changes_layout() {
    let mut mgr = TilingManager::new();
    mgr.set_layout(TilingLayout::Monocle);
    assert_eq!(mgr.layout(), TilingLayout::Monocle);
}

// ── ceil_sqrt helper ────────────────────────────────────────────

#[test]
fn ceil_sqrt_values() {
    assert_eq!(ceil_sqrt(0), 0);
    assert_eq!(ceil_sqrt(1), 1);
    assert_eq!(ceil_sqrt(2), 2);
    assert_eq!(ceil_sqrt(3), 2);
    assert_eq!(ceil_sqrt(4), 2);
    assert_eq!(ceil_sqrt(5), 3);
    assert_eq!(ceil_sqrt(9), 3);
    assert_eq!(ceil_sqrt(10), 4);
}

// ── Grid: 9 windows = 3x3 ──────────────────────────────────────

#[test]
fn grid_nine_windows_3x3() {
    let mgr = no_gap_manager(TilingLayout::Grid);
    let ids: Vec<&str> = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i"];
    let tiles = mgr.compute_layout(&ids, 900, 600);
    assert_eq!(tiles.len(), 9);
    // 3 cols x 3 rows. Cell: 300 x 200.
    assert_eq!(
        tiles[0].geometry,
        Geometry {
            x: 0,
            y: 0,
            w: 300,
            h: 200,
        }
    );
    assert_eq!(
        tiles[4].geometry,
        Geometry {
            x: 300,
            y: 200,
            w: 300,
            h: 200,
        }
    );
    assert_eq!(
        tiles[8].geometry,
        Geometry {
            x: 600,
            y: 400,
            w: 300,
            h: 200,
        }
    );
}

#[test]
fn overcrowded_layouts_never_wrap_tile_sizes() {
    // With more gaps than area, the last tile's start lies past the
    // usable area; its "remainder" size must clamp to the minimum
    // instead of wrapping to ~4 billion pixels.
    let ids: Vec<String> = (0..41).map(|i| format!("w{i}")).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    for layout in [
        TilingLayout::MasterStack,
        TilingLayout::Grid,
        TilingLayout::Columns,
        TilingLayout::Rows,
    ] {
        let mgr = TilingManager::with_layout(layout);
        for tile in mgr.compute_layout(&ids, 100, 100) {
            let g = tile.geometry;
            assert!(g.w <= 100 && g.h <= 100, "{layout:?}: {g:?}");
        }
    }
}

#[test]
fn huge_gap_and_margin_do_not_overflow_or_leave_the_area() {
    // `margin * 2`, `gap * (n - 1)` and `(cell + gap) as i32 * idx` used
    // to overflow (a debug-build panic) for large configured values.
    for layout in [
        TilingLayout::MasterStack,
        TilingLayout::Grid,
        TilingLayout::Columns,
        TilingLayout::Rows,
        TilingLayout::Monocle,
    ] {
        for (gap, margin) in [
            (u32::MAX, 0),
            (0, u32::MAX),
            (u32::MAX, u32::MAX),
            (500, 300),
        ] {
            let mut mgr = TilingManager::with_layout(layout);
            mgr.config_mut().gap = gap;
            mgr.config_mut().margin = margin;
            mgr.config_mut().min_width = 0;
            mgr.config_mut().min_height = 0;
            let tiles = mgr.compute_layout(&["a", "b", "c", "d", "e"], 800, 600);
            assert_eq!(tiles.len(), 5);
            for t in &tiles {
                let g = &t.geometry;
                assert!(g.x >= 0 && g.y >= 0, "{layout:?} {gap} {margin}: {g:?}");
                assert!(
                    i64::from(g.x) + i64::from(g.w) <= 800
                        && i64::from(g.y) + i64::from(g.h) <= 600,
                    "{layout:?} gap={gap} margin={margin}: {g:?} leaves the area"
                );
            }
        }
    }
}
