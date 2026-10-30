//! Tests for the icon presets.

use super::pixel::{PIXEL_BAND, PIXEL_BORDER, PIXEL_SIZE, pixel_files};
use super::*;

#[test]
fn classifier_matches_common_titles() {
    assert_eq!(
        IconCategory::from_app_title("Browser"),
        IconCategory::Browser
    );
    assert_eq!(
        IconCategory::from_app_title("File Manager"),
        IconCategory::Files
    );
    assert_eq!(
        IconCategory::from_app_title("Music Player"),
        IconCategory::Audio
    );
    assert_eq!(IconCategory::from_app_title("TV Guide"), IconCategory::Tv);
    assert_eq!(
        IconCategory::from_app_title("Internet Radio"),
        IconCategory::Radio
    );
    assert_eq!(
        IconCategory::from_app_title("Settings"),
        IconCategory::Settings
    );
    assert_eq!(IconCategory::from_app_title("YouTube"), IconCategory::Video);
    assert_eq!(
        IconCategory::from_app_title("Photo Viewer"),
        IconCategory::Gallery
    );
    assert_eq!(
        IconCategory::from_app_title("Weather"),
        IconCategory::Weather
    );
    assert_eq!(
        IconCategory::from_app_title("Network"),
        IconCategory::Network
    );
    assert_eq!(
        IconCategory::from_app_title("Shutdown"),
        IconCategory::Power
    );
    assert_eq!(IconCategory::from_app_title("Home"), IconCategory::Home);
}

#[test]
fn classifier_is_case_insensitive() {
    assert_eq!(
        IconCategory::from_app_title("BROWSER"),
        IconCategory::Browser
    );
    assert_eq!(
        IconCategory::from_app_title("browser"),
        IconCategory::Browser
    );
}

#[test]
fn unknown_titles_are_generic() {
    assert_eq!(
        IconCategory::from_app_title("Random"),
        IconCategory::Generic
    );
    assert_eq!(IconCategory::from_app_title(""), IconCategory::Generic);
}

#[test]
fn file_check_beats_generic() {
    // "File Manager" contains "manager" but should still be Files.
    assert_eq!(
        IconCategory::from_app_title("File Manager"),
        IconCategory::Files
    );
}

#[test]
fn outline_icons_produce_ops_for_all_categories() {
    let color = Color::WHITE;
    for cat in IconCategory::ALL {
        let icon = outline_icon(cat, color);
        assert!(!icon.ops.is_empty(), "no ops for {cat:?}");
        assert_eq!(icon.width, SIZE);
        assert_eq!(icon.height, SIZE);
    }
}

#[test]
fn solid_icons_produce_ops_for_all_categories() {
    let color = Color::WHITE;
    for cat in IconCategory::ALL {
        let icon = solid_icon(cat, color);
        assert!(!icon.ops.is_empty(), "no ops for {cat:?}");
        assert_eq!(icon.width, SIZE);
        assert_eq!(icon.height, SIZE);
    }
}

#[test]
fn classifier_routes_new_categories() {
    use IconCategory::*;
    for (title, cat) in [
        ("Package Manager", Packages),
        ("App Store", Packages),
        ("System Monitor", Monitor),
        ("Task Manager", Monitor),
        ("Calculator", Calculator),
        ("Paint", Paint),
        ("Drawing Pad", Paint),
        ("Text Editor", TextEditor),
        ("Notepad", TextEditor),
        ("Games", Games),
        ("Arcade", Games),
    ] {
        assert_eq!(IconCategory::from_app_title(title), cat, "{title}");
    }
}

#[test]
fn every_icon_stays_inside_its_box() {
    type IconFn = fn(IconCategory, Color) -> IconDef;
    let sets: [(&str, IconFn); 3] = [
        ("outline", outline_icon),
        ("solid", solid_icon),
        ("pixel", pixel_icon),
    ];
    for (set, make) in sets {
        for cat in IconCategory::ALL {
            let icon = make(cat, Color::WHITE);
            let (x0, y0, x1, y1) = ops_bounds(&icon.ops).expect("icon has ops");
            assert!(
                x0 >= 0 && y0 >= 0 && x1 <= icon.width as i32 && y1 <= icon.height as i32,
                "{set} {cat:?} bounds ({x0},{y0})-({x1},{y1}) escape {}x{}",
                icon.width,
                icon.height
            );
        }
    }
}

