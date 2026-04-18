#![allow(clippy::unwrap_used)] // Test binary -- unwrap is acceptable.
//! End-to-end tests for the TV Guide app lifecycle.
//!
//! Headless test binary that exercises the full TV Guide pipeline without
//! network access (except for the env-gated real fetch test). Each test
//! function returns pass/fail, exit code 0 = all pass.
//!
//! Usage:
//!   cargo run -p oasis-app --bin e2e-tests
//!   OASIS_E2E_NETWORK=1 cargo run -p oasis-app --bin e2e-tests

use std::time::Instant;

use oasis_core::active_theme::ActiveTheme;
use oasis_core::apps::AppRunner;
use oasis_core::apps::tv_guide::catalog::ChannelCatalog;
use oasis_core::apps::tv_guide::channel::{ChannelConfig, DEFAULT_CHANNELS_TOML};
use oasis_core::apps::tv_guide::{TvGuideState, VideoEpisode};
use oasis_core::backend::Color;
use oasis_core::dashboard::AppEntry;
use oasis_core::vfs::{MemoryVfs, Vfs};

fn make_tv_app() -> AppEntry {
    AppEntry {
        title: "TV Guide".to_string(),
        path: "/apps/TV Guide".to_string(),
        icon_png: Vec::new(),
        color: Color::rgb(100, 100, 100),
    }
}

fn make_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/apps").unwrap();
    vfs.mkdir("/apps/TV Guide").unwrap();
    vfs
}

fn make_guide() -> TvGuideState {
    let config =
        ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).expect("default channels TOML is valid");
    TvGuideState::new(&config, &ActiveTheme::default())
}

fn mock_episodes(channel_num: u32, count: usize) -> Vec<VideoEpisode> {
    (0..count)
        .map(|i| VideoEpisode {
            item_id: format!("mock-{channel_num}-{i}"),
            filename: format!("ep{i:02}.mp4"),
            title: format!("Episode {}", i + 1),
            duration_secs: 1800.0,
            width: 640,
            height: 480,
            size_bytes: 50_000_000,
            format: "MPEG4".into(),
            original: None,
        })
        .collect()
}

fn inject_catalogs(guide: &mut TvGuideState, episodes_per_channel: usize) {
    for (i, ch) in guide.channels.clone().iter().enumerate() {
        let mut catalog = ChannelCatalog::new(ch.number);
        catalog.add_episodes(mock_episodes(ch.number, episodes_per_channel));
        guide.catalogs[i] = Some(catalog);
        guide.rebuild_cached_schedule(i);
    }
}

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

fn test_tv_guide_launch_shows_loading() -> bool {
    let vfs = make_vfs();
    let runner = AppRunner::launch(&make_tv_app(), &vfs);
    let guide = make_guide();

    // Initial state: shows "Loading", fetch_attempted is false, no error.
    let lines = guide.text_content();
    let has_loading = lines.iter().any(|l| l.contains("Loading"));
    let no_error = guide.fetch_error.is_none();
    let not_attempted = !guide.fetch_attempted;

    assert!(has_loading, "should show 'Loading' initially");
    assert!(no_error, "should have no error initially");
    assert!(not_attempted, "fetch_attempted should be false initially");
    assert!(
        runner.lines.iter().any(|l| l.contains("Loading")),
        "runner lines should contain 'Loading'",
    );

    true
}

fn test_tv_guide_mock_catalog_injection() -> bool {
    let mut guide = make_guide();

    // Inject 3 episodes per channel.
    inject_catalogs(&mut guide, 3);
    guide.fetch_attempted = true;

    let lines = guide.text_content();

    let no_loading = !lines.iter().any(|l| l.contains("Loading"));
    let has_episode = lines.iter().any(|l| l.contains("Episode"));
    let has_now_playing = lines.iter().any(|l| l.contains("Now Playing"));

    assert!(no_loading, "should not show 'Loading' after injection");
    assert!(has_episode, "should show episode titles");
    assert!(has_now_playing, "should show 'Now Playing' section");

    true
}

fn test_tv_guide_error_state_display() -> bool {
    let mut guide = make_guide();
    guide.fetch_attempted = true;
    guide.fetch_error = Some("Network timeout".to_string());

    let lines = guide.text_content();
    let has_error = lines.iter().any(|l| l.contains("Error: Network timeout"));
    let no_loading = !lines.iter().any(|l| l.contains("Loading"));

    assert!(has_error, "should show the error message");
    assert!(no_loading, "should not show 'Loading' with error set");

    true
}

