//! End-to-end sessions: the text editor driven through the `App` trait the
//! way the desktop host drives it (keyboard routing, menu clicks on drawn
//! labels, per-frame VFS flushes), then reopened from the VFS.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_text_editor::{EditorMode, TextEditorApp};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::{MemoryVfs, Vfs};

fn vfs_with_home() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs
}

fn editor_on(vfs: MemoryVfs, path: &str) -> AppHarness {
    let app = TextEditorApp::open_from_vfs(path, &vfs);
    AppHarness::with_vfs(Box::new(app), vfs)
}

fn ed(h: &AppHarness) -> &TextEditorApp {
    h.app_as::<TextEditorApp>()
}

fn text(h: &AppHarness) -> String {
    ed(h).save_content()
}

fn read(h: &AppHarness, path: &str) -> String {
    String::from_utf8(h.vfs().read(path).unwrap()).unwrap()
}

/// Close the harness' editor and reopen `path` from the same VFS.
fn reopen(h: &mut AppHarness, path: &str) {
    let app = TextEditorApp::open_from_vfs(path, h.vfs());
    h.replace_app(Box::new(app));
}

#[test]
fn type_save_with_ctrl_s_and_reopen_shows_content() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/notes.txt", b"").unwrap();
    let mut h = editor_on(vfs, "/home/user/notes.txt");
    h.type_text("Hello World");
    h.key(Key::Enter);
    h.type_text("second line 42");
    assert!(ed(&h).is_modified());
    assert!(h.screen_text().contains("Hello World"));

    h.ctrl('s');
    // Nothing is written until the host's apply_vfs_ops pass.
    assert_eq!(read(&h, "/home/user/notes.txt"), "");
    h.frame(16);
    assert_eq!(
        read(&h, "/home/user/notes.txt"),
        "Hello World\nsecond line 42"
    );
    assert!(!ed(&h).is_modified());
    assert!(h.app().lines().join("\n").contains("Saved"));

    reopen(&mut h, "/home/user/notes.txt");
    assert_eq!(text(&h), "Hello World\nsecond line 42");
    assert!(!ed(&h).is_modified());
    let screen = h.screen_text();
    assert!(screen.contains("Hello World"), "{screen}");
    assert!(screen.contains("second line 42"), "{screen}");
}

#[test]
fn save_through_the_file_menu_by_clicking_drawn_labels() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/menu.txt", b"old").unwrap();
    let mut h = editor_on(vfs, "/home/user/menu.txt");
    h.ctrl('a');
    h.type_text("via menu");
    h.click_text("File");
    h.click_text("Save");
    h.frame(16);
    assert_eq!(read(&h, "/home/user/menu.txt"), "via menu");
    // Edit > Undo via the menu walks back to the replaced text.
    for _ in 0..8 {
        if text(&h) == "old" {
            break;
        }
        h.click_text("Edit");
        h.click_text("Undo");
    }
    assert_eq!(text(&h), "old");
    h.click_text("Edit");
    h.click_text("Redo");
    assert_ne!(text(&h), "old");
}

#[test]
fn untitled_document_save_as_prompt_writes_new_file() {
    let mut h = AppHarness::with_vfs(
        Box::new(TextEditorApp::new("/apps/Text Editor")),
        vfs_with_home(),
    );
    h.type_text("draft");
    h.ctrl('s');
    assert_eq!(ed(&h).mode(), EditorMode::SaveAs);
    // The prompt is pre-filled with a suggested path; clear it first.
    for _ in 0.."/home/user/untitled.txt".chars().count() {
        h.key(Key::Backspace);
    }
    h.type_text("/home/user/draft file.md");
    h.key(Key::Enter);
    h.frame(16);
    assert_eq!(read(&h, "/home/user/draft file.md"), "draft");
    assert_eq!(h.app().viewing_file(), Some("/home/user/draft file.md"));
    assert!(h.app().title().contains("draft file.md"));
}

#[test]
fn save_to_missing_parent_reports_failure_and_keeps_changes() {
    let mut h = AppHarness::with_vfs(
        Box::new(TextEditorApp::new("/apps/Text Editor")),
        MemoryVfs::new(),
    );
    h.type_text("data");
    h.key_mods(Key::Char('s'), Modifiers::CTRL | Modifiers::SHIFT);
    for _ in 0.."/home/user/untitled.txt".chars().count() {
        h.key(Key::Backspace);
    }
    h.type_text("/no/such/dir/x.txt");
    h.key(Key::Enter);
    h.frame(16);
    assert!(!h.vfs().exists("/no/such/dir/x.txt"));
    let lines = h.app().lines().join("\n");
    assert!(lines.contains("Save failed"), "{lines}");
    assert!(ed(&h).is_modified(), "failed save must keep the dirty flag");
    // Closing now must still ask about the unsaved changes.
    if ed(&h).mode() == EditorMode::Insert {
        h.press(Button::Cancel);
    }
    h.press(Button::Cancel); // request close
    assert!(!h.closed());
    assert_eq!(ed(&h).mode(), EditorMode::ConfirmDiscard);
}

