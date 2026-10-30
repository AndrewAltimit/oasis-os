//! Music Player / Photo Viewer unit tests.

use super::*;
use oasis_app_core::testing::click_at;
use oasis_vfs::MemoryVfs;

#[test]
fn inherit_textures_from_drains_old_app() {
    // Two BrowsingApps, old one has a cached texture + one in its
    // stale queue. After inherit the new app owns both.
    let vfs = setup_vfs();
    let old = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    let new = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    old.cached_photo_texture
        .set(Some(oasis_types::backend::TextureId(11)));
    old.stale_photo_textures
        .borrow_mut()
        .push(oasis_types::backend::TextureId(22));
    new.inherit_textures_from(&old);
    assert!(old.cached_photo_texture.get().is_none());
    assert!(old.stale_photo_textures.borrow().is_empty());
    let new_stale = new.stale_photo_textures.borrow();
    assert_eq!(new_stale.len(), 2);
    let ids: Vec<u64> = new_stale.iter().map(|t| t.0).collect();
    assert!(ids.contains(&11));
    assert!(ids.contains(&22));
}

fn setup_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/home").unwrap();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user").unwrap();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user/music").unwrap();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user/photos").unwrap();
    oasis_vfs::Vfs::write(&mut vfs, "/home/user/music/ambient_dawn.mp3", b"fake mp3").unwrap();
    oasis_vfs::Vfs::write(
        &mut vfs,
        "/home/user/music/nightfall_theme.mp3",
        b"fake mp3 2",
    )
    .unwrap();
    oasis_vfs::Vfs::write(&mut vfs, "/home/user/photos/sunset.png", b"fake png").unwrap();
    vfs
}

#[test]
fn music_player_title_and_path() {
    let vfs = setup_vfs();
    let app = BrowsingApp::music_player("/apps/music", &vfs);
    assert_eq!(app.title(), "Music Player");
    assert_eq!(app.path(), "/apps/music");
}

#[test]
fn music_player_lists_tracks() {
    let vfs = setup_vfs();
    let app = BrowsingApp::music_player("/apps/music", &vfs);
    assert!(app.browse_dir().is_some());
    assert!(app.lines().iter().any(|l| l.contains("ambient_dawn")));
    assert!(app.lines().iter().any(|l| l.contains("nightfall_theme")));
}

#[test]
fn photo_viewer_lists_photos() {
    let vfs = setup_vfs();
    let app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    assert!(app.browse_dir().is_some());
    assert!(app.lines().iter().any(|l| l.contains("sunset.png")));
}

#[test]
fn music_player_missing_dir() {
    let mut vfs = MemoryVfs::new();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/home").unwrap();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/home/user").unwrap();
    let app = BrowsingApp::music_player("/apps/music", &vfs);
    assert!(
        app.lines()
            .iter()
            .any(|l| l.contains("Music directory not found"))
    );
}

#[test]
fn cancel_exits_from_listing() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
}

#[test]
fn cancel_returns_from_viewer() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    // Open a file.
    app.open_file(&vfs, "/home/user/music/ambient_dawn.mp3");
    assert!(app.viewing_file().is_some());
    // Cancel returns to listing.
    let action = app.handle_input(&Button::Cancel, &vfs);
    assert_eq!(action, AppAction::None);
    assert!(app.viewing_file().is_none());
}

#[test]
fn navigate_up_down() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    app.content.cached_max_visible = 20;
    app.handle_input(&Button::Down, &vfs);
    assert_eq!(app.content.cursor, 1);
    app.handle_input(&Button::Up, &vfs);
    assert_eq!(app.content.cursor, 0);
}

#[test]
fn open_nonexistent_file_noop() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    app.open_file(&vfs, "/does/not/exist.mp3");
    assert!(app.viewing_file().is_none());
}

