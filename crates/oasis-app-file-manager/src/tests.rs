//! File Manager unit tests (navigation, file ops, dialogs, views, clicks).

use super::*;
use crate::model::{EntryKind, build_tree_entries, list_panel};
use crate::view::compute_explorer_geom;
use oasis_app_core::testing::click_at;
use oasis_vfs::MemoryVfs;

fn setup_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.mkdir("/home/user/music").unwrap();
    vfs.mkdir("/home/user/photos").unwrap();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/tmp").unwrap();
    vfs.write("/home/user/readme.txt", b"Hello!").unwrap();
    vfs.write("/etc/hostname", b"oasis").unwrap();
    vfs
}

#[test]
fn parent_dir_root() {
    assert_eq!(parent_dir("/"), "/");
}

#[test]
fn parent_dir_one_level() {
    assert_eq!(parent_dir("/home"), "/");
}

#[test]
fn parent_dir_two_levels() {
    assert_eq!(parent_dir("/home/user"), "/home");
}

#[test]
fn join_path_root() {
    assert_eq!(join_path("/", "home"), "/home");
}

#[test]
fn join_path_nested() {
    assert_eq!(join_path("/home", "user"), "/home/user");
}

#[test]
fn list_directory_root_no_dotdot() {
    let vfs = setup_vfs();
    let lines = list_directory(&vfs, "/");
    assert!(!lines.iter().any(|l| l == ".."));
    assert!(lines.iter().any(|l| l.starts_with("home")));
}

#[test]
fn list_directory_subdir_has_dotdot() {
    let vfs = setup_vfs();
    let lines = list_directory(&vfs, "/home");
    assert!(lines.iter().any(|l| l == ".."));
}

#[test]
fn list_directory_shows_sizes() {
    let vfs = setup_vfs();
    let lines = list_directory(&vfs, "/home/user");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("readme.txt") && l.contains("6 B"))
    );
}

#[test]
fn file_manager_launch() {
    let vfs = setup_vfs();
    let fm = FileManagerApp::new("/apps/fm", &vfs);
    assert_eq!(fm.title(), "File Manager");
    assert!(fm.browse_dir().is_some());
    assert!(!fm.lines().is_empty());
    assert!(fm.lines().iter().any(|l| l.contains("home")));
}

#[test]
fn file_manager_navigate_down() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.view_mode = ViewMode::Dual;
    assert_eq!(fm.panels[0].cursor, 0);
    fm.handle_input(&Button::Down, &vfs);
    assert_eq!(fm.panels[0].cursor, 1);
}

#[test]
fn file_manager_switch_panel() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.view_mode = ViewMode::Dual;
    assert_eq!(fm.active_panel, 0);
    fm.handle_input(&Button::Right, &vfs);
    assert_eq!(fm.active_panel, 1);
    fm.handle_input(&Button::Left, &vfs);
    assert_eq!(fm.active_panel, 0);
}

#[test]
fn file_manager_enter_directory() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    let home_idx = fm.panels[0]
        .lines
        .iter()
        .position(|l| l.starts_with("home"))
        .expect("home/ should be in listing");
    for _ in 0..home_idx {
        fm.handle_input(&Button::Down, &vfs);
    }
    fm.handle_input(&Button::Confirm, &vfs);
    assert_eq!(fm.panels[0].browse_dir, "/home");
}

#[test]
fn file_manager_cancel_goes_up() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    // Enter /home first.
    let home_idx = fm.panels[0]
        .lines
        .iter()
        .position(|l| l.starts_with("home"))
        .expect("home/ should be in listing");
    for _ in 0..home_idx {
        fm.handle_input(&Button::Down, &vfs);
    }
    fm.handle_input(&Button::Confirm, &vfs);
    assert_eq!(fm.panels[0].browse_dir, "/home");

    let action = fm.handle_input(&Button::Cancel, &vfs);
    assert_eq!(action, AppAction::None);
    assert_eq!(fm.panels[0].browse_dir, "/");
}

#[test]
fn file_manager_cancel_at_root_exits() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    let action = fm.handle_input(&Button::Cancel, &vfs);
    assert_eq!(action, AppAction::Exit);
}

#[test]
fn file_manager_open_file() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.open_file(&vfs, "/home/user/readme.txt");
    assert!(fm.viewing_file().is_some());
    assert!(fm.lines().iter().any(|l| l.contains("Hello!")));
}