#[test]
fn ops_bounds_samples_partial_arcs() {
    // Upper half arc: contributes nothing below its centre line.
    let arc = VectorOp::StrokeArc {
        cx: 10,
        cy: 10,
        radius: 5,
        start_angle: core::f32::consts::PI,
        end_angle: core::f32::consts::TAU,
        width: 2,
        color: Color::WHITE,
    };
    let (x0, y0, x1, y1) = ops_bounds(&[arc]).expect("bounds");
    assert_eq!((x0, y0, x1), (4, 4, 16));
    assert!(y1 <= 10);
    assert_eq!(ops_bounds(&[]), None);
}

#[test]
fn internet_explorer_routes_to_browser_not_files() {
    // Regression: bare `explorer` substring in Files would swallow
    // "Internet Explorer" before the Browser branch runs.
    assert_eq!(
        IconCategory::from_app_title("Internet Explorer"),
        IconCategory::Browser
    );
    // Plain "Explorer" still falls through to Files (Windows Explorer).
    assert_eq!(
        IconCategory::from_app_title("Explorer"),
        IconCategory::Files
    );
}

#[test]
fn guide_suffix_does_not_force_tv_category() {
    // Regression: bare `guide` was matching "User Guide" / "Help Guide".
    assert_eq!(
        IconCategory::from_app_title("User Guide"),
        IconCategory::Generic
    );
    assert_eq!(
        IconCategory::from_app_title("Help Guide"),
        IconCategory::Generic
    );
    // "TV Guide" still resolves to Tv via the "tv" substring.
    assert_eq!(IconCategory::from_app_title("TV Guide"), IconCategory::Tv);
}

#[test]
fn audio_streamer_routes_to_audio_not_video() {
    // Regression: `stream` in the Video branch was capturing "Audio
    // Streamer" before the Audio branch could match it.
    assert_eq!(
        IconCategory::from_app_title("Audio Streamer"),
        IconCategory::Audio
    );
    // "YouTube" / "Video" titles still resolve to Video.
    assert_eq!(IconCategory::from_app_title("YouTube"), IconCategory::Video);
    assert_eq!(
        IconCategory::from_app_title("Video Player"),
        IconCategory::Video
    );
}

#[test]
fn classifier_routes_terminal_titles() {
    assert_eq!(
        IconCategory::from_app_title("Terminal"),
        IconCategory::Terminal
    );
    assert_eq!(
        IconCategory::from_app_title("Agent Terminal"),
        IconCategory::Terminal
    );
    assert_eq!(
        IconCategory::from_app_title("Console"),
        IconCategory::Terminal
    );
}

#[test]
fn pixel_icons_produce_ops_for_all_categories() {
    let color = Color::WHITE;
    for cat in IconCategory::ALL {
        let icon = pixel_icon(cat, color);
        // Every pixel icon must include the 4-rect chrome container plus at
        // least one glyph op.
        assert!(
            icon.ops.len() >= 5,
            "pixel icon {cat:?} missing chrome + glyph ops"
        );
        assert_eq!(icon.width, PIXEL_SIZE);
        assert_eq!(icon.height, PIXEL_SIZE);
    }
}

#[test]
fn pixel_chrome_is_first_four_ops() {
    // Regression: the container chrome must render first so the glyph sits
    // on top of the silver tile body.
    let icon = pixel_files(Color::WHITE);
    match &icon.ops[0] {
        VectorOp::FillRect { color, .. } => assert_eq!(*color, PIXEL_BORDER),
        other => panic!("expected border rect first, got {other:?}"),
    }
    match &icon.ops[2] {
        VectorOp::FillRect { color, .. } => assert_eq!(*color, PIXEL_BAND),
        other => panic!("expected band rect third, got {other:?}"),
    }
}

#[test]
fn gear_teeth_polygon_has_expected_vertex_count() {
    let pts = gear_teeth_polygon(12, 12, 10, 7, 8);
    assert_eq!(pts.len(), 32); // 4 vertices per tooth × 8 teeth.
}