#[test]
fn photo_viewer_open_file() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.content.cached_max_visible = 20;
    // Find sunset.png and open it.
    let idx = app
        .content
        .lines
        .iter()
        .position(|l| l.contains("sunset.png"))
        .unwrap();
    app.content.cursor = idx;
    app.handle_input(&Button::Confirm, &vfs);
    assert!(app.viewing_file().is_some());
}

#[test]
fn downcast_works() {
    let vfs = setup_vfs();
    let app: Box<dyn App> = Box::new(BrowsingApp::music_player("/apps/music", &vfs));
    assert!(app.as_any().downcast_ref::<BrowsingApp>().is_some());
}

#[test]
fn music_add_to_playlist() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    app.content.cached_max_visible = 20;
    // Navigate to a track and add it.
    let idx = app
        .content
        .lines
        .iter()
        .position(|l| l.contains("ambient_dawn"))
        .unwrap();
    app.content.cursor = idx;
    app.handle_input(&Button::Triangle, &vfs);
    assert_eq!(app.playlist().len(), 1);
    assert!(app.playlist()[0].contains("ambient_dawn"));
}

#[test]
fn music_playlist_no_duplicates() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    app.content.cached_max_visible = 20;
    let idx = app
        .content
        .lines
        .iter()
        .position(|l| l.contains("ambient_dawn"))
        .unwrap();
    app.content.cursor = idx;
    app.handle_input(&Button::Triangle, &vfs);
    app.handle_input(&Button::Triangle, &vfs);
    assert_eq!(app.playlist().len(), 1);
}

#[test]
fn windowed_click_selects_then_double_click_opens() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    let ch = 220;
    app.layout(300, ch, &oasis_skin::ActiveTheme::default());
    let idx = app
        .content
        .lines
        .iter()
        .position(|l| l.starts_with("ambient_dawn.mp3"))
        .expect("track listed");
    let (_, y, _) = oasis_app_core::render::windowed_line_origin(
        &app.content,
        0,
        0,
        ch,
        &oasis_skin::ActiveTheme::default(),
        idx,
    )
    .expect("visible");
    // First click only selects.
    click_at(&mut app, 10, y + 2, 300, ch, &vfs);
    assert_eq!(app.content.scroll + app.content.cursor, idx);
    assert!(app.content.viewing_file.is_none());
    // Second click on the same line opens it (and asks to play).
    click_at(&mut app, 10, y + 2, 300, ch, &vfs);
    assert_eq!(
        app.content.viewing_file.as_deref(),
        Some("/home/user/music/ambient_dawn.mp3")
    );
    assert!(!app.content.requests.is_empty());
    // Clicks are ignored while a track is open, and in fullscreen.
    click_at(&mut app, 10, y + 2, 300, ch, &vfs);
    assert_eq!(
        app.content.viewing_file.as_deref(),
        Some("/home/user/music/ambient_dawn.mp3")
    );
}

#[test]
fn music_open_emits_play_ipc() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    app.open_file(&vfs, "/home/user/music/ambient_dawn.mp3");
    let req = app.content.requests.first().cloned().expect("ipc");
    assert_eq!(req.path, MEDIA_REQUEST_PATH);
    assert_eq!(req.data, "play_file /home/user/music/ambient_dawn.mp3");
}

#[test]
fn music_cancel_emits_stop_ipc() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    app.open_file(&vfs, "/home/user/music/ambient_dawn.mp3");
    // Drain the play IPC.
    app.content.requests.clear();
    app.handle_input(&Button::Cancel, &vfs);
    let req = app.content.requests.first().cloned().expect("ipc");
    assert_eq!(req.path, MEDIA_REQUEST_PATH);
    assert_eq!(req.data, "stop");
}

#[test]
fn photo_open_does_not_emit_media_ipc() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.open_file(&vfs, "/home/user/photos/sunset.png");
    assert!(
        app.content.requests.is_empty(),
        "photo viewer must not emit media IPC"
    );
}

#[test]
fn music_shuffle_toggle() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    assert!(!app.shuffle);
    app.handle_input(&Button::Select, &vfs);
    assert!(app.shuffle);
    app.handle_input(&Button::Select, &vfs);
    assert!(!app.shuffle);
}