#[test]
fn file_manager_cancel_from_viewer() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.open_file(&vfs, "/home/user/readme.txt");
    assert!(fm.viewing_file().is_some());
    let action = fm.handle_input(&Button::Cancel, &vfs);
    assert_eq!(action, AppAction::None);
    assert!(fm.viewing_file().is_none());
}

#[test]
fn file_manager_open_nonexistent_noop() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.open_file(&vfs, "/does/not/exist.txt");
    assert!(fm.viewing_file().is_none());
}

#[test]
fn view_generic_text_file() {
    let lines = view_generic_file("/test.txt", b"Hello\nWorld");
    assert!(lines.iter().any(|l| l.contains("Hello")));
    assert!(lines.iter().any(|l| l.contains("World")));
}

#[test]
fn view_generic_binary_file() {
    let lines = view_generic_file("/data.bin", &[0x00, 0x01, 0xFF, 0xFE, 0x80]);
    assert!(lines.iter().any(|l| l.contains("Binary file")));
}

#[test]
fn view_audio_wav() {
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&36u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&44100u32.to_le_bytes());
    wav.extend_from_slice(&176400u32.to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&0u32.to_le_bytes());

    let lines = view_audio_file("/music/test.wav", &wav);
    assert!(lines.iter().any(|l| l.contains("WAV")));
    assert!(lines.iter().any(|l| l.contains("44100")));
}

#[test]
fn view_audio_mp3() {
    let data = vec![0xFF, 0xFB, 0x90, 0x00, 0x00];
    let lines = view_audio_file("/music/song.mp3", &data);
    assert!(lines.iter().any(|l| l.contains("MP3")));
}

#[test]
fn view_image_png() {
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&480u32.to_be_bytes());
    png.extend_from_slice(&272u32.to_be_bytes());
    png.push(8);
    png.push(6);
    png.extend_from_slice(&[0, 0, 0]);

    let lines = view_image_file("/photos/test.png", &png);
    assert!(lines.iter().any(|l| l.contains("PNG")));
    assert!(lines.iter().any(|l| l.contains("480 x 272")));
    assert!(lines.iter().any(|l| l.contains("RGBA")));
}

#[test]
fn view_image_jpeg() {
    let data = vec![
        0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x01, 0x10, 0x01, 0xE0, 0x03, 0x01, 0x22, 0x00,
    ];
    let lines = view_image_file("/photos/pic.jpg", &data);
    assert!(lines.iter().any(|l| l.contains("JPEG")));
    assert!(lines.iter().any(|l| l.contains("480 x 272")));
}

#[test]
fn view_mode_defaults_to_explorer() {
    let vfs = setup_vfs();
    let fm = FileManagerApp::new("/apps/fm", &vfs);
    assert_eq!(fm.view_mode, ViewMode::Explorer);
}

#[test]
fn select_toggles_view_mode() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.handle_input(&Button::Select, &vfs);
    assert_eq!(fm.view_mode, ViewMode::Dual);
    fm.handle_input(&Button::Select, &vfs);
    assert_eq!(fm.view_mode, ViewMode::Explorer);
}

#[test]
fn select_does_not_toggle_in_file_viewer() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.open_file(&vfs, "/home/user/readme.txt");
    fm.handle_input(&Button::Select, &vfs);
    assert_eq!(fm.view_mode, ViewMode::Explorer);
}

#[test]
fn explorer_right_advances_cursor() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    // Force a 4-column grid so Right increments the cursor by 1.
    fm.explorer_cols = 4;
    fm.explorer_visible_rows = 4;
    let start = fm.panels[0].cursor;
    fm.handle_input(&Button::Right, &vfs);
    assert_eq!(fm.panels[0].cursor, start + 1);
}

#[test]
fn explorer_down_advances_by_columns() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    // Root has 3 entries (home, etc, tmp). With cols=2, Down moves the
    // absolute cursor by 2 -> entry 2 which is in range.
    fm.explorer_cols = 2;
    fm.explorer_visible_rows = 4;
    let start_abs = fm.panels[0].scroll + fm.panels[0].cursor;
    fm.handle_input(&Button::Down, &vfs);
    let new_abs = fm.panels[0].scroll + fm.panels[0].cursor;
    assert_eq!(new_abs, start_abs + 2);
}