#[test]
fn saving_keeps_the_files_trailing_newline() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/nl.txt", b"alpha\nbeta\n").unwrap();
    let mut h = editor_on(vfs, "/home/user/nl.txt");
    h.key_mods(Key::Home, Modifiers::CTRL);
    h.type_text("> ");
    h.ctrl('s');
    h.frame(16);
    assert_eq!(read(&h, "/home/user/nl.txt"), "> alpha\nbeta\n");
}

#[test]
fn undo_redo_round_trip() {
    let mut h = AppHarness::new(Box::new(TextEditorApp::new("/apps/e")));
    h.type_text("one two");
    h.key(Key::Enter);
    h.type_text("three");
    let full = text(&h);
    let mut steps = 0;
    while !text(&h).is_empty() {
        h.ctrl('z');
        steps += 1;
        assert!(steps < 50, "undo never reached the empty document");
    }
    assert!(
        steps >= 2,
        "typing across lines should be several undo groups"
    );
    for _ in 0..steps {
        h.ctrl('y');
    }
    assert_eq!(text(&h), full);
    // A redo after new typing is gone.
    h.ctrl('z');
    h.type_text("X");
    let before = text(&h);
    h.ctrl('y');
    assert_eq!(text(&h), before);
    // Ctrl+Shift+Z is also redo.
    h.ctrl('z');
    h.key_mods(Key::Char('z'), Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(text(&h), before);
}

#[test]
fn unicode_editing_moves_and_deletes_whole_characters() {
    let mut h = AppHarness::new(Box::new(TextEditorApp::new("/apps/e")));
    h.type_text("a\u{e9}\u{4e2d}\u{1f600}b");
    assert_eq!(text(&h), "a\u{e9}\u{4e2d}\u{1f600}b");
    // Left over 'b' and the emoji (4 bytes) lands on a char boundary.
    h.key(Key::Left);
    h.key(Key::Left);
    let (_, col) = ed(&h).cursor_position();
    assert_eq!(col, "a\u{e9}\u{4e2d}".len());
    // Backspace removes the CJK char entirely.
    h.key(Key::Backspace);
    assert_eq!(text(&h), "a\u{e9}\u{1f600}b");
    // Delete removes the emoji.
    h.key(Key::Delete);
    assert_eq!(text(&h), "a\u{e9}b");
    h.key(Key::Home);
    h.key(Key::Right);
    h.key(Key::Right);
    assert_eq!(ed(&h).cursor_position().1, "a\u{e9}".len());
    // Shift-select the accented char and cut / paste it at the end.
    h.key_mods(Key::Left, Modifiers::SHIFT);
    h.ctrl('x');
    assert_eq!(text(&h), "ab");
    h.key(Key::End);
    h.ctrl('v');
    assert_eq!(text(&h), "ab\u{e9}");
    let _ = h.draw();
}

#[test]
fn unicode_file_survives_save_and_reopen() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/u.txt", "\u{65e5}\u{672c}\n".as_bytes())
        .unwrap();
    let mut h = editor_on(vfs, "/home/user/u.txt");
    h.ctrl('a');
    h.type_text("caf\u{e9} \u{1f389} \u{4e2d}\u{6587}");
    h.ctrl('s');
    h.frame(16);
    reopen(&mut h, "/home/user/u.txt");
    assert_eq!(text(&h), "caf\u{e9} \u{1f389} \u{4e2d}\u{6587}");
    let _ = h.draw();
}

#[test]
fn selection_copy_paste_and_select_all() {
    let mut h = AppHarness::new(Box::new(TextEditorApp::new("/apps/e")));
    h.type_text("abc");
    h.ctrl('a');
    h.ctrl('c');
    h.key(Key::End);
    h.ctrl('v');
    h.ctrl('v');
    assert_eq!(text(&h), "abcabcabc");
    // Word-wise selection with Ctrl+Shift+Left.
    h.type_text(" tail");
    h.key_mods(Key::Left, Modifiers::CTRL | Modifiers::SHIFT);
    h.key(Key::Backspace);
    assert_eq!(text(&h), "abcabcabc ");
}