#[test]
fn photo_rotate() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.open_file(&vfs, "/home/user/photos/sunset.png");
    assert_eq!(app.rotation(), 0);
    // The sample file is not a real PNG: give the viewer a 2x1 image.
    app.decoded_image = Some(image::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    });
    app.cached_photo_texture.set(Some(TextureId(7)));
    app.handle_input(&Button::Square, &vfs);
    assert_eq!(app.rotation(), 90);
    // The pixels turn (dims swap) and the old texture is retired, so
    // the rotation is actually drawn.
    let now = app.decoded_image().expect("image");
    assert_eq!((now.width, now.height), (1, 2));
    assert!(app.cached_photo_texture.get().is_none());
    assert_eq!(
        app.stale_photo_textures.borrow().as_slice(),
        &[TextureId(7)]
    );
    app.handle_input(&Button::Square, &vfs);
    assert_eq!(app.rotation(), 180);
}

#[test]
fn photo_zoom_cycle() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.open_file(&vfs, "/home/user/photos/sunset.png");
    assert_eq!(app.zoom_level(), 1);
    app.handle_input(&Button::Triangle, &vfs);
    assert_eq!(app.zoom_level(), 2);
    app.handle_input(&Button::Triangle, &vfs);
    assert_eq!(app.zoom_level(), 4);
    app.handle_input(&Button::Triangle, &vfs);
    assert_eq!(app.zoom_level(), 1);
}

#[test]
fn photo_slideshow_toggle() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.open_file(&vfs, "/home/user/photos/sunset.png");
    assert!(!app.slideshow_active());
    app.handle_input(&Button::Start, &vfs);
    assert!(app.slideshow_active());
    app.handle_input(&Button::Start, &vfs);
    assert!(!app.slideshow_active());
}

#[test]
fn slideshow_advances_on_wall_time() {
    let mut vfs = setup_vfs();
    oasis_vfs::Vfs::write(&mut vfs, "/home/user/photos/beach.png", b"fake png 2").unwrap();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.open_file(&vfs, "/home/user/photos/beach.png");

    // Slideshow off: time passes, nothing happens.
    assert!(!app.tick(SLIDESHOW_INTERVAL_MS * 2, &vfs));
    assert_eq!(app.viewing_file(), Some("/home/user/photos/beach.png"));

    app.handle_input(&Button::Start, &vfs);
    // Just under one interval at 60 fps: still on the first slide.
    let mut t = 0;
    while t + 16 < SLIDESHOW_INTERVAL_MS {
        assert!(!app.tick(16, &vfs), "no redraw between slides");
        t += 16;
    }
    assert_eq!(app.viewing_file(), Some("/home/user/photos/beach.png"));
    assert!(app.tick(16, &vfs), "advancing a slide requests a redraw");
    assert_eq!(app.viewing_file(), Some("/home/user/photos/sunset.png"));
    // Same cadence at 10 fps, and it wraps around.
    for _ in 0..(SLIDESHOW_INTERVAL_MS / 100) {
        app.tick(100, &vfs);
    }
    assert_eq!(app.viewing_file(), Some("/home/user/photos/beach.png"));
}

fn music_with_playlist(n: usize) -> (MemoryVfs, BrowsingApp) {
    let mut vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    for i in 0..n {
        let path = format!("/home/user/music/track{i}.mp3");
        oasis_vfs::Vfs::write(&mut vfs, &path, b"fake mp3").unwrap();
        app.playlist.push(path);
    }
    let first = app.playlist[0].clone();
    app.open_file(&vfs, &first);
    (vfs, app)
}