#[test]
fn explorer_cancel_at_root_exits() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    let action = fm.handle_input(&Button::Cancel, &vfs);
    assert_eq!(action, AppAction::Exit);
}

#[test]
fn explorer_enter_directory() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.explorer_cols = 4;
    fm.explorer_visible_rows = 4;
    let home_idx = fm.panels[0]
        .lines
        .iter()
        .position(|l| l.starts_with("home"))
        .expect("home/ should be in listing");
    for _ in 0..home_idx {
        fm.handle_input(&Button::Right, &vfs);
    }
    fm.handle_input(&Button::Confirm, &vfs);
    assert_eq!(fm.panels[0].browse_dir, "/home");
}

#[test]
fn view_menu_grid_action_sets_explorer() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.view_mode = ViewMode::Dual;
    let action = fm.run_menu_action("view.grid");
    assert_eq!(action, AppAction::None);
    assert_eq!(fm.view_mode, ViewMode::Explorer);
}

#[test]
fn view_menu_list_action_sets_dual() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    let action = fm.run_menu_action("view.list");
    assert_eq!(action, AppAction::None);
    assert_eq!(fm.view_mode, ViewMode::Dual);
}

#[test]
fn file_close_action_exits() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    let action = fm.run_menu_action("file.close");
    assert_eq!(action, AppAction::Exit);
}

#[test]
fn double_click_on_folder_navigates() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.explorer_cols = 4;
    fm.explorer_visible_rows = 4;
    // Pick whichever directory ends up first after sorting.
    let first = fm.panels[0].lines[0].clone();
    let expected_name = first.trim_end_matches('/');
    let expected_path = format!("/{expected_name}");
    // Default view is Explorer + windowed (fullscreen=false), so the
    // handler uses body_top = 0 (no inner title bar — the WM's titlebar
    // serves that role and click coords are content-rect-relative).
    let g = compute_explorer_geom(0, 0, 600, 400);
    let tile_x = g.grid_x + 4 + g.tile_w as i32 / 2;
    let tile_y = g.body_y + 4 + g.tile_h as i32 / 2;
    // First click selects.
    click_at(&mut fm, tile_x, tile_y, 600, 400, &vfs);
    assert_eq!(fm.last_click_tile, Some(0));
    assert_eq!(fm.panels[0].browse_dir, "/");
    // Second click on same tile activates (the click has the vfs, so
    // it navigates right away).
    click_at(&mut fm, tile_x, tile_y, 600, 400, &vfs);
    assert_eq!(fm.panels[0].browse_dir, expected_path);
    assert!(fm.pending_navigation.is_none());
}

#[test]
fn click_on_tree_row_navigates_immediately() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    // First go into /home so the tree has multiple rows.
    fm.pending_navigation = Some(NavTarget::Folder("/home".to_string()));
    fm.apply_pending_navigation(&vfs);
    // Tree row 1 is "/" (root). Compute its position. Mirror the
    // handler in windowed Explorer mode: body_top = 0.
    let g = compute_explorer_geom(0, 0, 600, 400);
    // cached_font_hint defaults to 11 → (11+2).max(11) = 13.
    let tree_line_h = 13;
    let row_idx = 1; // "/" entry.
    let tx = g.tree_x + 8;
    let ty = g.body_y + 4 + row_idx * tree_line_h + tree_line_h / 2;
    click_at(&mut fm, tx, ty, 600, 400, &vfs);
    assert_eq!(fm.panels[0].browse_dir, "/");
    assert!(fm.pending_navigation.is_none());
}

#[test]
fn click_on_view_label_opens_dropdown() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    // Default view is Explorer + windowed: no inner title bar, so the
    // menu bar starts at y=0 (height 18). View is the third entry.
    // Labels are as wide as the font draws them (11 px bitmap) + padding.
    let label_w = |l: &str| oasis_types::backend::bitmap_measure_text(l, 11) as i32 + 16;
    let file_w = label_w("File");
    let edit_w = label_w("Edit");
    let view_x = 6 + file_w + edit_w + 4;
    let action = click_at(&mut fm, view_x, 8, 600, 400, &vfs);
    assert_eq!(action, AppAction::None);
    assert_eq!(fm.menu.open, Some(2));
}

