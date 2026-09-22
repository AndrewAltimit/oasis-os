//! End-to-end Music Player and Photo Viewer sessions driven through the
//! `App` trait (gamepad / keyboard input, ticks, host VFS IPC), checked
//! against app state, the rendered output and backend texture lifetime.
//!
//! Neither app implements `handle_click` yet, so these sessions use the
//! d-pad / keyboard path every host supports.
#![allow(clippy::unwrap_used)]

use oasis_app_core::testing::{AppHarness, THEMES, fuzz_app, theme};
use oasis_app_core::{App, AppAction};
use oasis_app_media::{
    BrowsingApp, MEDIA_POSITION_PATH, MEDIA_REQUEST_PATH, SLIDESHOW_INTERVAL_MS,
};
use oasis_test_backend::ClipAuditBackend;
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

const MUSIC: &str = "/home/user/music";
const PHOTOS: &str = "/home/user/photos";

fn mkdirs(vfs: &mut MemoryVfs, path: &str) {
    let mut cur = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        cur.push('/');
        cur.push_str(part);
        if !vfs.exists(&cur) {
            vfs.mkdir(&cur).unwrap();
        }
    }
}

/// ID3v2.3 tag with a TIT2 frame, followed by a 128 kbps MPEG frame header
/// and `audio_len` bytes of "audio".
fn tagged_mp3(title: &str, audio_len: usize) -> Vec<u8> {
    let mut frame = b"TIT2".to_vec();
    frame.extend_from_slice(&((title.len() + 1) as u32).to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0]);
    frame.extend_from_slice(title.as_bytes());
    let size = frame.len() as u32;
    let mut out = b"ID3\x03\x00\x00".to_vec();
    out.extend_from_slice(&[
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ]);
    out.extend_from_slice(&frame);
    out.extend_from_slice(&plain_mp3(audio_len));
    out
}

/// Bare MPEG-1 layer III frames at 128 kbps.
fn plain_mp3(len: usize) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFB, 0x90, 0x00];
    out.resize(len.max(4), 0);
    out
}

fn music_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    mkdirs(&mut vfs, &format!("{MUSIC}/albums"));
    vfs.write(
        &format!("{MUSIC}/a_tagged.mp3"),
        &tagged_mp3("Sunrise Song", 32_000),
    )
    .unwrap();
    // 128 kbps * 60 s = 960 000 bytes -> "01:00".
    vfs.write(&format!("{MUSIC}/b_plain_track.mp3"), &plain_mp3(960_000))
        .unwrap();
    vfs.write(
        &format!("{MUSIC}/c_garbage.mp3"),
        b"\x00\xffID3 not really\xff",
    )
    .unwrap();
    vfs.write(&format!("{MUSIC}/d_empty.mp3"), b"").unwrap();
    vfs.write(&format!("{MUSIC}/albums/deep.mp3"), &plain_mp3(100))
        .unwrap();
    vfs
}

fn music(vfs: MemoryVfs) -> AppHarness {
    let app = BrowsingApp::music_player("/apps/Music Player", &vfs);
    AppHarness::with_vfs(Box::new(app), vfs)
}

fn browsing(h: &AppHarness) -> &BrowsingApp {
    h.app_as::<BrowsingApp>()
}

/// Move the list cursor to the row whose text starts with `name`.
fn select_row(h: &mut AppHarness, name: &str) {
    for _ in 0..50 {
        let app = browsing(h);
        let idx = app.content.scroll + app.content.cursor;
        if app.lines().get(idx).is_some_and(|l| l.starts_with(name)) {
            return;
        }
        h.press(Button::Down);
    }
    // Wrap to the top and try once more.
    for _ in 0..50 {
        h.press(Button::Up);
    }
    for _ in 0..50 {
        let app = browsing(h);
        let idx = app.content.scroll + app.content.cursor;
        if app.lines().get(idx).is_some_and(|l| l.starts_with(name)) {
            return;
        }
        h.press(Button::Down);
    }
    panic!("row {name:?} not reachable in {:?}", browsing(h).lines());
}

fn viewing(h: &AppHarness) -> Option<String> {
    h.app().viewing_file().map(str::to_string)
}

