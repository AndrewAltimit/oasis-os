//! Every built-in app keeps its text inside its window at every skin
//! resolution: from a cramped 320x200 window (a PSP-size skin's typical
//! app window) up to XGA, under every coverage theme.
//!
//! The window clip hides overflow, so an app whose labels run past its
//! edges still passes the clip audit; it just shows glyphs cut in half.
//! Here a visible text draw must lie inside the content rect.

use oasis_app_core::testing::{AppHarness, THEMES};
use oasis_apps::{HostInfo, builtin_registry};
use oasis_vfs::{MemoryVfs, Vfs};

/// Window content sizes checked (smaller windows only need to stay
/// clipped, which the per-app clip audits cover).
const SIZES: &[(u32, u32)] = &[(320, 200), (480, 272), (800, 600), (1024, 768)];

fn vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    for d in ["/home", "/home/user", "/home/user/music", "/home/user/photos"] {
        vfs.mkdir(d).unwrap();
    }
    let long = "/home/user/a_really_long_file_name_to_overflow_narrow_panes.txt";
    vfs.write(long, b"hello").unwrap();
    vfs
}

#[test]
fn app_text_stays_inside_the_window_at_every_size_and_theme() {
    let reg = builtin_registry(&HostInfo::default());
    let mut overflows = Vec::new();
    for r in reg.iter() {
        for &(w, h) in SIZES {
            for theme in THEMES {
                let vfs = vfs();
                let app = r.create(&r.to_app_entry().path, &vfs);
                let mut harness = AppHarness::with_vfs(app, vfs);
                harness.set_size(w, h).set_theme(theme);
                harness.frames(3, 16);
                let b = harness.draw();
                // The harness window sits at (8, 20).
                let (x0, y0) = (8, 20);
                let (x1, y1) = (x0 + w as i32, y0 + h as i32);
                for t in b.texts() {
                    if !t.visible || t.text.trim().is_empty() {
                        continue;
                    }
                    let inside = t.x >= x0
                        && t.x + t.w as i32 <= x1 + 1
                        && t.y + t.h as i32 <= y1 + 1;
                    if !inside {
                        overflows.push(format!("{} {w}x{h} {theme}: {:?}", r.title, t.text));
                    }
                }
            }
        }
    }
    assert!(overflows.is_empty(), "text past the window edge:\n{}", overflows.join("\n"));
}