#[test]
fn list_panel_classifies_and_matches_list_directory() {
    let vfs = setup_vfs();
    let (lines, entries) = list_panel(&vfs, "/home/user");
    assert_eq!(lines, list_directory(&vfs, "/home/user"));
    assert_eq!(lines.len(), entries.len());
    let kinds: Vec<(String, EntryKind)> = entries
        .iter()
        .flatten()
        .map(|e| (e.name.clone(), e.kind))
        .collect();
    assert_eq!(kinds[0], ("..".to_string(), EntryKind::ParentDir));
    assert!(kinds.contains(&("music".to_string(), EntryKind::Dir)));
    assert!(kinds.contains(&("readme.txt".to_string(), EntryKind::File)));
    let (lines, entries) = list_panel(&vfs, "/tmp");
    assert_eq!(lines[1], "(empty directory)");
    assert!(entries[1].is_none());
}

#[test]
fn names_resembling_the_size_suffix_target_the_real_entry() {
    let mut vfs = setup_vfs();
    vfs.mkdir("/x").unwrap();
    vfs.mkdir("/x/a").unwrap();
    vfs.write("/x/a/keep.txt", b"k").unwrap();
    vfs.write("/x/a  (old).txt", b"old").unwrap();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    select(&mut fm, &vfs, 0, "/x", "a  (old).txt");
    assert_eq!(
        fm.panels[0].selected_path().as_deref(),
        Some("/x/a  (old).txt")
    );

    fm.handle_input(&Button::Triangle, &vfs);
    fm.handle_input(&Button::Confirm, &vfs);
    assert!(fm.apply_vfs_ops(&mut vfs));
    assert!(!vfs.exists("/x/a  (old).txt"));
    assert!(vfs.exists("/x/a/keep.txt"), "folder `a` must survive");
}

#[test]
fn refresh_after_shrink_keeps_selection_in_range() {
    let mut vfs = setup_vfs();
    for i in 0..10 {
        vfs.write(&format!("/tmp/f{i}.txt"), b"x").unwrap();
    }
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.panels[0].navigate_to("/tmp", &vfs);
    // `..` + 10 files; select the last one with scroll 5.
    fm.panels[0].scroll = 5;
    fm.panels[0].cursor = 5;
    for i in 5..10 {
        vfs.remove(&format!("/tmp/f{i}.txt")).unwrap();
    }
    fm.panels[0].refresh(&vfs);
    let p = &fm.panels[0];
    assert!(
        p.scroll + p.cursor < p.lines.len(),
        "scroll {} + cursor {} >= len {}",
        p.scroll,
        p.cursor,
        p.lines.len()
    );
}

#[test]
fn rescan_keeps_moved_selection_visible() {
    let mut vfs = setup_vfs();
    vfs.write("/tmp/z.txt", b"x").unwrap();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.view_mode = ViewMode::Dual;
    fm.dual_visible_rows = 4;
    select(&mut fm, &vfs, 0, "/tmp", "z.txt");
    // Many entries appear before the selection.
    for i in 0..20 {
        vfs.write(&format!("/tmp/a{i:02}.txt"), b"x").unwrap();
    }
    assert!(fm.rescan(&vfs));
    let p = &fm.panels[0];
    assert!(
        p.cursor < 4,
        "cursor {} outside the 4 visible rows",
        p.cursor
    );
    assert!(p.lines[p.scroll + p.cursor].starts_with("z.txt"));
}

/// Explorer tile labels are cut to the tile in pixels: a character-count
/// cut let wide names spill into the next tile.
#[test]
fn explorer_tile_labels_stay_inside_their_tile() {
    let mut vfs = setup_vfs();
    vfs.write("/a_rather_long_file_name.txt", b"x").unwrap();
    vfs.write("/WWWWWWWWW", b"x").unwrap();
    let fm = FileManagerApp::new("/apps/fm", &vfs);
    let at = ActiveTheme::default();
    let mut rec = oasis_test_backend::RecordingBackend::new(480, 272);
    fm.draw_windowed_explorer(0, 0, 480, 272, &mut rec, &at)
        .unwrap();
    let g = compute_explorer_geom(0, 0, 480, 272);
    let label_y = g.body_y + 4 + 4 + g.icon_h as i32 + 2;
    let labels: Vec<(String, i32)> = rec
        .text_positions()
        .into_iter()
        .filter(|&(_, _, y, _)| (y - label_y) % g.tile_h as i32 == 0 && y >= label_y)
        .map(|(t, x, _, size)| {
            (
                t.to_string(),
                x + oasis_types::backend::bitmap_measure_text(t, size) as i32,
            )
        })
        .collect();
    assert!(labels.iter().any(|(t, _)| t.ends_with("...")), "{labels:?}");
    assert!(labels.iter().any(|(t, _)| t == "etc"), "{labels:?}");
    for (text, right) in &labels {
        let col = (right - 1 - (g.grid_x + 4)) / g.tile_w as i32;
        let tile_right = g.grid_x + 4 + (col + 1) * g.tile_w as i32;
        assert!(
            *right <= tile_right,
            "{text:?} ends at {right}, tile at {tile_right}"
        );
    }
}