#[test]
fn music_listing_playlist_next_prev_and_ipc() {
    let mut h = music(music_vfs());
    let screen = h.screen_text();
    for name in [
        "albums/",
        "a_tagged.mp3",
        "b_plain_track.mp3",
        "c_garbage.mp3",
    ] {
        assert!(screen.contains(name), "{name} not listed:\n{screen}");
    }

    // Queue three tracks with Triangle; directories and duplicates are
    // ignored.
    select_row(&mut h, "albums/");
    h.press(Button::Triangle);
    for name in ["a_tagged", "b_plain_track", "c_garbage", "a_tagged"] {
        select_row(&mut h, name);
        h.press(Button::Triangle);
    }
    let playlist: Vec<_> = browsing(&h)
        .playlist()
        .iter()
        .map(|p| p.rsplit('/').next().unwrap().to_string())
        .collect();
    assert_eq!(
        playlist,
        ["a_tagged.mp3", "b_plain_track.mp3", "c_garbage.mp3"]
    );

    // Confirm opens the track: ID3 title on screen, play request for the host.
    select_row(&mut h, "a_tagged");
    h.press(Button::Confirm);
    assert_eq!(
        viewing(&h).as_deref(),
        Some("/home/user/music/a_tagged.mp3")
    );
    assert_eq!(browsing(&h).track_info().0, Some("Sunrise Song"));
    assert!(h.screen_text().contains("Sunrise Song"));
    assert_eq!(
        h.app_mut().take_pending_request(),
        Some((
            MEDIA_REQUEST_PATH.to_string(),
            "play_file /home/user/music/a_tagged.mp3".to_string()
        ))
    );

    // Right / Left step through the playlist (wrapping).
    h.press(Button::Right);
    assert_eq!(
        viewing(&h).as_deref(),
        Some("/home/user/music/b_plain_track.mp3")
    );
    // No ID3 tag: title from the file name, duration from the bitrate.
    assert_eq!(
        browsing(&h).track_info(),
        (Some("b plain track"), Some("01:00"), Some(960_000))
    );
    let screen = h.screen_text();
    assert!(screen.contains("b plain track") && screen.contains("Duration: 01:00"));
    h.press(Button::Right);
    assert!(viewing(&h).unwrap().ends_with("c_garbage.mp3"));
    // Garbage bytes: no duration, title from the file name, no panic.
    assert_eq!(browsing(&h).track_info().1, None);
    h.press(Button::Right);
    assert!(
        viewing(&h).unwrap().ends_with("a_tagged.mp3"),
        "wraps to first"
    );
    h.press(Button::Left);
    assert!(
        viewing(&h).unwrap().ends_with("c_garbage.mp3"),
        "wraps to last"
    );

    // The host reports progress through the VFS; stale reports are ignored.
    let current = viewing(&h).unwrap();
    h.vfs_mut().mkdir("/var").ok();
    h.vfs_mut().mkdir("/var/audio").ok();
    h.vfs_mut()
        .write(
            MEDIA_POSITION_PATH,
            b"30000 60000 /home/user/music/other.mp3",
        )
        .unwrap();
    h.frame(16);
    assert_eq!(browsing(&h).playback(), None);
    h.vfs_mut()
        .write(
            MEDIA_POSITION_PATH,
            format!("30000 60000 {current}").as_bytes(),
        )
        .unwrap();
    assert!(h.frame(16));
    assert_eq!(browsing(&h).playback(), Some((30_000, 60_000)));
    assert!(h.screen_text().contains("0:30 / 1:00"));

    // Cancel stops playback and returns to the listing; Cancel again exits.
    h.app_mut().take_pending_request();
    assert_eq!(h.press(Button::Cancel), AppAction::None);
    assert_eq!(viewing(&h), None);
    assert_eq!(
        h.app_mut().take_pending_request().map(|r| r.1),
        Some("stop".to_string())
    );
    assert!(h.screen_text().contains("a_tagged.mp3"));
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn music_shuffle_never_repeats_and_previous_retraces() {
    let mut vfs = MemoryVfs::new();
    mkdirs(&mut vfs, MUSIC);
    for i in 0..6 {
        vfs.write(&format!("{MUSIC}/t{i}.mp3"), &plain_mp3(1000))
            .unwrap();
    }
    let mut h = music(vfs);
    for i in 0..6 {
        select_row(&mut h, &format!("t{i}"));
        h.press(Button::Triangle);
    }
    select_row(&mut h, "t0");
    h.press(Button::Confirm);
    h.press(Button::Select);
    assert!(browsing(&h).shuffle());
    assert!(h.screen_text().contains("Shuffle: ON"));

    let mut order = vec![viewing(&h).unwrap()];
    for _ in 0..12 {
        h.frame(16); // Frame timing feeds the shuffle PRNG.
        h.press(Button::Right);
        let now = viewing(&h).unwrap();
        assert_ne!(Some(&now), order.last(), "shuffle repeated a track");
        order.push(now);
    }
    let distinct: std::collections::HashSet<_> = order.iter().collect();
    assert!(distinct.len() >= 3, "shuffle is stuck: {order:?}");
    // Previous walks the shuffled order backwards.
    for expected in order.iter().rev().skip(1).take(5) {
        h.press(Button::Left);
        assert_eq!(viewing(&h).as_ref(), Some(expected));
    }
    // Shuffle off: plain sequential order again.
    h.press(Button::Select);
    assert!(!browsing(&h).shuffle());
    let before = viewing(&h).unwrap();
    let idx: usize = before
        .rsplit('/')
        .next()
        .unwrap()
        .trim_start_matches('t')
        .trim_end_matches(".mp3")
        .parse()
        .unwrap();
    h.press(Button::Right);
    assert_eq!(
        viewing(&h).unwrap(),
        format!("{MUSIC}/t{}.mp3", (idx + 1) % 6)
    );
}

#[test]
fn music_directory_navigation_and_missing_folder() {
    let mut h = music(music_vfs());
    select_row(&mut h, "albums/");
    h.press(Button::Confirm);
    assert_eq!(h.app().browse_dir(), Some("/home/user/music/albums"));
    assert!(h.screen_text().contains("deep.mp3"));
    select_row(&mut h, "..");
    h.press(Button::Confirm);
    assert_eq!(h.app().browse_dir(), Some(MUSIC));
    // Empty track file opens without panicking.
    select_row(&mut h, "d_empty");
    h.press(Button::Confirm);
    assert!(viewing(&h).unwrap().ends_with("d_empty.mp3"));
    assert_eq!(browsing(&h).track_info().2, Some(0));
    let _ = h.draw();

    // No music folder at all: a helpful message, not a crash.
    let mut h = music(MemoryVfs::new());
    assert!(h.screen_text().contains("Music directory not found"));
    h.press(Button::Confirm);
    h.press(Button::Triangle);
    h.press(Button::Right);
    assert!(browsing(&h).playlist().is_empty());
    assert_eq!(h.key(Key::Escape), AppAction::Exit);
}

#[test]
fn music_player_at_opens_track_directly() {
    let vfs = music_vfs();
    let app = BrowsingApp::music_player_at("/apps/m", "/home/user/music/a_tagged.mp3", &vfs);
    let mut h = AppHarness::with_vfs(Box::new(app), vfs);
    assert_eq!(browsing(&h).track_info().0, Some("Sunrise Song"));
    assert_eq!(h.app().browse_dir(), Some(MUSIC));
    h.draw_all_sizes_and_themes();
    h.press(Button::Cancel);
    assert!(h.screen_text().contains("b_plain_track.mp3"));
}

// ── Photo viewer ──────────────────────────────────────────────────

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        let data: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        writer.write_image_data(&data).unwrap();
    }
    buf
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
    out
}

