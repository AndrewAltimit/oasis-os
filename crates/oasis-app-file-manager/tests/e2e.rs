//! End-to-end sessions: the File Manager driven through the `App` trait
//! the way the desktop host drives it (clicks on drawn menu labels, icon
//! tiles and dialog buttons, keyboard shortcuts, gamepad), with every
//! operation checked against the `MemoryVfs` and the drawn listing.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_file_manager::{Dialog, FileManagerApp, ViewMode};
use oasis_types::input::{Button, Key, Modifiers};
use oasis_vfs::{MemoryVfs, Vfs};

fn tree() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    for d in [
        "/home",
        "/home/user",
        "/home/user/docs",
        "/home/user/empty",
        "/music",
    ] {
        vfs.mkdir(d).unwrap();
    }
    vfs.write("/home/user/notes.txt", b"hello notes").unwrap();
    vfs.write("/home/user/photo.png", b"\x89PNG not really")
        .unwrap();
    vfs.write("/home/user/song.mp3", b"ID3").unwrap();
    vfs.write("/home/user/blob.bin", &[0u8, 1, 2, 255]).unwrap();
    vfs.write("/home/user/docs/a.md", b"# A").unwrap();
    vfs.write("/home/user/docs/b.md", b"# B").unwrap();
    vfs
}

fn fm(vfs: MemoryVfs) -> AppHarness {
    let app = FileManagerApp::new("/apps/File Manager", &vfs);
    AppHarness::with_vfs(Box::new(app), vfs)
}

fn app(h: &AppHarness) -> &FileManagerApp {
    h.app_as::<FileManagerApp>()
}

fn cwd(h: &AppHarness) -> String {
    let a = app(h);
    a.panels[a.active_panel].browse_dir.clone()
}

/// Entry names of the active panel (".." and size suffixes stripped).
fn listing(h: &AppHarness) -> Vec<String> {
    let a = app(h);
    a.panels[a.active_panel]
        .lines
        .iter()
        .map(|l| {
            let l = l.trim();
            l.split("  (").next().unwrap_or(l).to_string()
        })
        .filter(|l| l != "..")
        .collect()
}

/// Double-click an icon tile by its drawn label (the grid is drawn after
/// the folder tree, so the last match is the tile).
fn open_tile(h: &mut AppHarness, label: &str) -> AppAction {
    h.click_text_last(label);
    h.click_text_last(label)
}

fn select_tile(h: &mut AppHarness, label: &str) {
    h.click_text_last(label);
}

fn menu(h: &mut AppHarness, top: &str, item: &str) -> AppAction {
    h.click_text(top);
    h.click_text(item)
}

/// Replace the name field of an open name dialog and press the OK button.
fn enter_name(h: &mut AppHarness, name: &str) {
    assert!(h.app().accepts_text(), "name dialog not open");
    for _ in 0..80 {
        h.key(Key::Backspace);
    }
    h.type_text(name);
    h.click_text("OK");
    h.frame(16);
}

fn go_home(h: &mut AppHarness) {
    open_tile(h, "home");
    open_tile(h, "user");
    assert_eq!(cwd(h), "/home/user");
}