#[test]
fn shuffle_picks_random_other_tracks() {
    let (vfs, mut app) = music_with_playlist(6);
    app.handle_input(&Button::Select, &vfs); // Shuffle on.
    let mut order = vec![app.playlist_index];
    for _ in 0..60 {
        app.handle_input(&Button::Right, &vfs);
        assert_ne!(Some(&app.playlist_index), order.last(), "no repeats");
        assert!(app.playlist_index < 6);
        order.push(app.playlist_index);
    }
    let sequential: Vec<usize> = (0..order.len()).map(|i| i % 6).collect();
    assert_ne!(order, sequential, "shuffle must not play in order");
    for track in 0..6 {
        assert!(order.contains(&track), "track {track} never played");
    }
    // Previous retraces the shuffled order.
    let last = order.len() - 1;
    app.handle_input(&Button::Left, &vfs);
    assert_eq!(app.playlist_index, order[last - 1]);
    app.handle_input(&Button::Left, &vfs);
    assert_eq!(app.playlist_index, order[last - 2]);
}

#[test]
fn shuffle_off_plays_in_order() {
    let (vfs, mut app) = music_with_playlist(3);
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.playlist_index, 1);
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.playlist_index, 0);
}

#[test]
fn shuffle_single_track_stays_put() {
    let (vfs, mut app) = music_with_playlist(1);
    app.handle_input(&Button::Select, &vfs);
    app.handle_input(&Button::Right, &vfs);
    assert_eq!(app.playlist_index, 0);
}

#[test]
fn photo_cancel_resets_state() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.open_file(&vfs, "/home/user/photos/sunset.png");
    app.handle_input(&Button::Square, &vfs); // rotate
    app.handle_input(&Button::Triangle, &vfs); // zoom
    app.handle_input(&Button::Cancel, &vfs); // back to listing
    assert_eq!(app.rotation(), 0);
    assert_eq!(app.zoom_level(), 1);
    assert!(!app.slideshow_active());
}

// -- Theming + playback progress --

use oasis_skin::ActiveTheme;
use oasis_test_backend::{DrawCommand, RecordingBackend};
use oasis_types::backend::Color;

const W: u32 = 400;
const H: u32 = 240;

fn skin_theme(name: &str) -> ActiveTheme {
    let skin = oasis_skin::builtin::load_builtin(name).expect("skin");
    ActiveTheme::from_skin(&skin.theme)
}

fn draw_cmds(app: &BrowsingApp, at: &ActiveTheme) -> Vec<DrawCommand> {
    let mut backend = RecordingBackend::new(W, H);
    app.draw_windowed(0, 0, W, H, &mut backend, at)
        .expect("draw");
    backend.commands().to_vec()
}

/// Color of the first full-window fill (the app background).
fn background(cmds: &[DrawCommand]) -> Color {
    cmds.iter()
        .find_map(|c| match c {
            DrawCommand::FillRect { w, h, color, .. } if *w == W && *h == H => Some(*color),
            _ => None,
        })
        .expect("background fill")
}

fn open_music() -> (MemoryVfs, BrowsingApp) {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::music_player("/apps/music", &vfs);
    app.open_file(&vfs, "/home/user/music/ambient_dawn.mp3");
    (vfs, app)
}

#[test]
fn music_background_follows_skin() {
    let (_vfs, app) = open_music();
    let light = skin_theme("paper");
    let dark = skin_theme("classic");
    let (bl, bd) = (
        background(&draw_cmds(&app, &light)),
        background(&draw_cmds(&app, &dark)),
    );
    assert_ne!(
        bl, bd,
        "light and dark skins must paint different backgrounds"
    );
    assert_eq!(bl, light.app.bg);
    assert_eq!(bd, dark.app.bg);
}

#[test]
fn photo_background_follows_skin() {
    let vfs = setup_vfs();
    let mut app = BrowsingApp::photo_viewer("/apps/photos", &vfs);
    app.open_file(&vfs, "/home/user/photos/sunset.png");
    let light = skin_theme("paper");
    let dark = skin_theme("classic");
    assert_ne!(
        background(&draw_cmds(&app, &light)),
        background(&draw_cmds(&app, &dark))
    );
    // The undecodable placeholder message is centered using the
    // measured width.
    let cmds = draw_cmds(&app, &dark);
    let (x, text) = cmds
        .iter()
        .find_map(|c| match c {
            DrawCommand::DrawText { text, x, .. } if text.contains("preview") => {
                Some((*x, text.clone()))
            },
            _ => None,
        })
        .expect("placeholder text");
    let w = oasis_types::backend::bitmap_measure_text(&text, dark.font_body) as i32;
    assert!((x - (W as i32 - w) / 2).abs() <= 1, "x={x} w={w}");
}