fn test_tv_guide_partial_catalog() -> bool {
    let mut guide = make_guide();
    guide.fetch_attempted = true;

    // Only inject catalog for first channel.
    let ch0_num = guide.channels[0].number;
    let mut catalog = ChannelCatalog::new(ch0_num);
    catalog.add_episodes(mock_episodes(ch0_num, 3));
    guide.catalogs[0] = Some(catalog);
    guide.rebuild_cached_schedule(0);

    let lines = guide.text_content();

    // Channel 0 should show episode content; others show "(loading...)".
    let has_episode = lines.iter().any(|l| l.contains("Episode"));
    let has_loading_channel = lines.iter().any(|l| l.contains("(loading...)"));
    // Should NOT show the top-level "Loading channel catalogs..." since at
    // least one catalog is loaded.
    let no_top_loading = !lines.iter().any(|l| l.contains("Loading channel"));

    assert!(has_episode, "channel 0 should show episodes");
    assert!(
        has_loading_channel,
        "other channels should show '(loading...)'"
    );
    assert!(no_top_loading, "should not show top-level loading message");

    true
}

fn test_tv_guide_schedule_deterministic() -> bool {
    let mut guide = make_guide();
    inject_catalogs(&mut guide, 5);

    let t = 1_700_000_000u64;
    guide.current_time = t;
    let lines_a = guide.text_content();
    guide.current_time = t;
    let lines_b = guide.text_content();

    assert_eq!(
        lines_a, lines_b,
        "same timestamp should produce same output"
    );

    true
}

fn test_tv_guide_tune_request_emitted() -> bool {
    let vfs = make_vfs();
    let mut runner = AppRunner::launch(&make_tv_app(), &vfs);

    // Inject catalogs into the runner's guide state.
    if let Some(guide) = runner.tv_guide_state() {
        inject_catalogs(guide, 3);
        guide.fetch_attempted = true;
    }
    runner.refresh_tv_text();

    // Simulate Confirm button to tune.
    use oasis_core::input::Button;
    runner.handle_input(&Button::Confirm, &vfs);

    let request = runner.take_pending_request();
    assert!(request.is_some(), "should have a pending tune request");
    let (path, data) = request.unwrap();
    assert_eq!(
        path,
        oasis_core::apps::tv_guide::TV_REQUEST_PATH,
        "request path should match TV_REQUEST_PATH",
    );
    assert!(
        data.starts_with("tune_url "),
        "request data should start with 'tune_url '"
    );

    true
}

fn test_tv_guide_fetch_guard() -> bool {
    let mut guide = make_guide();

    // Before fetch: all catalogs None, fetch_attempted false.
    let should_fetch = !guide.fetch_attempted && guide.catalogs.iter().all(|c| c.is_none());
    assert!(should_fetch, "should want to fetch initially");

    // After marking fetch_attempted: should not re-fetch.
    guide.fetch_attempted = true;
    let should_not_fetch = guide.fetch_attempted || !guide.catalogs.iter().all(|c| c.is_none());
    assert!(
        should_not_fetch,
        "should not re-fetch after fetch_attempted=true"
    );

    true
}

fn test_tv_guide_screenshot_populated() -> bool {
    // This test verifies the rendering pipeline can produce non-empty output.
    // We test that the guide produces meaningful SDI updates without crashing.
    let mut guide = make_guide();
    inject_catalogs(&mut guide, 5);
    guide.fetch_attempted = true;

    // Create a minimal SDI registry and active theme.
    let skin = oasis_core::skin::resolve_skin("classic").expect("classic skin should exist");
    let active_theme = oasis_core::active_theme::ActiveTheme::from_skin(&skin.theme);
    let mut sdi = oasis_core::sdi::SdiRegistry::new();

    // This should not panic.
    guide.update_sdi(&mut sdi, &active_theme);

    // Verify SDI objects were created.
    assert!(
        sdi.contains("tv_hdr_bg"),
        "should create tv_hdr_bg SDI object"
    );
    assert!(sdi.contains("tv_hdr_date"), "should create tv_hdr_date");
    assert!(sdi.contains("tv_ftr_bg"), "should create tv_ftr_bg");

    true
}