#[test]
fn dual_view_draws_at_zero_width() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.view_mode = ViewMode::Dual;
    let at = ActiveTheme::default();
    let mut backend = oasis_test_backend::NullBackend::new(480, 272);
    fm.draw_windowed_dual(0, 0, 0, 200, &mut backend, &at)
        .expect("draw at zero width");
}

#[test]
fn build_tree_entries_marks_current_and_lists_siblings() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.mkdir("/home/guest").unwrap();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/var").unwrap();

    let entries = build_tree_entries("/home/user", &vfs);
    let current = entries
        .iter()
        .find(|e| e.is_current)
        .expect("must mark current");
    assert_eq!(current.label, "user");
    assert_eq!(current.path, "/home/user");

    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    // Path crumbs.
    assert!(labels.contains(&"Desktop"));
    assert!(labels.contains(&"/"));
    assert!(labels.contains(&"home"));
    // Siblings of `home` at root level.
    assert!(labels.contains(&"etc"));
    assert!(labels.contains(&"var"));
    // Siblings of `user` inside `home`.
    assert!(labels.contains(&"guest"));
}

#[test]
fn build_tree_entries_root_is_current() {
    let vfs = MemoryVfs::new();
    let entries = build_tree_entries("/", &vfs);
    let current = entries
        .iter()
        .find(|e| e.is_current)
        .expect("root must be current");
    assert_eq!(current.label, "/");
    assert_eq!(current.path, "/");
}

#[test]
fn build_tree_entries_lists_children_of_current_dir() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.mkdir("/home/user/documents").unwrap();
    vfs.mkdir("/home/user/photos").unwrap();
    vfs.mkdir("/home/user/scripts").unwrap();

    let entries = build_tree_entries("/home/user", &vfs);
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    // Children of the current directory must be navigable from the tree.
    assert!(labels.contains(&"documents"), "got {labels:?}");
    assert!(labels.contains(&"photos"), "got {labels:?}");
    assert!(labels.contains(&"scripts"), "got {labels:?}");

    // And those children should carry their absolute paths so a click
    // on the row can navigate directly.
    let docs = entries
        .iter()
        .find(|e| e.label == "documents")
        .expect("documents row");
    assert_eq!(docs.path, "/home/user/documents");
}

// -- file operations ------------------------------------------------

/// Point panel `pi` at `dir` and put the cursor on entry `name`.
fn select(fm: &mut FileManagerApp, vfs: &MemoryVfs, pi: usize, dir: &str, name: &str) {
    fm.active_panel = pi;
    fm.panels[pi].navigate_to(dir, vfs);
    let idx = fm.panels[pi]
        .entries
        .iter()
        .position(|e| e.as_ref().is_some_and(|e| e.name == name))
        .expect("entry in listing");
    fm.panels[pi].scroll = 0;
    fm.panels[pi].cursor = idx;
}

fn key(fm: &mut FileManagerApp, vfs: &MemoryVfs, k: Key, mods: Modifiers) -> bool {
    fm.handle_key(&k, mods, vfs).is_some()
}