/// A well-formed PNG header claiming a `w` x `h` RGBA image, with a tiny
/// (far too short) IDAT: exercises allocation limits, not the inflater.
fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend(chunk(b"IHDR", &ihdr));
    out.extend(chunk(
        b"IDAT",
        &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    out.extend(chunk(b"IEND", &[]));
    out
}

/// Baseline JPEG SOF0 claiming `w` x `h`, then truncated.
fn jpeg_claiming(w: u16, h: u16) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
    out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x0C, 3, 1, 0, 2, 0x11, 3, 0x11, 0, 63, 0]);
    out.extend_from_slice(&[0x12; 64]);
    out
}

fn photo_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    mkdirs(&mut vfs, PHOTOS);
    vfs.write(&format!("{PHOTOS}/a_small.png"), &png(4, 3))
        .unwrap();
    vfs.write(&format!("{PHOTOS}/b_wide.png"), &png(1500, 10))
        .unwrap();
    let mut truncated = png(8, 8);
    truncated.truncate(truncated.len() / 2);
    vfs.write(&format!("{PHOTOS}/c_truncated.png"), &truncated)
        .unwrap();
    vfs.write(&format!("{PHOTOS}/d_zero.png"), b"").unwrap();
    vfs.write(
        &format!("{PHOTOS}/e_huge.png"),
        &png_claiming(60_000, 60_000),
    )
    .unwrap();
    vfs.write(
        &format!("{PHOTOS}/f_huge.jpg"),
        &jpeg_claiming(65_000, 65_000),
    )
    .unwrap();
    vfs.write(&format!("{PHOTOS}/g_text.jpg"), b"just text")
        .unwrap();
    vfs.write(&format!("{PHOTOS}/notes.txt"), b"not an image")
        .unwrap();
    // Names that merely contain an image extension are not images.
    vfs.write(&format!("{PHOTOS}/b_readme.png.txt"), b"text")
        .unwrap();
    mkdirs(&mut vfs, &format!("{PHOTOS}/f.jpg.d"));
    vfs
}