#[test]
fn find_and_replace_all_from_the_keyboard() {
    let mut h = AppHarness::new(Box::new(TextEditorApp::new("/apps/e")));
    h.type_text("cat dog cat bird cat");
    h.ctrl('h');
    h.type_text("cat");
    h.key(Key::Tab);
    h.type_text("owl");
    h.key_mods(Key::Enter, Modifiers::CTRL);
    assert_eq!(text(&h), "owl dog owl bird owl");
    assert!(h.app().lines().join("\n").contains("Replaced 3"));
    h.key(Key::Escape);
    assert!(matches!(
        ed(&h).mode(),
        EditorMode::Normal | EditorMode::Insert
    ));
}

#[test]
fn escape_with_unsaved_changes_prompts_then_save_and_close() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/q.txt", b"").unwrap();
    let mut h = editor_on(vfs, "/home/user/q.txt");
    h.type_text("keep me");
    h.key(Key::Escape); // Insert -> Normal
    h.key(Key::Escape); // close -> prompt
    assert!(!h.closed());
    assert_eq!(ed(&h).mode(), EditorMode::ConfirmDiscard);
    h.key(Key::Char('s'));
    // The close only happens once the write lands.
    assert!(!h.closed());
    h.frame(16);
    assert!(h.closed());
    assert_eq!(read(&h, "/home/user/q.txt"), "keep me");
}

#[test]
fn discard_closes_without_writing() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/d.txt", b"orig").unwrap();
    let mut h = editor_on(vfs, "/home/user/d.txt");
    h.type_text("junk");
    h.press(Button::Cancel);
    h.press(Button::Cancel);
    assert_eq!(h.press(Button::Square), AppAction::Exit);
    h.frame(16);
    assert_eq!(read(&h, "/home/user/d.txt"), "orig");
}

#[test]
fn huge_line_and_many_lines_edit_draw_and_save() {
    let mut vfs = vfs_with_home();
    let long = "x".repeat(20_000);
    let many: String = (0..5000).map(|i| format!("line {i}\n")).collect();
    vfs.write("/home/user/long.txt", long.as_bytes()).unwrap();
    vfs.write("/home/user/many.txt", many.as_bytes()).unwrap();

    let mut h = editor_on(vfs, "/home/user/long.txt");
    h.key(Key::End);
    h.type_text("END");
    h.draw_all_sizes_and_themes();
    h.ctrl('s');
    h.frame(16);
    assert_eq!(read(&h, "/home/user/long.txt").len(), 20_003);

    reopen(&mut h, "/home/user/many.txt");
    h.key_mods(Key::End, Modifiers::CTRL);
    // 5000 lines plus the empty line after the final newline.
    assert_eq!(ed(&h).cursor_position().0, 5000);
    for _ in 0..3 {
        h.key(Key::PageUp);
    }
    h.ctrl('g');
    h.type_text("2500");
    h.key(Key::Enter);
    assert_eq!(ed(&h).cursor_position().0, 2499);
    let screen = h.screen_text();
    assert!(screen.contains("line 2499"), "{screen}");
    h.draw_all_sizes_and_themes();
}

#[test]
fn clicking_in_the_text_places_the_caret() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/c.txt", b"first\nsecond\nthird")
        .unwrap();
    let mut h = editor_on(vfs, "/home/user/c.txt");
    h.click_text_containing("third");
    assert_eq!(ed(&h).cursor_position().0, 2);
    h.type_text("!");
    assert!(text(&h).lines().nth(2).unwrap().contains('!'));
}

#[test]
fn opening_a_missing_file_reports_it() {
    let vfs = MemoryVfs::new();
    let h = editor_on(vfs, "/nope.txt");
    let lines = h.app().lines().join("\n");
    assert!(lines.contains("Could not read /nope.txt"), "{lines}");
    let _ = h.draw();
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut vfs = vfs_with_home();
    vfs.write("/home/user/s.rs", b"fn main() {\n    let x = 1; // hi\n}\n")
        .unwrap();
    let mut h = editor_on(vfs, "/home/user/s.rs");
    h.draw_all_sizes_and_themes();
    h.click_text("Edit");
    h.draw_all_sizes_and_themes();
    h.key(Key::Escape);
    h.ctrl('h');
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |vfs: &dyn Vfs| -> Box<dyn App> {
        Box::new(TextEditorApp::open_from_vfs("/home/user/f.txt", vfs))
    };
    for seed in 1..=3 {
        let mut vfs = vfs_with_home();
        vfs.write(
            "/home/user/f.txt",
            "seed \u{e9}\u{4e2d}\nline two\n".as_bytes(),
        )
        .unwrap();
        fuzz_app(&make, vfs, seed, 3000);
    }
}