#[test]
fn delete_requires_confirmation() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    select(&mut fm, &vfs, 0, "/home/user", "readme.txt");

    fm.handle_input(&Button::Triangle, &vfs);
    assert!(matches!(fm.dialog, Some(Dialog::ConfirmDelete { .. })));
    assert!(!fm.apply_vfs_ops(&mut vfs), "nothing queued before confirm");
    assert!(vfs.exists("/home/user/readme.txt"));

    // Cancel keeps the file.
    fm.handle_input(&Button::Cancel, &vfs);
    assert!(fm.dialog.is_none());
    fm.apply_vfs_ops(&mut vfs);
    assert!(vfs.exists("/home/user/readme.txt"));

    // Delete key + "n" also keeps it.
    assert!(key(&mut fm, &vfs, Key::Delete, Modifiers::NONE));
    assert!(key(&mut fm, &vfs, Key::Char('n'), Modifiers::NONE));
    fm.apply_vfs_ops(&mut vfs);
    assert!(vfs.exists("/home/user/readme.txt"));

    // Delete key + "y" removes it and refreshes the listing.
    key(&mut fm, &vfs, Key::Delete, Modifiers::NONE);
    key(&mut fm, &vfs, Key::Char('y'), Modifiers::NONE);
    assert!(fm.apply_vfs_ops(&mut vfs));
    assert!(!vfs.exists("/home/user/readme.txt"));
    assert!(!fm.panels[0].lines.iter().any(|l| l.contains("readme")));
}

#[test]
fn delete_directory_tree_after_confirm() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    select(&mut fm, &vfs, 0, "/", "home");
    fm.handle_input(&Button::Triangle, &vfs);
    fm.handle_input(&Button::Confirm, &vfs);
    fm.apply_vfs_ops(&mut vfs);
    assert!(!vfs.exists("/home"));
    assert!(!vfs.exists("/home/user/readme.txt"));
}

#[test]
fn rename_via_f2_and_typing() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    select(&mut fm, &vfs, 0, "/home/user", "readme.txt");
    assert!(!fm.accepts_text());

    assert!(key(&mut fm, &vfs, Key::F(2), Modifiers::NONE));
    assert!(fm.accepts_text());
    for _ in 0.."readme.txt".len() {
        fm.handle_backspace();
    }
    for ch in "notes.md".chars() {
        // Printable keys fall through to handle_text_input.
        assert!(!key(&mut fm, &vfs, Key::Char(ch), Modifiers::NONE));
        fm.handle_text_input(ch);
    }
    assert!(key(&mut fm, &vfs, Key::Enter, Modifiers::NONE));
    assert!(!fm.accepts_text());
    fm.apply_vfs_ops(&mut vfs);
    assert!(!vfs.exists("/home/user/readme.txt"));
    assert_eq!(vfs.read("/home/user/notes.md").unwrap(), b"Hello!");
}

#[test]
fn rename_onto_existing_name_fails() {
    let mut vfs = setup_vfs();
    vfs.write("/home/user/other.txt", b"x").unwrap();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    select(&mut fm, &vfs, 0, "/home/user", "readme.txt");
    fm.begin_rename();
    if let Some(Dialog::NameEntry { text, .. }) = &mut fm.dialog {
        *text = "other.txt".to_string();
    }
    fm.handle_input(&Button::Confirm, &vfs);
    fm.apply_vfs_ops(&mut vfs);
    assert_eq!(vfs.read("/home/user/readme.txt").unwrap(), b"Hello!");
    assert_eq!(vfs.read("/home/user/other.txt").unwrap(), b"x");
    assert!(fm.status.as_deref().is_some_and(|s| s.contains("exists")));
}

#[test]
fn invalid_name_keeps_prompt_open() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.begin_new_folder();
    if let Some(Dialog::NameEntry { text, .. }) = &mut fm.dialog {
        *text = "a/b".to_string();
    }
    fm.handle_input(&Button::Confirm, &vfs);
    assert!(fm.dialog.is_some());
    assert!(fm.pending_ops.is_empty());
}