fn photos(vfs: MemoryVfs) -> AppHarness {
    let app = BrowsingApp::photo_viewer("/apps/Photo Viewer", &vfs);
    AppHarness::with_vfs(Box::new(app), vfs)
}

/// Draw with a persistent backend (textures survive across frames, like
/// a real host).
fn draw_into(h: &AppHarness, b: &mut ClipAuditBackend) {
    b.clear_records();
    b.push_host_clip();
    h.app()
        .draw_windowed(8, 20, 480, 272, b, &theme("classic"))
        .unwrap();
    b.pop_host_clip();
    assert!(b.violations().is_empty(), "{:?}", b.violations());
}

fn blits(b: &ClipAuditBackend) -> usize {
    b.rects().iter().filter(|r| r.color.is_none()).count()
}

#[test]
fn photo_viewer_shows_valid_images_and_reports_broken_ones() {
    let mut h = photos(photo_vfs());
    let mut b = ClipAuditBackend::new(1100, 800, (8, 20, 480, 272));
    select_row(&mut h, "a_small.png");
    h.press(Button::Confirm);
    let img = browsing(&h).decoded_image().unwrap();
    assert_eq!((img.width, img.height), (4, 3));
    draw_into(&h, &mut b);
    assert_eq!(blits(&b), 1, "image blitted");
    assert!(b.visible_text().contains("a_small.png  4x3"));
    assert_eq!(b.live_texture_count(), 1);

    // Right walks the images in the folder (skipping notes.txt,
    // b_readme.png.txt and the f.jpg.d/ folder): a wide
    // image is downscaled to the decode budget, broken ones show a
    // placeholder instead of a texture.
    let expect = [
        ("b_wide.png", Some((1024, 6))),
        ("c_truncated.png", None),
        ("d_zero.png", None),
        ("e_huge.png", None),
        ("f_huge.jpg", None),
        ("g_text.jpg", None),
        ("a_small.png", Some((4, 3))),
    ];
    for (name, dims) in expect {
        h.press(Button::Right);
        assert!(viewing(&h).unwrap().ends_with(name), "expected {name}");
        let got = browsing(&h).decoded_image().map(|i| (i.width, i.height));
        assert_eq!(got, dims, "{name}");
        draw_into(&h, &mut b);
        if dims.is_some() {
            assert_eq!(blits(&b), 1, "{name}");
        } else {
            assert_eq!(blits(&b), 0, "{name}");
            assert!(
                b.visible_text().contains("(preview not available)"),
                "{name}: {}",
                b.visible_text()
            );
        }
        // Old textures are released: never more than the one on screen.
        assert!(b.live_texture_count() <= 1, "{name}: texture leak");
    }
    h.press(Button::Left);
    assert!(viewing(&h).unwrap().ends_with("g_text.jpg"));
}