#[test]
fn navigate_into_and_out_of_directories_by_clicking() {
    let mut h = fm(tree());
    assert_eq!(listing(&h), vec!["home/", "music/"]);
    go_home(&mut h);
    let screen = h.screen_text();
    for name in ["docs", "empty", "notes.txt", "photo.png"] {
        assert!(screen.contains(name), "{name} not drawn:\n{screen}");
    }
    open_tile(&mut h, "empty");
    assert_eq!(listing(&h), vec!["(empty directory)"]);
    // ".." tile goes up; Cancel goes up too, and exits only at "/".
    open_tile(&mut h, "..");
    assert_eq!(cwd(&h), "/home/user");
    assert_eq!(h.press(Button::Cancel), AppAction::None);
    assert_eq!(cwd(&h), "/home");
    // The folder tree navigates with a single click.
    h.click_text("/");
    assert_eq!(cwd(&h), "/");
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn create_folder_via_menu_then_rename_it() {
    let mut h = fm(tree());
    go_home(&mut h);
    menu(&mut h, "Edit", "New Folder");
    enter_name(&mut h, "My Stuff \u{e9}\u{4e2d}");
    assert!(h.vfs().exists("/home/user/My Stuff \u{e9}\u{4e2d}"));
    assert!(listing(&h).contains(&"My Stuff \u{e9}\u{4e2d}/".to_string()));
    // A second folder with the same name gets a unique suffix.
    menu(&mut h, "Edit", "New Folder");
    enter_name(&mut h, "My Stuff \u{e9}\u{4e2d}");
    assert!(h.vfs().exists("/home/user/My Stuff \u{e9}\u{4e2d} (2)"));

    // Rename with F2 on the selected tile.
    select_tile(&mut h, "docs");
    h.key(Key::F(2));
    enter_name(&mut h, "papers");
    assert!(!h.vfs().exists("/home/user/docs"));
    assert!(h.vfs().exists("/home/user/papers/a.md"));
    assert!(listing(&h).contains(&"papers/".to_string()));
    assert!(!listing(&h).contains(&"docs/".to_string()));
}

#[test]
fn rename_onto_an_existing_name_is_refused() {
    let mut h = fm(tree());
    go_home(&mut h);
    select_tile(&mut h, "notes.txt");
    menu(&mut h, "Edit", "Rename");
    enter_name(&mut h, "blob.bin");
    assert_eq!(
        h.vfs().read("/home/user/notes.txt").unwrap(),
        b"hello notes"
    );
    assert_eq!(
        h.vfs().read("/home/user/blob.bin").unwrap(),
        &[0u8, 1, 2, 255]
    );
    let status = app(&h).status.clone().unwrap_or_default();
    assert!(status.contains("already exists"), "{status}");
}

#[test]
fn invalid_names_keep_the_dialog_open() {
    let mut h = fm(tree());
    go_home(&mut h);
    menu(&mut h, "Edit", "New Folder");
    for bad in ["", "   ", "..", "a/b"] {
        for _ in 0..80 {
            h.key(Key::Backspace);
        }
        h.type_text(bad);
        h.key(Key::Enter);
        assert!(
            matches!(app(&h).dialog, Some(Dialog::NameEntry { .. })),
            "{bad:?} closed the dialog"
        );
    }
    h.key(Key::Escape);
    assert!(app(&h).dialog.is_none());
    h.frame(16);
    assert_eq!(h.vfs().readdir("/home/user").unwrap().len(), 6);
}

#[test]
fn delete_non_empty_directory_after_confirmation() {
    let mut h = fm(tree());
    go_home(&mut h);
    select_tile(&mut h, "docs");
    h.key(Key::Delete);
    // "No" keeps everything.
    h.click_text("No");
    h.frame(16);
    assert!(h.vfs().exists("/home/user/docs/a.md"));
    h.key(Key::Delete);
    h.click_text("Yes");
    h.frame(16);
    assert!(!h.vfs().exists("/home/user/docs"));
    assert!(!h.vfs().exists("/home/user/docs/a.md"));
    assert!(!listing(&h).contains(&"docs/".to_string()));
    assert!(
        h.draw().find_text("docs").is_none(),
        "deleted tile still drawn"
    );
}

#[test]
fn deleting_the_directory_you_are_in_from_the_other_panel_falls_back() {
    let mut h = fm(tree());
    go_home(&mut h);
    open_tile(&mut h, "docs");
    // Switch to the dual view; the right panel deletes /home/user.
    h.press(Button::Select);
    assert_eq!(app(&h).view_mode, ViewMode::Dual);
    h.press(Button::Right);
    // Right panel at "/": select "home" (index 0) and delete it.
    h.press(Button::Triangle);
    h.key(Key::Char('y'));
    h.frame(16);
    assert!(!h.vfs().exists("/home"));
    let a = app(&h);
    assert_eq!(a.panels[0].browse_dir, "/", "left panel kept a dead dir");
    let _ = h.draw();
}

#[test]
fn copy_paste_file_and_folder_then_cut_paste_moves() {
    let mut h = fm(tree());
    go_home(&mut h);
    select_tile(&mut h, "notes.txt");
    h.ctrl('c');
    open_tile(&mut h, "empty");
    h.ctrl('v');
    h.frame(16);
    assert_eq!(
        h.vfs().read("/home/user/empty/notes.txt").unwrap(),
        b"hello notes"
    );
    assert_eq!(listing(&h), vec!["notes.txt"]);
    // Pasting again makes a unique copy; the source stays.
    menu(&mut h, "Edit", "Paste");
    h.frame(16);
    assert!(h.vfs().exists("/home/user/empty/notes (2).txt"));
    assert!(h.vfs().exists("/home/user/notes.txt"));

    // Recursive folder copy.
    open_tile(&mut h, "..");
    select_tile(&mut h, "docs");
    menu(&mut h, "Edit", "Copy");
    open_tile(&mut h, "empty");
    menu(&mut h, "Edit", "Paste");
    h.frame(16);
    assert_eq!(h.vfs().read("/home/user/empty/docs/b.md").unwrap(), b"# B");

    // Cut + paste moves.
    open_tile(&mut h, "..");
    select_tile(&mut h, "blob.bin");
    h.ctrl('x');
    open_tile(&mut h, "empty");
    h.ctrl('v');
    h.frame(16);
    assert!(!h.vfs().exists("/home/user/blob.bin"));
    assert!(h.vfs().exists("/home/user/empty/blob.bin"));
    // The clipboard is consumed by a move.
    h.ctrl('v');
    h.frame(16);
    assert!(app(&h).status.clone().unwrap_or_default().contains("empty"));
}

#[test]
fn copying_a_folder_into_itself_is_refused() {
    let mut h = fm(tree());
    go_home(&mut h);
    select_tile(&mut h, "docs");
    h.ctrl('c');
    open_tile(&mut h, "docs");
    h.ctrl('v');
    h.frame(16);
    assert!(!h.vfs().exists("/home/user/docs/docs"));
    let status = app(&h).status.clone().unwrap_or_default();
    assert!(status.contains("into itself"), "{status}");
}

#[test]
fn opening_typed_files_hands_off_to_their_apps() {
    let mut h = fm(tree());
    go_home(&mut h);
    let cases = [
        ("notes.txt", "Text Editor"),
        ("photo.png", "Photo Viewer"),
        ("song.mp3", "Music Player"),
    ];
    for (name, want) in cases {
        let launch = AppAction::LaunchAppWithFile {
            app_title: want.to_string(),
            file_path: format!("/home/user/{name}"),
        };
        // Mouse: double-click the tile.
        assert_eq!(open_tile(&mut h, name), launch, "double-click on {name}");
        assert_eq!(h.app().viewing_file(), None);
        // Keyboard / gamepad: the tile stays selected, Confirm opens it.
        assert_eq!(h.press(Button::Confirm), launch, "Confirm on {name}");
    }
    // Untyped files open in the built-in viewer; Escape returns.
    open_tile(&mut h, "blob.bin");
    assert_eq!(h.app().viewing_file(), Some("/home/user/blob.bin"));
    let _ = h.draw();
    h.key(Key::Escape);
    assert_eq!(h.app().viewing_file(), None);
    assert_eq!(cwd(&h), "/home/user");
}

#[test]
fn dual_view_gamepad_session() {
    let mut h = fm(tree());
    h.click_text("View");
    h.click_text("List");
    assert_eq!(app(&h).view_mode, ViewMode::Dual);
    // Enter home/user with the d-pad.
    h.press(Button::Confirm); // home/
    h.press(Button::Down); // skip ".."
    h.press(Button::Confirm); // user/
    assert_eq!(cwd(&h), "/home/user");
    // Square = new folder with the default name.
    h.press(Button::Square);
    h.press(Button::Confirm);
    h.frame(16);
    assert!(h.vfs().exists("/home/user/new_folder"));
    let screen = h.screen_text();
    assert!(screen.contains("new_folder"), "{screen}");
    h.draw_all_sizes_and_themes();
}

#[test]
fn names_with_spaces_and_unicode_round_trip_through_listing() {
    let mut vfs = tree();
    vfs.write("/home/user/my file (1).txt", b"x").unwrap();
    vfs.write("/home/user/\u{65e5}\u{672c}\u{8a9e}.txt", b"y")
        .unwrap();
    let mut h = fm(vfs);
    go_home(&mut h);
    let names = listing(&h);
    assert!(names.contains(&"my file (1).txt".to_string()), "{names:?}");
    assert!(names.contains(&"\u{65e5}\u{672c}\u{8a9e}.txt".to_string()));
    select_tile(&mut h, "\u{65e5}\u{672c}\u{8a9e}.txt");
    h.key(Key::Delete);
    h.key(Key::Char('y'));
    h.frame(16);
    assert!(!h.vfs().exists("/home/user/\u{65e5}\u{672c}\u{8a9e}.txt"));
    assert!(h.vfs().exists("/home/user/my file (1).txt"));
}

#[test]
fn modal_dialog_swallows_navigation_keys() {
    let mut h = fm(tree());
    go_home(&mut h);
    h.key_mods(Key::Char('n'), Modifiers::CTRL | Modifiers::SHIFT);
    assert!(h.app().accepts_text());
    h.key(Key::Up);
    h.key(Key::Left);
    h.press(Button::Triangle);
    assert_eq!(cwd(&h), "/home/user");
    assert!(matches!(app(&h).dialog, Some(Dialog::NameEntry { .. })));
    // Typing "q"/"e" goes to the name, not to desktop switching.
    h.type_text("qe");
    if let Some(Dialog::NameEntry { text, .. }) = &app(&h).dialog {
        assert!(text.ends_with("qe"), "{text}");
    }
}

#[test]
fn draws_inside_window_at_all_sizes_and_themes() {
    let mut vfs = tree();
    for i in 0..60 {
        vfs.write(
            &format!("/home/user/file_{i:02}_with_a_long_name.txt"),
            b"z",
        )
        .unwrap();
    }
    let mut h = fm(vfs);
    go_home(&mut h);
    h.draw_all_sizes_and_themes();
    menu(&mut h, "Edit", "New Folder");
    h.draw_all_sizes_and_themes();
    h.key(Key::Escape);
    h.press(Button::Select);
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |vfs: &dyn Vfs| -> Box<dyn App> {
        Box::new(FileManagerApp::new("/apps/File Manager", vfs))
    };
    for seed in 1..=3 {
        fuzz_app(&make, tree(), seed, 3000);
    }
}