#[test]
fn copy_dir_tree_and_paste_into_other_panel() {
    let mut vfs = setup_vfs();
    vfs.write("/home/user/music/song.mp3", b"abc").unwrap();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.view_mode = ViewMode::Dual;
    fm.panels[1].navigate_to("/tmp", &vfs);
    select(&mut fm, &vfs, 0, "/home", "user");

    assert!(key(&mut fm, &vfs, Key::Char('c'), Modifiers::CTRL));
    // Switch to the right panel (/tmp) and paste there.
    fm.handle_input(&Button::Right, &vfs);
    assert_eq!(fm.active_panel, 1);
    assert!(key(&mut fm, &vfs, Key::Char('v'), Modifiers::CTRL));
    assert!(fm.apply_vfs_ops(&mut vfs));

    assert_eq!(vfs.read("/tmp/user/readme.txt").unwrap(), b"Hello!");
    assert_eq!(vfs.read("/tmp/user/music/song.mp3").unwrap(), b"abc");
    assert!(vfs.exists("/tmp/user/photos"));
    // Source untouched, right panel shows the copy.
    assert!(vfs.exists("/home/user/readme.txt"));
    assert!(fm.panels[1].lines.iter().any(|l| l == "user/"));

    // Pasting again picks a unique name.
    key(&mut fm, &vfs, Key::Char('v'), Modifiers::CTRL);
    fm.apply_vfs_ops(&mut vfs);
    assert!(vfs.exists("/tmp/user (2)/music/song.mp3"));
}

#[test]
fn cut_paste_moves_and_clears_clipboard() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.panels[1].navigate_to("/tmp", &vfs);
    select(&mut fm, &vfs, 0, "/etc", "hostname");
    key(&mut fm, &vfs, Key::Char('x'), Modifiers::CTRL);
    fm.active_panel = 1;
    key(&mut fm, &vfs, Key::Char('v'), Modifiers::CTRL);
    fm.apply_vfs_ops(&mut vfs);
    assert!(!vfs.exists("/etc/hostname"));
    assert_eq!(vfs.read("/tmp/hostname").unwrap(), b"oasis");
    assert!(fm.clipboard.is_none());
    // The left panel's listing no longer shows the moved file.
    assert!(!fm.panels[0].lines.iter().any(|l| l.contains("hostname")));
}

#[test]
fn menu_copy_paste_entries() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    select(&mut fm, &vfs, 0, "/etc", "hostname");
    fm.run_menu_action("edit.copy");
    fm.panels[0].navigate_to("/tmp", &vfs);
    fm.run_menu_action("edit.paste");
    fm.apply_vfs_ops(&mut vfs);
    assert!(vfs.exists("/tmp/hostname"));
    assert!(vfs.exists("/etc/hostname"));
}

#[test]
fn new_folder_names_are_unique() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.panels[0].navigate_to("/tmp", &vfs);
    for _ in 0..3 {
        fm.handle_input(&Button::Square, &vfs);
        assert!(fm.accepts_text());
        fm.handle_input(&Button::Confirm, &vfs);
        fm.apply_vfs_ops(&mut vfs);
    }
    assert!(vfs.exists("/tmp/new_folder"));
    assert!(vfs.exists("/tmp/new_folder (2)"));
    assert!(vfs.exists("/tmp/new_folder (3)"));
    assert!(fm.panels[0].lines.iter().any(|l| l == "new_folder (3)/"));
}

#[test]
fn dialog_swallows_navigation() {
    let vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.view_mode = ViewMode::Dual;
    fm.request_delete();
    let before = fm.panels[0].cursor;
    fm.handle_input(&Button::Down, &vfs);
    assert!(key(&mut fm, &vfs, Key::Down, Modifiers::NONE));
    assert_eq!(fm.panels[0].cursor, before);
    assert!(fm.dialog.is_some());
}

#[test]
fn dialog_buttons_clickable() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    select(&mut fm, &vfs, 0, "/etc", "hostname");
    fm.request_delete();
    let g = crate::dialog::dialog_geom(0, 0, 400, 240);
    click_at(&mut fm, g.ok.x + 2, g.ok.y + 2, 400, 240, &vfs);
    assert!(fm.dialog.is_none());
    fm.apply_vfs_ops(&mut vfs);
    assert!(!vfs.exists("/etc/hostname"));
}

#[test]
fn deleting_open_directory_moves_panel_up() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.panels[1].navigate_to("/home/user/music", &vfs);
    select(&mut fm, &vfs, 0, "/home", "user");
    fm.request_delete();
    fm.commit_dialog();
    fm.apply_vfs_ops(&mut vfs);
    assert_eq!(fm.panels[1].browse_dir, "/home");
}