#[test]
fn photo_zoom_rotate_and_slideshow() {
    let mut h = photos(photo_vfs());
    select_row(&mut h, "a_small.png");
    h.press(Button::Confirm);
    h.press(Button::Triangle);
    assert_eq!(browsing(&h).zoom_level(), 2);
    assert!(h.screen_text().contains("4x3  2x"));
    h.press(Button::Triangle);
    h.press(Button::Triangle);
    assert_eq!(browsing(&h).zoom_level(), 1);
    for want in [90, 180, 270, 0] {
        h.press(Button::Square);
        assert_eq!(browsing(&h).rotation(), want);
    }
    // Zoomed-in images overflow their area but stay inside the window.
    h.press(Button::Triangle);
    h.press(Button::Triangle);
    h.draw_all_sizes_and_themes();

    // Slideshow advances once per interval, by wall time not frames.
    h.press(Button::Start);
    assert!(browsing(&h).slideshow_active());
    assert!(!h.frame(SLIDESHOW_INTERVAL_MS - 1));
    assert!(viewing(&h).unwrap().ends_with("a_small.png"));
    assert!(h.frame(1));
    assert!(viewing(&h).unwrap().ends_with("b_wide.png"));
    // A long stall advances a single slide.
    assert!(h.frame(SLIDESHOW_INTERVAL_MS * 10));
    assert!(viewing(&h).unwrap().ends_with("c_truncated.png"));
    h.press(Button::Start);
    assert!(!h.frame(SLIDESHOW_INTERVAL_MS * 2));

    // Cancel resets view state and returns to the gallery.
    h.press(Button::Cancel);
    assert_eq!(viewing(&h), None);
    assert_eq!(browsing(&h).zoom_level(), 1);
    assert!(!browsing(&h).slideshow_active());
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn photo_viewer_at_and_all_themes() {
    let vfs = photo_vfs();
    let app = BrowsingApp::photo_viewer_at("/apps/p", "/home/user/photos/b_wide.png", &vfs);
    let mut h = AppHarness::with_vfs(Box::new(app), vfs);
    assert!(browsing(&h).decoded_image().is_some());
    h.draw_all_sizes_and_themes();
    for name in THEMES {
        h.set_theme(name);
        assert!(h.screen_text().contains("b_wide.png"), "{name}");
    }
}

/// Image headers too short for the metadata the text viewer reads.
/// `oasis_app_core::file_viewer::view_image_file` indexes `data[24]` /
/// `data[25]` after checking only `len >= 24` (PNG) and `data[6..10]`
/// after `len >= 6` (GIF), so a 24-byte PNG or a bare `GIF89a` panics.
#[test]
#[ignore = "panics in oasis-app-core file_viewer::view_image_file (reported, not in this crate)"]
fn photo_viewer_survives_minimal_headers() {
    let mut vfs = MemoryVfs::new();
    mkdirs(&mut vfs, PHOTOS);
    vfs.write(&format!("{PHOTOS}/a.png"), &png(1, 1)[..24])
        .unwrap();
    vfs.write(&format!("{PHOTOS}/b.gif"), b"GIF89a").unwrap();
    let mut h = photos(vfs);
    select_row(&mut h, "a.png");
    h.press(Button::Confirm);
    h.press(Button::Right);
    let _ = h.draw();
}

#[test]
fn fuzz_music_and_photos() {
    let music = |vfs: &dyn Vfs| -> Box<dyn App> {
        Box::new(BrowsingApp::music_player("/apps/Music Player", vfs))
    };
    let photos = |vfs: &dyn Vfs| -> Box<dyn App> {
        Box::new(BrowsingApp::photo_viewer("/apps/Photo Viewer", vfs))
    };
    for seed in 1..=3 {
        fuzz_app(&music, music_vfs(), seed, 3000);
        fuzz_app(&photos, photo_vfs(), seed, 3000);
    }
}