#[test]
fn music_progress_hidden_until_position_known() {
    let (_vfs, mut app) = open_music();
    let at = skin_theme("classic");
    let colors = palette::MusicColors::from_theme(&at);
    let rest = draw_cmds(&app, &at);
    assert!(
        !rest
            .iter()
            .any(|c| matches!(c, DrawCommand::FillRect { h: 4, .. })),
        "no fake progress bar"
    );

    app.set_playback(30_000, 120_000);
    let cmds = draw_cmds(&app, &at);
    let track_w = cmds
        .iter()
        .find_map(|c| match c {
            DrawCommand::FillRect { w, h: 4, color, .. } if *color == colors.progress_track => {
                Some(*w)
            },
            _ => None,
        })
        .expect("progress track drawn");
    let fill_w = cmds
        .iter()
        .find_map(|c| match c {
            DrawCommand::FillRect { w, h: 4, color, .. } if *color == colors.progress_fill => {
                Some(*w)
            },
            _ => None,
        })
        .expect("progress fill drawn");
    assert_eq!(fill_w, music_ui::progress_width(30_000, 120_000, track_w));
    assert!((fill_w as i32 - track_w as i32 / 4).abs() <= 1);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, DrawCommand::DrawText { text, .. } if text == "0:30 / 2:00"))
    );
}

#[test]
fn position_report_parsing() {
    let p = "/home/user/music/a.mp3";
    assert_eq!(
        parse_position_report("1500 90000 /home/user/music/a.mp3", p),
        Some((1500, 90000))
    );
    assert_eq!(
        parse_position_report("1500 90000 /other.mp3", p),
        None,
        "stale track"
    );
    assert_eq!(
        parse_position_report("1500 0 /home/user/music/a.mp3", p),
        None,
        "unknown dur"
    );
    assert_eq!(parse_position_report("garbage", p), None);
    assert_eq!(parse_position_report("", p), None);
}

#[test]
fn tick_polls_host_position() {
    let (mut vfs, mut app) = open_music();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/var").unwrap();
    oasis_vfs::Vfs::mkdir(&mut vfs, "/var/audio").unwrap();
    // No report yet: nothing changes, bar stays hidden.
    assert!(!app.tick(16, &vfs));
    assert_eq!(app.playback(), None);

    let report = "5000 10000 /home/user/music/ambient_dawn.mp3";
    oasis_vfs::Vfs::write(&mut vfs, MEDIA_POSITION_PATH, report.as_bytes()).unwrap();
    assert!(app.tick(16, &vfs), "new position requests a redraw");
    assert_eq!(app.playback(), Some((5000, 10000)));
    assert!(!app.tick(16, &vfs), "unchanged position");

    // Opening another track discards the old position; the stale
    // report (for the previous file) is ignored.
    app.open_file(&vfs, "/home/user/music/nightfall_theme.mp3");
    assert_eq!(app.playback(), None);
    app.tick(16, &vfs);
    assert_eq!(app.playback(), None);
}

#[test]
fn music_ui_uses_theme_fonts() {
    let (_vfs, app) = open_music();
    let at = skin_theme("classic");
    let cmds = draw_cmds(&app, &at);
    let size_of = |needle: &str| {
        cmds.iter().find_map(|c| match c {
            DrawCommand::DrawText {
                text, font_size, ..
            } if text.starts_with(needle) => Some(*font_size),
            _ => None,
        })
    };
    assert_eq!(size_of("Now Playing"), Some(at.font_hint));
    assert_eq!(size_of("ambient_dawn"), Some(at.font_body));
}