#[test]
fn tick_picks_up_changes_made_by_other_apps() {
    let mut vfs = setup_vfs();
    let mut fm = FileManagerApp::new("/apps/fm", &vfs);
    fm.panels[0].navigate_to("/home/user", &vfs);
    fm.panels[1].navigate_to("/home/user/music", &vfs);
    select(&mut fm, &vfs, 0, "/home/user", "readme.txt");

    // Another app writes a file and removes the right panel's folder.
    vfs.write("/home/user/a_new.txt", b"x").unwrap();
    vfs.write("/home/user/b_new.txt", b"y").unwrap();
    vfs.remove("/home/user/music").unwrap();

    // Nothing happens before the rescan interval elapses.
    assert!(!fm.tick(100, &vfs));
    assert!(
        !fm.panels[0]
            .lines
            .iter()
            .any(|l| l.starts_with("a_new.txt"))
    );
    assert!(
        fm.tick(state::RESCAN_INTERVAL_MS, &vfs),
        "rescan reports a change"
    );
    assert!(
        fm.panels[0]
            .lines
            .iter()
            .any(|l| l.starts_with("a_new.txt"))
    );
    // The selection stays on the same entry although it moved down.
    let p = &fm.panels[0];
    assert!(p.lines[p.scroll + p.cursor].starts_with("readme.txt"));
    assert_eq!(
        fm.panels[1].browse_dir, "/home/user",
        "vanished dir -> parent"
    );
    // Unchanged listings don't report a change.
    assert!(!fm.tick(state::RESCAN_INTERVAL_MS, &vfs));
}

/// A VFS with `/many` holding `n` files.
fn many_files_vfs(n: usize) -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/many").unwrap();
    for i in 0..n {
        vfs.write(&format!("/many/f{i:02}.txt"), b"x").unwrap();
    }
    vfs
}

#[test]
fn switching_to_explorer_after_dual_scroll_keeps_cursor_valid() {
    // The dual view scrolls one line at a time; the Explorer grid assumes
    // whole rows. Switching views used to leave `scroll = 3` with 4
    // columns, and Left then underflowed `cursor` (a debug panic).
    let vfs = many_files_vfs(40);
    let mut fm = FileManagerApp::new("/apps/files", &vfs);
    fm.panels[0].navigate_to("/many", &vfs);
    fm.view_mode = ViewMode::Dual;
    fm.layout(480, 272, &ActiveTheme::default());
    fm.panels[0].scroll = 3;
    fm.panels[0].cursor = 0;
    fm.handle_input(&Button::Select, &vfs);
    assert_eq!(fm.view_mode, ViewMode::Explorer);
    fm.layout(480, 272, &ActiveTheme::default());
    let cols = fm.explorer_cols;
    assert!(cols > 1, "grid has several columns at 480 wide");
    let p = &fm.panels[0];
    assert_eq!(p.scroll % cols, 0, "scroll aligned to a grid row");
    assert_eq!(p.scroll + p.cursor, 3, "same entry still selected");
    fm.handle_input(&Button::Left, &vfs);
    let p = &fm.panels[0];
    assert_eq!(p.scroll + p.cursor, 2);
}

#[test]
fn dual_view_cursor_stays_on_a_drawn_row() {
    // Navigation used the shared line-list row count, which is taller than
    // the dual view's panel area: Down walked the `> ` marker off-screen.
    let vfs = many_files_vfs(40);
    let mut fm = FileManagerApp::new("/apps/files", &vfs);
    fm.panels[0].navigate_to("/many", &vfs);
    fm.view_mode = ViewMode::Dual;
    let mut h = oasis_app_core::testing::AppHarness::with_vfs(Box::new(fm), vfs);
    let _ = h.draw();
    for _ in 0..30 {
        h.press(Button::Down);
        let fm = h.app_as::<FileManagerApp>();
        let p = &fm.panels[0];
        let selected = p.lines[p.scroll + p.cursor].clone();
        let shown = h.screen_text();
        assert!(
            shown.contains(&format!("> {selected}")),
            "selected `{selected}` not drawn:\n{shown}"
        );
    }
}

#[test]
fn explorer_status_counts_real_entries_only() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/empty").unwrap();
    let mut fm = FileManagerApp::new("/apps/files", &vfs);
    fm.panels[0].navigate_to("/empty", &vfs);
    let mut h = oasis_app_core::testing::AppHarness::with_vfs(Box::new(fm), vfs);
    let text = h.screen_text();
    assert!(text.contains("0 object(s)"), "{text}");
}