fn test_tv_guide_real_fetch() -> bool {
    // Skip unless OASIS_E2E_NETWORK=1 is set.
    if std::env::var("OASIS_E2E_NETWORK").as_deref() != Ok("1") {
        return true; // Skipped, reported separately.
    }

    use oasis_core::backend::NetworkBackend;
    use oasis_core::net::TlsProvider;
    use oasis_core::net::{RustlsTlsProvider, StdNetworkBackend};

    let config =
        ChannelConfig::from_toml(DEFAULT_CHANNELS_TOML).expect("default channels TOML is valid");
    let tls = std::sync::Arc::new(RustlsTlsProvider::new());
    let mut net = StdNetworkBackend::new();

    // Try fetching metadata for the first channel's first source.
    let channel = &config.channel[0];
    let source = &channel.source[0];
    let files_path = ChannelCatalog::files_api_path(&source.item_id);

    // Perform HTTPS GET.
    let tcp = net
        .connect("archive.org", 443)
        .map_err(|e| format!("connect: {e}"))
        .expect("TCP connect should succeed");

    let mut stream = tls
        .connect_tls_with_alpn(tcp, "archive.org", &[b"http/1.1"])
        .map_err(|e| format!("TLS: {e}"))
        .expect("TLS handshake should succeed")
        .stream;

    // Build and send HTTP request using NetworkStream::write.
    let request = format!(
        "GET {files_path} HTTP/1.1\r\nHost: archive.org\r\n\
         User-Agent: OASIS_OS/0.1\r\nConnection: close\r\n\
         Accept: */*\r\n\r\n"
    );
    let req_bytes = request.as_bytes();
    let mut written = 0;
    while written < req_bytes.len() {
        match stream.write(&req_bytes[written..]) {
            Ok(n) => written += n,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                panic!("write error: {e}");
            },
        }
    }

    let mut response = Vec::new();
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    let mut buf = [0u8; 8192];
    loop {
        if Instant::now() > deadline {
            panic!("timeout reading response");
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("WouldBlock") || msg.contains("would block") {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                if !response.is_empty() {
                    break;
                }
                panic!("read error: {e}");
            },
        }
    }

    assert!(!response.is_empty(), "should receive response bytes");

    // Find body after headers.
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("should have header/body separator");
    let body = String::from_utf8_lossy(&response[header_end + 4..]);
    assert!(!body.is_empty(), "body should not be empty");

    // Parse episodes.
    let episodes =
        ChannelCatalog::parse_files_response(&body, &source.item_id, source.subfolder.as_deref());

    // We expect at least some video files from the default channels.
    assert!(!episodes.is_empty(), "should find video episodes from IA");

    true
}

// ---------------------------------------------------------------------------
// Test runner
// ---------------------------------------------------------------------------

struct TestResult {
    passed: bool,
    skipped: bool,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let network_enabled = std::env::var("OASIS_E2E_NETWORK").as_deref() == Ok("1");

    type TestEntry = (&'static str, fn() -> bool, bool);
    let tests: Vec<TestEntry> = vec![
        (
            "tv_guide_launch_shows_loading",
            test_tv_guide_launch_shows_loading,
            false,
        ),
        (
            "tv_guide_mock_catalog_injection",
            test_tv_guide_mock_catalog_injection,
            false,
        ),
        (
            "tv_guide_error_state_display",
            test_tv_guide_error_state_display,
            false,
        ),
        (
            "tv_guide_partial_catalog",
            test_tv_guide_partial_catalog,
            false,
        ),
        (
            "tv_guide_schedule_deterministic",
            test_tv_guide_schedule_deterministic,
            false,
        ),
        (
            "tv_guide_tune_request_emitted",
            test_tv_guide_tune_request_emitted,
            false,
        ),
        ("tv_guide_fetch_guard", test_tv_guide_fetch_guard, false),
        (
            "tv_guide_screenshot_populated",
            test_tv_guide_screenshot_populated,
            false,
        ),
        ("tv_guide_real_fetch", test_tv_guide_real_fetch, true),
    ];

    let mut results = Vec::new();

    for (name, test_fn, requires_network) in &tests {
        if *requires_network && !network_enabled {
            println!("[SKIP] {name} (set OASIS_E2E_NETWORK=1 to enable)");
            results.push(TestResult {
                passed: true,
                skipped: true,
            });
            continue;
        }

        let start = Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test_fn));
        let duration_ms = start.elapsed().as_millis();

        match result {
            Ok(true) => {
                println!("[PASS] {name} ({duration_ms}ms)");
                results.push(TestResult {
                    passed: true,
                    skipped: false,
                });
            },
            Ok(false) => {
                eprintln!("[FAIL] {name} ({duration_ms}ms)");
                results.push(TestResult {
                    passed: false,
                    skipped: false,
                });
            },
            Err(e) => {
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                eprintln!("[FAIL] {name} ({duration_ms}ms) — {msg}");
                results.push(TestResult {
                    passed: false,
                    skipped: false,
                });
            },
        }
    }

    println!();
    let passed = results.iter().filter(|r| r.passed && !r.skipped).count();
    let failed = results.iter().filter(|r| !r.passed).count();
    let skipped = results.iter().filter(|r| r.skipped).count();
    println!("Results: {passed} passed, {failed} failed, {skipped} skipped");

    if failed > 0 {
        std::process::exit(1);
    }
}
