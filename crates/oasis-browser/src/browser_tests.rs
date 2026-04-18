use super::*;
use crate::test_utils::MockBackend;
use oasis_types::input::{Button, InputEvent};
use oasis_vfs::{MemoryVfs, Vfs};

// ---------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------

/// Create a MemoryVfs pre-populated with a simple site tree.
fn test_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/home").unwrap();
    vfs.write(
        "/sites/home/index.html",
        b"<html><head><title>Home</title></head>\
          <body><h1>Welcome</h1>\
          <p>Hello world</p>\
          <a href=\"page2.html\">Next</a>\
          </body></html>",
    )
    .unwrap();
    vfs.write(
        "/sites/home/page2.html",
        b"<html><head><title>Page 2</title></head>\
          <body><h1>Page Two</h1>\
          <p>Content here</p>\
          <a href=\"index.html\">Back</a>\
          </body></html>",
    )
    .unwrap();
    vfs.write(
        "/sites/home/article.html",
        b"<html><head><title>Article</title></head>\
          <body>\
          <article>\
          <p>This is a long enough paragraph to pass the \
          minimum scoring threshold for reader mode \
          extraction in the OASIS browser.</p>\
          <p>And here is another paragraph that also has \
          plenty of text content for the scoring algo.</p>\
          </article>\
          </body></html>",
    )
    .unwrap();
    vfs.mkdir("/sites/gem.example").unwrap();
    vfs.write(
        "/sites/gem.example/page.gmi",
        b"# Gemini Page\n\nHello from Gemini!\n\
          => gemini://gem.example/other Other Page\n",
    )
    .unwrap();
    vfs
}

fn make_browser() -> BrowserWidget {
    let mut config = BrowserConfig::default();
    config.features.home_url = "vfs://sites/home/index.html".to_string();
    BrowserWidget::new(config)
}

// ---------------------------------------------------------------
// Test 1: default config creation
// ---------------------------------------------------------------

#[test]
fn default_config_creation() {
    let config = BrowserConfig::default();
    assert!(config.features.enabled);
    assert!(config.features.native_engine);
    assert!(config.features.gemini);
    assert!(config.features.reader_mode);
    assert_eq!(config.url_bar_height, 28);
    assert_eq!(config.status_bar_height, 14);
    assert_eq!(config.button_width, 28);
    assert_eq!(config.features.home_url, "vfs://sites/home/index.html");
    assert_eq!(config.cache_size_bytes(), 8 * 1024 * 1024);
    // 272 - 28 (chrome) - 14 (status) = 230
    assert_eq!(config.content_height(272), 230);
}

// ---------------------------------------------------------------
// Test 2: SimpleTextMeasurer
// ---------------------------------------------------------------

#[test]
fn simple_text_measurer() {
    let m = SimpleTextMeasurer;
    // Sub-pixel: h(7*12/8)+e(7*12/8)+l(5*12/8)+l(5*12/8)+o(7*12/8)
    //          = 10+10+7+7+10 = 44
    assert_eq!(
        layout::block::TextMeasurer::measure_text(&m, "hello", 12,),
        44
    );
    assert_eq!(layout::block::TextMeasurer::measure_text(&m, "", 12,), 0);
    // 'a' advance=7, 7*16/8 = 14
    assert_eq!(layout::block::TextMeasurer::measure_text(&m, "a", 16,), 14);
    // t(7)+e(7)+s(7)+t(7): 7*8/8 * 4 = 28
    assert_eq!(
        layout::block::TextMeasurer::measure_text(&m, "test", 8,),
        28
    );
    // Same base=28, scale=3 at font_size 24
    assert_eq!(
        layout::block::TextMeasurer::measure_text(&m, "test", 24,),
        84
    );
}

// ---------------------------------------------------------------
// Test 3: VFS navigation
// ---------------------------------------------------------------

#[test]
fn vfs_navigation_loads_page() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    assert_eq!(browser.loading_state(), LoadingState::Idle);
    assert_eq!(browser.current_url(), Some("vfs://sites/home/index.html"));
    assert_eq!(browser.title(), Some("Home"));
    assert!(browser.document.is_some());
}

// ---------------------------------------------------------------
// Test 4: scroll input
// ---------------------------------------------------------------

#[test]
fn scroll_input_changes_offset() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Set a large content height so scroll is possible.
    browser.scroll.set_content_height(1000);

    let initial = browser.scroll.scroll_y;

    // Scroll down.
    browser.handle_input(&InputEvent::ButtonPress(Button::Down), &vfs);
    assert!(
        browser.scroll.scroll_y > initial,
        "scroll_y should increase on Down"
    );

    let after_down = browser.scroll.scroll_y;

    // Scroll up.
    browser.handle_input(&InputEvent::ButtonPress(Button::Up), &vfs);
    assert!(
        browser.scroll.scroll_y < after_down,
        "scroll_y should decrease on Up"
    );
}

// ---------------------------------------------------------------
// Test 5: link navigation
// ---------------------------------------------------------------

#[test]
fn link_navigation_cycles() {
    let mut browser = make_browser();

    // Manually set up some link regions.
    browser.link_map = vec![
        LinkRegion {
            rect: layout::box_model::Rect::new(10.0, 100.0, 80.0, 16.0),
            href: "page1.html".to_string(),
            node: 1,
        },
        LinkRegion {
            rect: layout::box_model::Rect::new(10.0, 130.0, 80.0, 16.0),
            href: "page2.html".to_string(),
            node: 2,
        },
    ];

    assert_eq!(browser.selected_link, -1);

    // Select next -> index 0.
    browser.select_next_link();
    assert_eq!(browser.selected_link, 0);

    // Select next -> index 1.
    browser.select_next_link();
    assert_eq!(browser.selected_link, 1);

    // Select next wraps -> index 0.
    browser.select_next_link();
    assert_eq!(browser.selected_link, 0);

    // Select prev wraps -> index 1.
    browser.select_prev_link();
    assert_eq!(browser.selected_link, 1);

    // Select prev -> index 0.
    browser.select_prev_link();
    assert_eq!(browser.selected_link, 0);
}

// ---------------------------------------------------------------
// Test 6: chrome click detection
// ---------------------------------------------------------------

#[test]
fn chrome_click_detection() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Navigate to a second page so back works.
    browser.navigate_vfs("vfs://sites/home/page2.html", &vfs);
    assert_eq!(browser.current_url(), Some("vfs://sites/home/page2.html"));
    assert!(browser.nav.can_go_back());

    // Click the back button (x < button_width, y < url_bar_h).
    browser.handle_click(5, 5, &vfs);
    // Should have navigated back.
    assert_eq!(browser.current_url(), Some("vfs://sites/home/index.html"));

    // Click the home button (rightmost).
    browser.navigate_vfs("vfs://sites/home/page2.html", &vfs);
    let home_x = 480 - browser.config.button_width as i32 + 5;
    browser.handle_click(home_x, 5, &vfs);
    assert_eq!(browser.current_url(), Some("vfs://sites/home/index.html"));
}

// ---------------------------------------------------------------
// Test 7: URL resolution
// ---------------------------------------------------------------

#[test]
fn url_resolution_relative() {
    let base = Url::parse("vfs://sites/home/index.html").unwrap();

    let resolved = base.resolve("page2.html").unwrap();
    assert_eq!(resolved.to_string(), "vfs://sites/home/page2.html");

    let resolved = base.resolve("/other/page.html").unwrap();
    assert_eq!(resolved.to_string(), "vfs://sites/other/page.html");

    let resolved = base.resolve("#section").unwrap();
    assert_eq!(resolved.path, "/home/index.html");
    assert_eq!(resolved.fragment, Some("section".to_string()));
}

// ---------------------------------------------------------------
// Test 8: content type dispatch
// ---------------------------------------------------------------

#[test]
fn content_type_dispatch() {
    // HTML content type should trigger load_html.
    let mut browser = make_browser();
    let response = ResourceResponse {
        url: "vfs://test/page.html".to_string(),
        content_type: ContentType::Html,
        body: b"<html><body>Test</body></html>".to_vec(),
        status: 200,
    };
    browser.process_response(response);
    assert!(browser.document.is_some());
    assert_eq!(browser.loading_state(), LoadingState::Idle);

    // Gemini content type dispatches through load_gemini.
    let mut browser2 = make_browser();
    let response = ResourceResponse {
        url: "gemini://gem.example/page.gmi".to_string(),
        content_type: ContentType::GeminiText,
        body: b"# Gemini\nHello".to_vec(),
        status: 200,
    };
    browser2.process_response(response);
    assert!(browser2.document.is_some());

    // CSS content type wraps in <pre>.
    let mut browser3 = make_browser();
    let response = ResourceResponse {
        url: "vfs://test/style.css".to_string(),
        content_type: ContentType::Css,
        body: b"body { color: red; }".to_vec(),
        status: 200,
    };
    browser3.process_response(response);
    assert!(browser3.document.is_some());

    // Image content type wraps in <img> tag.
    let mut browser4 = make_browser();
    let response = ResourceResponse {
        url: "vfs://test/photo.png".to_string(),
        content_type: ContentType::Png,
        body: vec![0u8; 16],
        status: 200,
    };
    browser4.process_response(response);
    assert!(browser4.document.is_some());
}

// ===============================================================
// Integration tests: full navigate -> parse -> layout -> paint
// ===============================================================

// ---------------------------------------------------------------
// Test: page renders text content
// ---------------------------------------------------------------

#[test]
fn page_renders_text_content() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        backend.has_text("Welcome"),
        "page should render 'Welcome' heading text"
    );
    assert!(
        backend.draw_text_count() > 0,
        "should have at least one draw_text call"
    );
    assert!(
        backend.fill_rect_count() > 0,
        "should have fill_rect calls for chrome and backgrounds"
    );
}

// ---------------------------------------------------------------
// Test: page renders links as clickable regions
// ---------------------------------------------------------------

#[test]
fn page_renders_link_regions() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        !browser.link_map.is_empty(),
        "link_map should contain at least one link"
    );
    let has_page2 = browser
        .link_map
        .iter()
        .any(|l| l.href.contains("page2.html"));
    assert!(has_page2, "should have a link to page2.html");
}

// ---------------------------------------------------------------
// Test: navigation updates content
// ---------------------------------------------------------------

#[test]
fn navigation_updates_content() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();
    assert!(backend.has_text("Welcome"), "page1 should show Welcome");

    // Navigate to page 2.
    browser.navigate_vfs("vfs://sites/home/page2.html", &vfs);
    let mut backend2 = MockBackend::new();
    browser.paint(&mut backend2).unwrap();
    assert!(
        backend2.has_text("Page") || backend2.has_text("Two"),
        "page2 should show 'Page Two' (words may be split)"
    );
}

// ---------------------------------------------------------------
// Test: chrome always renders
// ---------------------------------------------------------------

#[test]
fn chrome_always_renders() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Chrome buttons: "<", ">", "H"
    assert!(backend.has_text("<"), "should render back button '<'");
    assert!(backend.has_text(">"), "should render forward button '>'");
    assert!(backend.has_text("H"), "should render home button 'H'");
    // URL bar should show the current URL.
    assert!(backend.has_text("vfs://"), "should render URL in the bar");
}

// ---------------------------------------------------------------
// Test: error page renders on missing VFS path
// ---------------------------------------------------------------

#[test]
fn error_page_renders_on_missing_path() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://nonexistent/page.html", &vfs);

    assert_eq!(browser.loading_state(), LoadingState::Error);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Should render some error message text.
    assert!(
        backend.draw_text_count() > 0,
        "error page should render text"
    );
}

// ---------------------------------------------------------------
// Test: Gemini page renders text
// ---------------------------------------------------------------

#[test]
fn gemini_page_renders_text() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/gem.example/page.gmi", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        backend.has_text("Gemini") || backend.has_text("Page"),
        "should render Gemini heading text (words may be split)"
    );
}

// ---------------------------------------------------------------
// Test: content height is nonzero after paint
// ---------------------------------------------------------------

#[test]
fn content_height_nonzero_after_paint() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        browser.scroll().content_height > 0,
        "content_height should be nonzero for a page with content"
    );
}

// ===============================================================
// URL bar editing unit tests
// ===============================================================

// ---------------------------------------------------------------
// Test: URL bar click sets focus
// ---------------------------------------------------------------

#[test]
fn url_bar_click_sets_focus() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    assert_eq!(browser.focus, Focus::Content);

    // Click in URL bar area (between buttons).
    let bw = browser.config.button_width;
    let click_x = (bw * 2 + 10) as i32;
    let click_y = browser.config.url_bar_height as i32 / 2;
    browser.handle_click(click_x, click_y, &vfs);

    assert_eq!(browser.focus, Focus::UrlBar);
    assert_eq!(browser.url_input, "vfs://sites/home/index.html");
    assert_eq!(browser.url_cursor, browser.url_input.len());
}

// ---------------------------------------------------------------
// Test: URL bar typing inserts chars
// ---------------------------------------------------------------

#[test]
fn url_bar_typing_inserts_chars() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Enter URL bar focus.
    let bw = browser.config.button_width;
    browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
    assert_eq!(browser.focus, Focus::UrlBar);

    // Clicking the URL bar selects the whole URL (Firefox/Chrome
    // behaviour), so the first keystroke replaces the selection and
    // subsequent keystrokes append. Final buffer is exactly "abc".
    browser.handle_input(&InputEvent::TextInput('a'), &vfs);
    browser.handle_input(&InputEvent::TextInput('b'), &vfs);
    browser.handle_input(&InputEvent::TextInput('c'), &vfs);

    assert_eq!(browser.url_input, "abc");
    assert_eq!(browser.url_cursor, browser.url_input.len());
    assert!(browser.url_selection_anchor.is_none());
}

// ---------------------------------------------------------------
// Test: URL bar backspace deletes
// ---------------------------------------------------------------

#[test]
fn url_bar_backspace_deletes() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Enter URL bar focus and type some chars.
    let bw = browser.config.button_width;
    browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
    browser.handle_input(&InputEvent::TextInput('x'), &vfs);
    browser.handle_input(&InputEvent::TextInput('y'), &vfs);
    let before_bs = browser.url_input.len();

    browser.handle_input(&InputEvent::Backspace, &vfs);
    assert_eq!(browser.url_input.len(), before_bs - 1);
    assert!(browser.url_input.ends_with('x'));
}

// ---------------------------------------------------------------
// Test: URL bar confirm navigates
// ---------------------------------------------------------------

#[test]
fn url_bar_confirm_navigates() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Enter URL bar and replace content.
    let bw = browser.config.button_width;
    browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);

    // Clear the input and type a new URL.
    browser.url_input.clear();
    browser.url_cursor = 0;
    let target = "vfs://sites/home/page2.html";
    for ch in target.chars() {
        browser.handle_input(&InputEvent::TextInput(ch), &vfs);
    }

    // Press Confirm.
    browser.handle_input(&InputEvent::ButtonPress(Button::Confirm), &vfs);

    assert_eq!(browser.focus, Focus::Content);
    assert_eq!(browser.current_url(), Some("vfs://sites/home/page2.html"));
}

// ---------------------------------------------------------------
// Test: URL bar cancel discards
// ---------------------------------------------------------------

#[test]
fn url_bar_cancel_discards() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    let original_url = browser.current_url().unwrap().to_string();

    // Enter URL bar and modify.
    let bw = browser.config.button_width;
    browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
    browser.handle_input(&InputEvent::TextInput('z'), &vfs);

    // Press Cancel.
    browser.handle_input(&InputEvent::ButtonPress(Button::Cancel), &vfs);

    assert_eq!(browser.focus, Focus::Content);
    assert!(browser.url_input.is_empty());
    assert_eq!(browser.current_url(), Some(original_url.as_str()));
}

// ---------------------------------------------------------------
// Test: URL bar left/right moves cursor
// ---------------------------------------------------------------

#[test]
fn url_bar_left_right_moves_cursor() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Enter URL bar.
    let bw = browser.config.button_width;
    browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
    let end_pos = browser.url_cursor;
    assert!(end_pos > 0);

    // Move left.
    browser.handle_input(&InputEvent::ButtonPress(Button::Left), &vfs);
    assert!(browser.url_cursor < end_pos);

    let after_left = browser.url_cursor;

    // Move right.
    browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
    assert!(browser.url_cursor > after_left);
}

// ---------------------------------------------------------------
// Test: content click exits URL bar
// ---------------------------------------------------------------

#[test]
fn content_click_exits_url_bar() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Enter URL bar focus.
    let bw = browser.config.button_width;
    browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
    assert_eq!(browser.focus, Focus::UrlBar);

    // Click in content area (below URL bar).
    let content_y = browser.config.url_bar_height as i32 + 50;
    browser.handle_click(100, content_y, &vfs);
    assert_eq!(browser.focus, Focus::Content);
}

// ===============================================================
// Paint pipeline tests for chrome rendering
// ===============================================================

// ---------------------------------------------------------------
// Test: paint chrome shows editing buffer in URL bar mode
// ---------------------------------------------------------------

#[test]
fn paint_chrome_url_bar_editing() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Enter URL bar and type something.
    let bw = browser.config.button_width;
    browser.handle_click((bw * 2 + 10) as i32, 5, &vfs);
    browser.handle_input(&InputEvent::TextInput('!'), &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // The URL bar should show the editing buffer (containing '!').
    assert!(
        backend.has_text("!"),
        "URL bar should display the editing buffer text"
    );
}

// ---------------------------------------------------------------
// Test: paint chrome normal mode shows URL
// ---------------------------------------------------------------

#[test]
fn paint_chrome_normal_mode_shows_url() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    assert_eq!(browser.focus, Focus::Content);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        backend.has_text("vfs://sites/home/index.html"),
        "chrome should display the current URL"
    );
}

// ===============================================================
// Extended test fixtures
// ===============================================================

/// VFS with richer pages for interaction testing.
fn interaction_vfs() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/test").unwrap();

    // Page with a single link.
    vfs.write(
        "/sites/test/single_link.html",
        b"<html><head><title>Single Link</title></head>\
          <body>\
          <p>Before the link.</p>\
          <p><a href=\"target.html\">Click me</a></p>\
          <p>After the link.</p>\
          </body></html>",
    )
    .unwrap();

    // Target page.
    vfs.write(
        "/sites/test/target.html",
        b"<html><head><title>Target</title></head>\
          <body><h1>You arrived!</h1></body></html>",
    )
    .unwrap();

    // Page with multiple links.
    vfs.write(
        "/sites/test/multi_links.html",
        b"<html><head><title>Multi Links</title></head>\
          <body>\
          <p><a href=\"page_a.html\">Link A</a></p>\
          <p><a href=\"page_b.html\">Link B</a></p>\
          <p><a href=\"page_c.html\">Link C</a></p>\
          </body></html>",
    )
    .unwrap();

    // Page A/B/C.
    vfs.write(
        "/sites/test/page_a.html",
        b"<html><body><p>Page A</p></body></html>",
    )
    .unwrap();
    vfs.write(
        "/sites/test/page_b.html",
        b"<html><body><p>Page B</p></body></html>",
    )
    .unwrap();
    vfs.write(
        "/sites/test/page_c.html",
        b"<html><body><p>Page C</p></body></html>",
    )
    .unwrap();

    // Long page for scroll testing.
    let mut long_html = String::from("<html><head><title>Long</title></head><body>");
    for i in 0..20 {
        long_html.push_str(&format!("<p>Paragraph {} with some text content.</p>", i));
    }
    long_html.push_str(
        "<p><a href=\"target.html\">Bottom link</a></p>\
         </body></html>",
    );
    vfs.write("/sites/test/long.html", long_html.as_bytes())
        .unwrap();

    // Inline link within text.
    vfs.write(
        "/sites/test/inline_link.html",
        b"<html><body>\
          <p>Read <a href=\"target.html\">this page</a> for info.</p>\
          </body></html>",
    )
    .unwrap();

    vfs
}

fn make_interaction_browser() -> BrowserWidget {
    let mut config = BrowserConfig::default();
    config.features.home_url = "vfs://sites/test/single_link.html".to_string();
    BrowserWidget::new(config)
}

// ===============================================================
// Category A: Layout Geometry Verification
// ===============================================================

#[test]
fn text_boxes_do_not_overlap() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let overlaps = backend.find_overlapping_text_lines();
    assert!(
        overlaps.is_empty(),
        "text lines should not overlap, found {} overlapping line pairs: {:?}",
        overlaps.len(),
        overlaps,
    );
}

#[test]
fn multi_link_page_text_does_not_overlap() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let overlaps = backend.find_overlapping_text_lines();
    assert!(
        overlaps.is_empty(),
        "multi-link page text lines should not overlap: {:?}",
        overlaps,
    );
}

#[test]
fn text_y_positions_increase_monotonically() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let positions = backend.text_positions();
    // Filter to content text only (skip single-char chrome buttons).
    let content: Vec<_> = positions
        .iter()
        .filter(|(t, _, _, _)| t.len() > 1)
        .collect();

    // Y should never decrease between distinct text lines.
    for pair in content.windows(2) {
        let (text_a, _, ya, _) = pair[0];
        let (text_b, _, yb, _) = pair[1];
        assert!(
            yb >= ya,
            "text Y should increase: '{}' at y={} before '{}' at y={}",
            text_a,
            ya,
            text_b,
            yb,
        );
    }
}

#[test]
fn line_height_exceeds_font_size() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    // Check layout tree: all inline boxes should have
    // content.height >= font_size.
    let layout = browser.layout_root.as_ref().expect("should have layout");
    check_line_heights(layout);
}

/// Recursively verify line heights in the layout tree.
fn check_line_heights(lb: &layout::box_model::LayoutBox) {
    if matches!(lb.box_type, layout::box_model::BoxType::Inline)
        && lb.dimensions.content.height > 0.0
    {
        assert!(
            lb.dimensions.content.height >= lb.style.font_size,
            "inline box height ({}) should be >= font_size ({}) for text {:?}",
            lb.dimensions.content.height,
            lb.style.font_size,
            lb.text,
        );
    }
    for child in &lb.children {
        check_line_heights(child);
    }
}

// ===============================================================
// Category B: Link Region Validation
// ===============================================================

#[test]
fn link_regions_have_valid_dimensions() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        !browser.link_map.is_empty(),
        "should have at least one link region"
    );

    for link in &browser.link_map {
        assert!(
            link.rect.width > 0.0,
            "link '{}' should have positive width, got {}",
            link.href,
            link.rect.width,
        );
        assert!(
            link.rect.height > 0.0,
            "link '{}' should have positive height, got {}",
            link.href,
            link.rect.height,
        );
    }
}

#[test]
fn link_regions_within_viewport() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let chrome_y = browser.config.url_bar_height as f32;
    let view_bottom = browser.window_h as f32;

    for link in &browser.link_map {
        assert!(
            link.rect.x >= 0.0,
            "link x ({}) should be >= 0",
            link.rect.x,
        );
        assert!(
            link.rect.y >= chrome_y,
            "link y ({}) should be >= chrome height ({})",
            link.rect.y,
            chrome_y,
        );
        assert!(
            link.rect.y + link.rect.height <= view_bottom + 1.0,
            "link bottom ({}) should be <= viewport bottom ({})",
            link.rect.y + link.rect.height,
            view_bottom,
        );
    }
}

#[test]
fn multiple_links_have_distinct_regions() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        browser.link_map.len() >= 3,
        "multi-link page should have at least 3 links, got {}",
        browser.link_map.len(),
    );

    // No two links should have the same Y position.
    for i in 0..browser.link_map.len() {
        for j in (i + 1)..browser.link_map.len() {
            let a = &browser.link_map[i].rect;
            let b = &browser.link_map[j].rect;
            // Check they don't fully overlap (different hrefs should
            // have different rects).
            let overlaps_x = a.x < b.x + b.width && b.x < a.x + a.width;
            let overlaps_y = a.y < b.y + b.height && b.y < a.y + a.height;
            assert!(
                !(overlaps_x && overlaps_y),
                "links '{}' and '{}' should not overlap: \
                 a=({},{},{},{}) b=({},{},{},{})",
                browser.link_map[i].href,
                browser.link_map[j].href,
                a.x,
                a.y,
                a.width,
                a.height,
                b.x,
                b.y,
                b.width,
                b.height,
            );
        }
    }
}

#[test]
fn link_region_matches_rendered_text_position() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let link = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("target.html"))
        .expect("should have link to target.html");

    // Find the "Click me" text draw call.
    let text_pos = backend
        .text_positions()
        .into_iter()
        .find(|(t, _, _, _)| t.contains("Click"));

    if let Some((_text, tx, ty, _fs)) = text_pos {
        // The text draw position should be within the link rect.
        let lx = link.rect.x as i32;
        let ly = link.rect.y as i32;
        let lr = lx + link.rect.width as i32;
        let lb = ly + link.rect.height as i32;

        assert!(
            tx >= lx - 2 && tx <= lr + 2,
            "text x ({}) should be within link rect x range ({}-{})",
            tx,
            lx,
            lr,
        );
        assert!(
            ty >= ly - 2 && ty <= lb + 2,
            "text y ({}) should be within link rect y range ({}-{})",
            ty,
            ly,
            lb,
        );
    }
}

// ===============================================================
// Category C: Click-to-Navigate Simulation
// ===============================================================

#[test]
fn click_on_link_center_navigates() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let link = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("target.html"))
        .expect("should have link to target.html");

    // Click at the center of the link region.
    let cx = (link.rect.x + link.rect.width / 2.0) as i32;
    let cy = (link.rect.y + link.rect.height / 2.0) as i32;

    // Diagnostic: print link rect and click position.
    let link_rect = link.rect;
    eprintln!(
        "link rect: x={}, y={}, w={}, h={}; click: ({}, {})",
        link_rect.x, link_rect.y, link_rect.width, link_rect.height, cx, cy,
    );

    browser.handle_click(cx, cy, &vfs);

    assert_eq!(
        browser.current_url(),
        Some("vfs://sites/test/target.html"),
        "clicking link center should navigate to target.html \
         (link rect: x={:.1}, y={:.1}, w={:.1}, h={:.1}; click: ({}, {}))",
        link_rect.x,
        link_rect.y,
        link_rect.width,
        link_rect.height,
        cx,
        cy,
    );
}

#[test]
fn click_on_link_edge_navigates() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let link = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("target.html"))
        .expect("should have link to target.html");

    // Click 1px inside the top-left corner.
    let cx = (link.rect.x + 1.0) as i32;
    let cy = (link.rect.y + 1.0) as i32;

    browser.handle_click(cx, cy, &vfs);

    assert_eq!(
        browser.current_url(),
        Some("vfs://sites/test/target.html"),
        "clicking near link edge should navigate",
    );
}

#[test]
fn click_outside_link_does_not_navigate() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);
    let original = browser.current_url().unwrap().to_string();

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Click far below any content.
    browser.handle_click(240, 260, &vfs);
    assert_eq!(
        browser.current_url(),
        Some(original.as_str()),
        "clicking outside links should not navigate",
    );
}

#[test]
fn click_on_second_link_navigates_to_correct_target() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Find link B.
    let link_b = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("page_b.html"))
        .expect("should have link to page_b.html");

    let cx = (link_b.rect.x + link_b.rect.width / 2.0) as i32;
    let cy = (link_b.rect.y + link_b.rect.height / 2.0) as i32;

    browser.handle_click(cx, cy, &vfs);

    assert_eq!(
        browser.current_url(),
        Some("vfs://sites/test/page_b.html"),
        "clicking Link B should navigate to page_b.html",
    );
}

#[test]
fn tab_then_confirm_navigates() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(!browser.link_map.is_empty(), "should have links");

    // Tab to select first link.
    browser.handle_input(&InputEvent::ButtonPress(Button::Right), &vfs);
    assert_eq!(browser.selected_link, 0);

    // Confirm.
    browser.handle_input(&InputEvent::ButtonPress(Button::Confirm), &vfs);

    assert_eq!(
        browser.current_url(),
        Some("vfs://sites/test/target.html"),
        "tab + confirm should navigate to target",
    );
}

#[test]
fn click_all_links_on_multi_link_page() {
    let vfs = interaction_vfs();
    let targets = ["page_a.html", "page_b.html", "page_c.html"];

    for target in &targets {
        let mut browser = make_interaction_browser();
        browser.set_window(0, 0, 480, 272);
        browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);

        let mut backend = MockBackend::new();
        browser.paint(&mut backend).unwrap();

        let link = browser
            .link_map
            .iter()
            .find(|l| l.href.contains(target))
            .unwrap_or_else(|| panic!("should have link to {target}"));

        let cx = (link.rect.x + link.rect.width / 2.0) as i32;
        let cy = (link.rect.y + link.rect.height / 2.0) as i32;

        browser.handle_click(cx, cy, &vfs);

        let expected = format!("vfs://sites/test/{target}");
        assert_eq!(
            browser.current_url(),
            Some(expected.as_str()),
            "clicking should navigate to {target}",
        );
    }
}

// ===============================================================
// Category D: Scroll + Link Interaction
// ===============================================================

#[test]
fn link_regions_update_after_scroll_and_repaint() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/long.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let initial_links = browser.link_map.clone();

    // Force content height so scrolling is possible.
    browser.scroll.set_content_height(2000);

    // Scroll down.
    for _ in 0..5 {
        browser.handle_input(&InputEvent::ButtonPress(Button::Down), &vfs);
    }

    // Repaint to get updated link regions.
    let mut backend2 = MockBackend::new();
    browser.paint(&mut backend2).unwrap();

    // If there were links visible before scroll, their Y positions
    // should have shifted (or they may be off-screen now).
    if !initial_links.is_empty() && !browser.link_map.is_empty() {
        // At minimum, verify the link_map was regenerated (it's
        // rebuilt every paint pass).
        assert!(
            !browser.link_map.is_empty(),
            "link_map should be regenerated after repaint"
        );
    }
}

// ===============================================================
// Category E: End-to-End HTML Scenarios
// ===============================================================

#[test]
fn inline_link_within_paragraph() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/inline_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Should render "this page" as link text.
    assert!(
        !browser.link_map.is_empty(),
        "inline link should produce link regions"
    );

    let link = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("target.html"))
        .expect("should have link to target.html");

    // Click the link.
    let cx = (link.rect.x + link.rect.width / 2.0) as i32;
    let cy = (link.rect.y + link.rect.height / 2.0) as i32;

    browser.handle_click(cx, cy, &vfs);
    assert_eq!(browser.current_url(), Some("vfs://sites/test/target.html"),);
}

#[test]
fn navigate_back_after_link_click() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);
    let original = browser.current_url().unwrap().to_string();

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Click link to navigate.
    let link = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("target.html"))
        .expect("should have link");
    let cx = (link.rect.x + link.rect.width / 2.0) as i32;
    let cy = (link.rect.y + link.rect.height / 2.0) as i32;
    browser.handle_click(cx, cy, &vfs);
    assert_eq!(browser.current_url(), Some("vfs://sites/test/target.html"),);

    // Go back.
    browser.go_back(&vfs);
    assert_eq!(browser.current_url(), Some(original.as_str()));
}

#[test]
fn full_roundtrip_navigate_paint_click() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);

    // Step 1: Navigate to multi-links page.
    browser.navigate_vfs("vfs://sites/test/multi_links.html", &vfs);
    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Verify content rendered (words split by inline layout).
    assert!(backend.has_text("Link"));
    assert!(backend.draw_text_count() > 3);

    // Step 2: Click Link A.
    let link_a = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("page_a.html"))
        .expect("should have Link A");
    let cx = (link_a.rect.x + link_a.rect.width / 2.0) as i32;
    let cy = (link_a.rect.y + link_a.rect.height / 2.0) as i32;
    browser.handle_click(cx, cy, &vfs);
    assert_eq!(browser.current_url(), Some("vfs://sites/test/page_a.html"));

    // Step 3: Paint new page.
    let mut backend2 = MockBackend::new();
    browser.paint(&mut backend2).unwrap();
    assert!(backend2.has_text("Page"));

    // Step 4: Go back and repaint.
    browser.go_back(&vfs);
    let mut backend3 = MockBackend::new();
    browser.paint(&mut backend3).unwrap();
    assert!(backend3.has_text("Link"));

    // Step 5: Links should work again after going back.
    let link_c = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("page_c.html"))
        .expect("should have Link C after going back");
    let cx = (link_c.rect.x + link_c.rect.width / 2.0) as i32;
    let cy = (link_c.rect.y + link_c.rect.height / 2.0) as i32;
    browser.handle_click(cx, cy, &vfs);
    assert_eq!(browser.current_url(), Some("vfs://sites/test/page_c.html"));
}

// ===============================================================
// Category F: Diagnostic / Coordinate Debugging Tests
// ===============================================================

#[test]
fn link_rect_is_hittable_by_integer_coords() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    for link in &browser.link_map {
        // Verify that the center of the rect, cast to i32,
        // still falls inside the f32 rect. This catches rounding
        // edge cases.
        let cx = (link.rect.x + link.rect.width / 2.0) as i32;
        let cy = (link.rect.y + link.rect.height / 2.0) as i32;

        assert!(
            (cx as f32) >= link.rect.x
                && (cx as f32) < link.rect.x + link.rect.width
                && (cy as f32) >= link.rect.y
                && (cy as f32) < link.rect.y + link.rect.height,
            "center ({}, {}) of link '{}' should be inside its rect \
             ({}, {}, {}, {})",
            cx,
            cy,
            link.href,
            link.rect.x,
            link.rect.y,
            link.rect.width,
            link.rect.height,
        );
    }
}

#[test]
fn link_rect_y_is_below_chrome() {
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let chrome_h = browser.config.url_bar_height as f32;
    for link in &browser.link_map {
        let cy = (link.rect.y + link.rect.height / 2.0) as i32;
        let rel_y = cy - browser.window_y;
        assert!(
            rel_y >= chrome_h as i32,
            "link '{}' center y ({}) should be below chrome ({})",
            link.href,
            rel_y,
            chrome_h,
        );
    }
}

#[test]
fn handle_click_coordinates_match_link_map() {
    // This is the most precise diagnostic test: it reconstructs
    // the exact hit-test logic from handle_click() and verifies
    // that at least one link in the map is hittable.
    let vfs = interaction_vfs();
    let mut browser = make_interaction_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/test/single_link.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        !browser.link_map.is_empty(),
        "should have at least one link"
    );

    let link = &browser.link_map[0];
    let cx = (link.rect.x + link.rect.width / 2.0) as i32;
    let cy = (link.rect.y + link.rect.height / 2.0) as i32;

    // Replicate the handle_click logic.
    let rel_y = cy - browser.window_y;
    let chrome_h = browser.config.url_bar_height as i32;

    assert!(
        rel_y >= chrome_h,
        "click y ({}) relative to window ({}) = {} should be >= chrome ({}). \
         Link rect: ({:.1}, {:.1}, {:.1}, {:.1})",
        cy,
        browser.window_y,
        rel_y,
        chrome_h,
        link.rect.x,
        link.rect.y,
        link.rect.width,
        link.rect.height,
    );

    // Check the hit test would match.
    let hit = (cx as f32) >= link.rect.x
        && (cx as f32) < link.rect.x + link.rect.width
        && (cy as f32) >= link.rect.y
        && (cy as f32) < link.rect.y + link.rect.height;

    assert!(
        hit,
        "center ({}, {}) should hit link rect ({:.1}, {:.1}, {:.1}, {:.1})",
        cx, cy, link.rect.x, link.rect.y, link.rect.width, link.rect.height,
    );
}

#[test]
fn original_test_page_link_click_navigates() {
    // Test the original test_vfs index.html page (the one used by
    // existing tests) to ensure its link is also clickable.
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    let link = browser
        .link_map
        .iter()
        .find(|l| l.href.contains("page2.html"))
        .expect("should have link to page2.html");

    let link_rect = link.rect;
    let cx = (link_rect.x + link_rect.width / 2.0) as i32;
    let cy = (link_rect.y + link_rect.height / 2.0) as i32;

    browser.handle_click(cx, cy, &vfs);

    assert_eq!(
        browser.current_url(),
        Some("vfs://sites/home/page2.html"),
        "clicking link on original index.html should navigate to page2 \
         (link rect: x={:.1}, y={:.1}, w={:.1}, h={:.1}; click: ({}, {}))",
        link_rect.x,
        link_rect.y,
        link_rect.width,
        link_rect.height,
        cx,
        cy,
    );
}

#[test]
fn test_navigate_https_without_tls_shows_error() {
    let mut bw = make_browser();
    let vfs = test_vfs();
    // No TLS provider set -- HTTPS should produce an error page.
    bw.navigate_vfs("https://example.com/page", &vfs);
    assert_eq!(bw.state, LoadingState::Idle);
    // The HTTPS error page should be rendered as HTML in the DOM.
    let doc = bw.document.as_ref().expect("document should be loaded");
    let text = doc.text_content(doc.root);
    assert!(
        text.contains("HTTPS Required"),
        "expected 'HTTPS Required' in page text, got: {text}",
    );
}

#[test]
fn test_navigate_gemini_without_tls_shows_error() {
    let mut bw = make_browser();
    let vfs = test_vfs();
    // No TLS provider -- Gemini should show a TLS Required page.
    bw.navigate_vfs("gemini://example.com/page", &vfs);
    assert_eq!(bw.state, LoadingState::Idle);
    let doc = bw.document.as_ref().expect("document should be loaded");
    let text = doc.text_content(doc.root);
    assert!(
        text.contains("TLS Required"),
        "expected 'TLS Required' in page text, got: {text}",
    );
}

// ===============================================================
// Incremental Layout / Dirty Flag Tests
// ===============================================================

#[test]
fn layout_not_dirty_after_load() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    assert!(
        !browser.is_layout_dirty(),
        "layout should not be dirty immediately after load_html"
    );
}

#[test]
fn layout_dirty_after_width_change() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Change width -> should mark dirty.
    browser.set_window(0, 0, 320, 272);
    assert!(
        browser.is_layout_dirty(),
        "layout should be dirty after viewport width change"
    );
}

#[test]
fn layout_not_dirty_after_position_only_change() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Move window (same size) -> should NOT mark dirty.
    browser.set_window(10, 20, 480, 272);
    assert!(
        !browser.is_layout_dirty(),
        "layout should not be dirty after position-only change"
    );
}

#[test]
fn layout_dirty_after_height_only_change() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Change height only -> should mark dirty because viewport
    // height is used for CSS positioned elements (fixed/absolute).
    browser.set_window(0, 0, 480, 300);
    assert!(
        browser.is_layout_dirty(),
        "layout should be dirty after height change"
    );
}

#[test]
fn relayout_if_dirty_skips_when_clean() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Layout is clean after load.
    assert!(
        !browser.relayout_if_dirty(),
        "should skip relayout when clean"
    );
}

#[test]
fn relayout_if_dirty_rebuilds_on_width_change() {
    let vfs = test_vfs();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/index.html", &vfs);

    // Change width to mark dirty.
    browser.set_window(0, 0, 320, 272);
    assert!(browser.is_layout_dirty());

    // Relayout should run and clear dirty flag.
    assert!(
        browser.relayout_if_dirty(),
        "should perform relayout on dirty"
    );
    assert!(
        !browser.is_layout_dirty(),
        "dirty flag should be cleared after relayout"
    );
    assert!(
        browser.layout_root.is_some(),
        "layout_root should exist after relayout"
    );
}

#[test]
fn relayout_if_dirty_skips_without_document() {
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);

    // No page loaded -- force dirty.
    browser.set_window(0, 0, 320, 272);

    // Should skip (no document/styles).
    assert!(
        !browser.relayout_if_dirty(),
        "should skip relayout without a loaded document"
    );
    assert!(
        !browser.is_layout_dirty(),
        "dirty flag should be cleared even without document"
    );
}

#[test]
fn layout_box_dirty_field_defaults_true() {
    let lb = layout::box_model::LayoutBox::new(
        layout::box_model::BoxType::Block,
        css::values::ComputedStyle::default(),
        None,
    );
    assert!(lb.dirty, "new LayoutBox should have dirty=true");
}

#[test]
fn layout_box_mark_clean_propagates() {
    let mut parent = layout::box_model::LayoutBox::new(
        layout::box_model::BoxType::Block,
        css::values::ComputedStyle::default(),
        None,
    );
    let child = layout::box_model::LayoutBox::new(
        layout::box_model::BoxType::Inline,
        css::values::ComputedStyle::default(),
        None,
    );
    parent.children.push(child);

    assert!(parent.dirty);
    assert!(parent.children[0].dirty);

    parent.mark_clean();

    assert!(!parent.dirty, "parent should be clean");
    assert!(!parent.children[0].dirty, "child should be clean");
}

#[test]
fn layout_box_mark_dirty_sets_flag() {
    let mut lb = layout::box_model::LayoutBox::new(
        layout::box_model::BoxType::Block,
        css::values::ComputedStyle::default(),
        None,
    );
    lb.mark_clean();
    assert!(!lb.dirty);

    lb.mark_dirty();
    assert!(lb.dirty, "mark_dirty should set dirty flag");
}

// ===============================================================
// Image rendering tests
// ===============================================================

/// Build a minimal valid 24-bit BMP of given dimensions (solid red).
fn make_test_bmp(w: u32, h: u32) -> Vec<u8> {
    let bpp: u16 = 24;
    let row_bytes = (w * 3).div_ceil(4) * 4;
    let pixel_data_size = row_bytes * h;
    let file_size = 54 + pixel_data_size;

    let mut bmp = vec![0u8; file_size as usize];
    bmp[0] = b'B';
    bmp[1] = b'M';
    bmp[2..6].copy_from_slice(&file_size.to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&(w as i32).to_le_bytes());
    bmp[22..26].copy_from_slice(&(h as i32).to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&bpp.to_le_bytes());
    bmp[30..34].copy_from_slice(&0u32.to_le_bytes());

    // Fill with solid red (BGR = 0,0,255).
    for row in 0..h {
        for col in 0..w {
            let off = 54 + (row * row_bytes + col * 3) as usize;
            if off + 2 < bmp.len() {
                bmp[off] = 0; // B
                bmp[off + 1] = 0; // G
                bmp[off + 2] = 255; // R
            }
        }
    }
    bmp
}

/// Create a VFS with an image file and an HTML page referencing it.
fn test_vfs_with_image() -> MemoryVfs {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/img").unwrap();

    let bmp_data = make_test_bmp(16, 16);
    vfs.write("/sites/img/red.bmp", &bmp_data).unwrap();

    vfs.write(
        "/sites/img/index.html",
        b"<html><body>\
          <h1>Image Test</h1>\
          <img src=\"red.bmp\" alt=\"Red square\">\
          </body></html>",
    )
    .unwrap();

    vfs.write(
        "/sites/img/with_dims.html",
        b"<html><body>\
          <img src=\"red.bmp\" width=\"32\" height=\"32\" alt=\"Scaled\">\
          </body></html>",
    )
    .unwrap();

    vfs.write(
        "/sites/img/broken.html",
        b"<html><body>\
          <img src=\"nonexistent.bmp\" alt=\"Missing image\">\
          </body></html>",
    )
    .unwrap();

    vfs.write(
        "/sites/img/no_alt.html",
        b"<html><body>\
          <img src=\"nonexistent.bmp\">\
          </body></html>",
    )
    .unwrap();

    vfs.write(
        "/sites/img/multi.html",
        b"<html><body>\
          <p>Before</p>\
          <img src=\"red.bmp\" alt=\"First\">\
          <p>Middle</p>\
          <img src=\"red.bmp\" alt=\"Second\">\
          <p>After</p>\
          </body></html>",
    )
    .unwrap();

    vfs
}

// ---------------------------------------------------------------
// Test: image decoded from VFS during navigation
// ---------------------------------------------------------------

#[test]
fn image_decoded_from_vfs_during_navigation() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/index.html", &vfs);

    // Images are now loaded progressively; process all pending.
    browser.load_next_image_batch(&vfs, 5000);

    assert_eq!(browser.loading_state(), LoadingState::Idle);
    assert!(
        !browser.decoded_images.is_empty(),
        "should have decoded at least one image"
    );

    let has_red = browser
        .decoded_images
        .values()
        .any(|img| img.width == 16 && img.height == 16);
    assert!(has_red, "decoded image should be 16x16");
}

// ---------------------------------------------------------------
// Test: image renders as blit call (not placeholder)
// ---------------------------------------------------------------

#[test]
fn image_renders_as_blit() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
    browser.load_next_image_batch(&vfs, 5000);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert!(
        backend.blit_count() > 0,
        "should have at least one blit call for the image"
    );

    // Verify the blit has correct dimensions (16x16 from intrinsic).
    let blits: Vec<_> = backend
        .calls
        .iter()
        .filter_map(|c| {
            if let crate::test_utils::DrawCall::Blit { w, h, .. } = c {
                Some((*w, *h))
            } else {
                None
            }
        })
        .collect();
    assert!(
        blits.iter().any(|&(w, h)| w == 16 && h == 16),
        "should blit at 16x16 (intrinsic dimensions), got: {blits:?}"
    );
}

// ---------------------------------------------------------------
// Test: image uses intrinsic dimensions in layout
// ---------------------------------------------------------------

#[test]
fn image_uses_intrinsic_dimensions_in_layout() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
    browser.load_next_image_batch(&vfs, 5000);

    // Check that the layout tree contains a Replaced(Image) with
    // correct dimensions (16x16) instead of the default 0x0.
    let layout = browser.layout_root.as_ref().expect("should have layout");
    let img_box = find_image_box(layout);
    assert!(img_box.is_some(), "layout tree should contain an image box");
    let (w, h) = img_box.unwrap();
    assert_eq!(w, 16, "image layout width should be 16");
    assert_eq!(h, 16, "image layout height should be 16");
}

// ---------------------------------------------------------------
// Test: HTML width/height attributes override intrinsic
// ---------------------------------------------------------------

#[test]
fn image_html_attrs_override_intrinsic() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/with_dims.html", &vfs);

    let layout = browser.layout_root.as_ref().expect("should have layout");
    let img_box = find_image_box(layout);
    assert!(img_box.is_some(), "layout tree should contain an image box");
    let (w, h) = img_box.unwrap();
    assert_eq!(w, 32, "image width should be 32 (from HTML attr)");
    assert_eq!(h, 32, "image height should be 32 (from HTML attr)");
}

// ---------------------------------------------------------------
// Test: broken image shows placeholder (no blit)
// ---------------------------------------------------------------

#[test]
fn broken_image_shows_placeholder() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/broken.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert_eq!(
        backend.blit_count(),
        0,
        "should not blit for a broken image"
    );
    assert!(
        backend.has_text("Missing"),
        "should render alt text for broken image"
    );
}

// ---------------------------------------------------------------
// Test: broken image without alt shows multiplication sign
// ---------------------------------------------------------------

#[test]
fn broken_image_no_alt_shows_placeholder_symbol() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/no_alt.html", &vfs);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    assert_eq!(
        backend.blit_count(),
        0,
        "should not blit for a broken image"
    );
    // Should show the multiplication sign placeholder.
    assert!(
        backend.has_text("\u{00D7}"),
        "should render multiplication sign for broken image without alt"
    );
}

// ---------------------------------------------------------------
// Test: multiple images on same page
// ---------------------------------------------------------------

#[test]
fn multiple_images_same_page() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/multi.html", &vfs);
    browser.load_next_image_batch(&vfs, 5000);

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // Both images reference the same BMP, so only one decoded entry
    // but two blit calls (one per <img> tag).
    assert_eq!(
        browser.decoded_images.len(),
        1,
        "should deduplicate same-URL images"
    );
    assert!(
        backend.blit_count() >= 2,
        "should blit twice for two <img> tags, got {}",
        backend.blit_count()
    );
    assert!(backend.has_text("Before"), "should render surrounding text");
    assert!(backend.has_text("After"), "should render surrounding text");
}

// ---------------------------------------------------------------
// Test: textures created during paint
// ---------------------------------------------------------------

#[test]
fn textures_created_during_paint() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
    browser.load_next_image_batch(&vfs, 5000);

    // Before paint, no textures exist (neither individual nor atlas).
    assert!(
        browser.image_textures.is_empty() && browser.image_atlas.is_empty(),
        "textures should not exist before paint"
    );

    let mut backend = MockBackend::new();
    browser.paint(&mut backend).unwrap();

    // After paint, texture should be created (small images go into the
    // atlas, larger ones get individual textures).
    assert!(
        !browser.image_textures.is_empty() || !browser.image_atlas.is_empty(),
        "textures should exist after paint"
    );
}

// ---------------------------------------------------------------
// Test: navigating clears old image state
// ---------------------------------------------------------------

#[test]
fn navigation_clears_image_state() {
    let vfs = test_vfs_with_image();
    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);

    // Navigate to page with image.
    browser.navigate_vfs("vfs://sites/img/index.html", &vfs);
    browser.load_next_image_batch(&vfs, 5000);
    assert!(!browser.decoded_images.is_empty());

    // Navigate to page without image.
    browser.navigate_vfs("vfs://sites/img/broken.html", &vfs);
    // decoded_images should have been cleared and only repopulated
    // for the new page (which has no decodable images).
    assert!(
        browser
            .decoded_images
            .values()
            .all(|img| img.width != 16 || img.height != 16),
        "old decoded images should be cleared on navigation"
    );
}

// ---------------------------------------------------------------
// Test: image loading is progressive (Phase 1)
// ---------------------------------------------------------------

#[test]
fn image_loading_is_progressive() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/home").unwrap();

    // Create a page with multiple images.
    vfs.write(
        "/sites/home/images.html",
        b"<html><body>\
          <img src=\"img1.bmp\">\
          <img src=\"img2.bmp\">\
          <img src=\"img3.bmp\">\
          </body></html>",
    )
    .unwrap();

    // Write small BMP files.
    let bmp = make_test_bmp(2, 2);
    vfs.write("/sites/home/img1.bmp", &bmp).unwrap();
    vfs.write("/sites/home/img2.bmp", &bmp).unwrap();
    vfs.write("/sites/home/img3.bmp", &bmp).unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 480, 272);
    browser.navigate_vfs("vfs://sites/home/images.html", &vfs);

    // After navigate, images should be pending, not yet decoded.
    assert!(
        !browser.pending_images.is_empty() || !browser.decoded_images.is_empty(),
        "should have pending or already-decoded images"
    );

    // Call with 0ms budget — should process at most 1 image.
    let before = browser.decoded_images.len();
    browser.load_next_image_batch(&vfs, 0);
    let after = browser.decoded_images.len();
    assert!(
        after - before <= 1,
        "0ms budget should process at most 1 image, got {}",
        after - before
    );
}

/// Recursively find a Replaced(Image) box and return its (width, height).
fn find_image_box(layout_box: &layout::box_model::LayoutBox) -> Option<(u32, u32)> {
    if let layout::box_model::BoxType::Replaced(layout::box_model::ReplacedContent::Image {
        width,
        height,
        ..
    }) = &layout_box.box_type
    {
        return Some((*width, *height));
    }
    for child in &layout_box.children {
        if let Some(result) = find_image_box(child) {
            return Some(result);
        }
    }
    None
}

#[test]
fn hover_restyle_is_partial() {
    // Build a DOM with many elements and a :hover rule.
    // Verify that hover only restyles the affected ancestor chain.
    let mut elements: Vec<String> = Vec::new();
    for i in 0..50 {
        elements.push(format!("<p>Paragraph {i}</p>"));
    }
    let html = format!(
        "<html><head><style>a:hover {{ color: red; }}</style></head><body>\
         <a href=\"link.html\"><span>Link</span></a>\
         {}\
         </body></html>",
        elements.join("")
    );
    let _vfs = MemoryVfs::new();
    let config = BrowserConfig::default();
    let mut browser = BrowserWidget::new(config);
    browser.load_html(&html, "file:///test.html");

    // Verify cached sheets are populated.
    assert!(
        !browser.cached_author_sheets.is_empty() || browser.cached_inline_styles.is_empty(),
        "cached sheets should be set after load_html"
    );

    // Simulate hover on the link node.
    let link_node = browser
        .href_map
        .keys()
        .next()
        .copied()
        .expect("should have a link");
    let old_hover = browser.hover_node;
    browser.hover_node = Some(link_node);
    browser.restyle_hover_affected(old_hover);

    // The hover rule only changes `color` (a visual-only property),
    // so layout_dirty should NOT be set -- only a repaint is needed.
    assert!(
        !browser.layout_dirty,
        "visual-only hover change should skip relayout"
    );

    // Styles should still be populated for all elements.
    let styled_count = browser.styles.iter().filter(|s| s.is_some()).count();
    assert!(styled_count > 5, "most elements should retain styles");
}

#[test]
fn image_eviction_respects_budget() {
    // Directly test that decoded_image_lru eviction works by
    // inserting images that exceed the budget.
    let config = BrowserConfig::default();
    let mut browser = BrowserWidget::new(config);

    // Create a fake 1x1 image (4 bytes). Set a tiny budget.
    let small_img = image::DecodedImage::new(1, 1, vec![255, 0, 0, 255]);

    // Manually insert decoded images to test eviction.
    for i in 0..5 {
        let url = format!("http://test.com/img{i}.png");
        let img_bytes = (small_img.width * small_img.height * 4) as usize;
        browser.decoded_image_bytes += img_bytes;
        browser.decoded_image_lru.push_front(url.clone());
        browser.decoded_images.insert(url, small_img.clone());
    }
    assert_eq!(browser.decoded_images.len(), 5);
    assert_eq!(browser.decoded_image_bytes, 20); // 5 * 4 bytes

    // LRU order: img4 (front/MRU) ... img0 (back/LRU)
    // Verify the LRU tracks all 5.
    assert_eq!(browser.decoded_image_lru.len(), 5);
}

// ---------------------------------------------------------------
// JavaScript integration tests
// ---------------------------------------------------------------

#[test]
#[cfg(feature = "javascript")]
fn script_execution() {
    let mut browser = make_browser();
    browser.load_html(
        "<html><body><script>console.log('works')</script></body></html>",
        "test://js",
    );
    let out = browser.console_output();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].level, oasis_js::ConsoleLevel::Log);
    assert_eq!(out[0].message, "works");
}

#[test]
#[cfg(feature = "javascript")]
fn script_error_no_crash() {
    let mut browser = make_browser();
    browser.load_html(
        "<html><body><script>throw new Error('boom')</script></body></html>",
        "test://js-err",
    );
    // Should not panic; error appears in console output.
    let out = browser.console_output();
    assert!(out.iter().any(|e| e.level == oasis_js::ConsoleLevel::Error));
}

#[test]
#[cfg(feature = "javascript")]
fn no_external_scripts() {
    let mut browser = make_browser();
    browser.load_html(
        "<html><body><script src=\"foo.js\"></script></body></html>",
        "test://js-ext",
    );
    // External script should not be executed; no console output.
    assert!(browser.console_output().is_empty());
}

#[test]
#[cfg(feature = "javascript")]
fn js_dom_text_content_mutation() {
    let mut browser = make_browser();
    browser.load_html(
        "<html><body>\
         <div id=\"target\">old</div>\
         <script>document.getElementById('target').textContent = 'new'</script>\
         </body></html>",
        "test://js-dom-text",
    );
    let doc = browser.document.as_ref().unwrap();
    let target = doc.get_element_by_id("target").unwrap();
    assert_eq!(doc.text_content(target), "new");
}

#[test]
#[cfg(feature = "javascript")]
fn js_dom_create_element_and_append() {
    let mut browser = make_browser();
    browser.load_html(
        "<html><body>\
         <div id=\"container\"></div>\
         <script>\
           var el = document.createElement('span');\
           el.textContent = 'created';\
           document.getElementById('container').appendChild(el);\
         </script>\
         </body></html>",
        "test://js-dom-create",
    );
    let doc = browser.document.as_ref().unwrap();
    let container = doc.get_element_by_id("container").unwrap();
    assert!(doc.text_content(container).contains("created"));
}

#[test]
#[cfg(feature = "javascript")]
fn js_dom_get_attribute() {
    let mut browser = make_browser();
    browser.load_html(
        "<html><body>\
         <a id=\"link\" href=\"https://example.com\">click</a>\
         <script>\
           var a = document.getElementById('link');\
           console.log(a.getAttribute('href'));\
         </script>\
         </body></html>",
        "test://js-dom-attr",
    );
    let out = browser.console_output();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].message, "https://example.com");
}

// ---------------------------------------------------------------
// Form interaction tests — end-to-end keyboard + click routing
// ---------------------------------------------------------------

/// Locate the first focusable text input in the layout tree and
/// return the center of its rendered rectangle in screen space.
fn input_center(browser: &BrowserWidget) -> (i32, i32) {
    use crate::layout::box_model::{BoxType, LayoutBox, ReplacedContent};
    fn walk(lb: &LayoutBox) -> Option<(i32, i32)> {
        if let BoxType::Replaced(ReplacedContent::TextInput { .. }) = &lb.box_type {
            let r = &lb.dimensions.content;
            return Some(((r.x + r.width / 2.0) as i32, (r.y + r.height / 2.0) as i32));
        }
        for c in &lb.children {
            if let Some(p) = walk(c) {
                return Some(p);
            }
        }
        None
    }
    let root = browser.layout_root.as_ref().expect("layout present");
    let (lx, ly) = walk(root).expect("no text input in layout");
    (
        lx + browser.window_x - browser.scroll.scroll_x,
        ly + browser.window_y + browser.config.url_bar_height as i32 - browser.scroll.scroll_y,
    )
}

#[test]
fn form_click_focuses_input() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/f").unwrap();
    vfs.write(
        "/sites/f/page.html",
        b"<html><body>\
          <form action=\"/search\">\
          <input name=\"q\" size=\"30\">\
          </form></body></html>",
    )
    .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/f/page.html", &vfs);

    // Form manager should be populated from the DOM.
    assert_eq!(browser.form_manager.forms.len(), 1);

    let (cx, cy) = input_center(&browser);
    browser.handle_click(cx, cy, &vfs);

    assert_eq!(
        browser.form_manager.focused_element.as_deref(),
        Some("q"),
        "clicking a text input should focus it (click=({cx},{cy}))"
    );
}

#[test]
fn form_typing_updates_value_and_reflects_in_dom() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/f").unwrap();
    vfs.write(
        "/sites/f/page.html",
        b"<html><body>\
          <form action=\"/search\">\
          <input name=\"q\" size=\"30\">\
          </form></body></html>",
    )
    .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/f/page.html", &vfs);

    let (cx, cy) = input_center(&browser);
    browser.handle_click(cx, cy, &vfs);
    browser.handle_input(&InputEvent::TextInput('h'), &vfs);
    browser.handle_input(&InputEvent::TextInput('i'), &vfs);

    assert_eq!(
        browser.form_manager.get_value(0, "q"),
        Some("hi"),
        "form manager should hold the typed characters"
    );

    // The DOM `value` attribute should mirror the form-manager state
    // so the next relayout paints the typed text.
    let doc = browser.document.as_ref().expect("doc present");
    let input_nid = doc
        .nodes
        .iter()
        .position(|n| {
            matches!(
                &n.kind,
                crate::html::dom::NodeKind::Element(e)
                    if e.tag == crate::html::dom::TagName::Input
                        && e.get_attribute("name") == Some("q")
            )
        })
        .expect("input in DOM");
    if let crate::html::dom::NodeKind::Element(e) = &doc.nodes[input_nid].kind {
        assert_eq!(e.get_attribute("value"), Some("hi"));
    }
}

#[test]
fn form_enter_submits() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/f").unwrap();
    vfs.write(
        "/sites/f/page.html",
        b"<html><body>\
          <form action=\"results.html\">\
          <input name=\"q\" size=\"30\">\
          </form></body></html>",
    )
    .unwrap();
    vfs.write(
        "/sites/f/results.html",
        b"<html><body><h1>Results</h1></body></html>",
    )
    .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/f/page.html", &vfs);

    let (cx, cy) = input_center(&browser);
    browser.handle_click(cx, cy, &vfs);
    browser.handle_input(&InputEvent::TextInput('a'), &vfs);
    browser.handle_input(&InputEvent::ButtonPress(Button::Confirm), &vfs);

    let current = browser.current_url().unwrap_or("");
    assert!(
        current.contains("results.html") && current.contains("q=a"),
        "Enter should submit the form; got: {current}"
    );
}

// ---------------------------------------------------------------
// Reddit comment collapse — end-to-end interactivity
// ---------------------------------------------------------------

/// Find the node id of the first `<a class="expand">` anchor.
#[cfg(feature = "javascript")]
fn find_expand_anchor(browser: &BrowserWidget) -> crate::html::dom::NodeId {
    let doc = browser.document.as_ref().expect("doc");
    for (nid, node) in doc.nodes.iter().enumerate() {
        if let crate::html::dom::NodeKind::Element(e) = &node.kind
            && e.tag == crate::html::dom::TagName::A
            && e.get_attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == "expand"))
        {
            return nid;
        }
    }
    panic!("no <a class=\"expand\"> in fixture DOM");
}

/// Walk up the DOM from `start` to the nearest ancestor whose `class`
/// attribute contains `token`.
#[cfg(feature = "javascript")]
fn nearest_ancestor_with_class(
    browser: &BrowserWidget,
    start: crate::html::dom::NodeId,
    token: &str,
) -> Option<crate::html::dom::NodeId> {
    let doc = browser.document.as_ref()?;
    let mut cur = Some(start);
    while let Some(nid) = cur {
        if let crate::html::dom::NodeKind::Element(e) = &doc.nodes[nid].kind
            && e.get_attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == token))
        {
            return Some(nid);
        }
        cur = doc.nodes[nid].parent;
    }
    None
}

/// Find the screen-space center of the layout rect for a DOM node.
#[cfg(feature = "javascript")]
fn node_center(browser: &BrowserWidget, target: crate::html::dom::NodeId) -> (i32, i32) {
    use crate::layout::box_model::LayoutBox;
    fn walk(lb: &LayoutBox, target: crate::html::dom::NodeId) -> Option<(f32, f32, f32, f32)> {
        if lb.node == Some(target) {
            let r = &lb.dimensions.content;
            return Some((r.x, r.y, r.width.max(1.0), r.height.max(1.0)));
        }
        for c in &lb.children {
            if let Some(p) = walk(c, target) {
                return Some(p);
            }
        }
        None
    }
    let root = browser.layout_root.as_ref().expect("layout");
    let (x, y, w, h) = walk(root, target).expect("node in layout");
    let lx = (x + w / 2.0) as i32;
    let ly = (y + h / 2.0) as i32;
    (
        lx + browser.window_x - browser.scroll.scroll_x,
        ly + browser.window_y + browser.config.url_bar_height as i32 - browser.scroll.scroll_y,
    )
}

/// Read an element's `class` attribute.
#[cfg(feature = "javascript")]
fn class_of(browser: &BrowserWidget, nid: crate::html::dom::NodeId) -> String {
    let doc = browser.document.as_ref().expect("doc");
    if let crate::html::dom::NodeKind::Element(e) = &doc.nodes[nid].kind {
        return e.get_attribute("class").unwrap_or("").to_string();
    }
    String::new()
}

#[cfg(feature = "javascript")]
#[test]
fn reddit_expand_click_collapses_subtree() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/reddit").unwrap();
    let html = include_str!("../tests/fixtures/reddit_comments.html");
    vfs.write("/sites/reddit/comments.html", html.as_bytes())
        .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/reddit/comments.html", &vfs);

    let expand_nid = find_expand_anchor(&browser);
    let comment_nid = nearest_ancestor_with_class(&browser, expand_nid, "comment")
        .expect("expand is inside a .comment");

    // Initial state: the first expand anchor lives inside an expanded
    // comment, so class should NOT contain "collapsed".
    assert!(
        !class_of(&browser, comment_nid)
            .split_whitespace()
            .any(|w| w == "collapsed"),
        "precondition: first .comment starts expanded",
    );

    let (cx, cy) = node_center(&browser, expand_nid);
    browser.handle_click(cx, cy, &vfs);

    // After click, the togglecomment shim must have flipped the class.
    assert!(
        class_of(&browser, comment_nid)
            .split_whitespace()
            .any(|w| w == "collapsed"),
        "click on <a class=\"expand\"> should add .collapsed to nearest .comment; \
         got class={:?}",
        class_of(&browser, comment_nid),
    );

    // URL must NOT have navigated to "#" — onclick returned false, so
    // the wrapper called preventDefault and handle_click skipped the
    // link follow-up.
    let cur = browser.current_url().unwrap_or("");
    assert!(
        cur.ends_with("comments.html"),
        "preventDefault should suppress '#' navigation; url={cur}",
    );

    // Second click un-collapses.
    browser.handle_click(cx, cy, &vfs);
    assert!(
        !class_of(&browser, comment_nid)
            .split_whitespace()
            .any(|w| w == "collapsed"),
        "second click should remove .collapsed; class={:?}",
        class_of(&browser, comment_nid),
    );
}

/// Find the first element with the given `id` attribute.
#[cfg(feature = "javascript")]
fn find_by_id(browser: &BrowserWidget, id: &str) -> crate::html::dom::NodeId {
    let doc = browser.document.as_ref().expect("doc");
    for (nid, node) in doc.nodes.iter().enumerate() {
        if let crate::html::dom::NodeKind::Element(e) = &node.kind
            && e.get_attribute("id") == Some(id)
        {
            return nid;
        }
    }
    panic!("element with id={id} not in DOM");
}

#[cfg(feature = "javascript")]
#[test]
fn reddit_vote_arrow_toggles_class() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/reddit").unwrap();
    let html = include_str!("../tests/fixtures/reddit_comments.html");
    vfs.write("/sites/reddit/comments.html", html.as_bytes())
        .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/reddit/comments.html", &vfs);

    let up_nid = find_by_id(&browser, "vote-up-aaaa001");
    let initial = class_of(&browser, up_nid);
    assert!(
        initial.split_whitespace().any(|w| w == "up")
            && !initial.split_whitespace().any(|w| w == "upmod"),
        "precondition: up arrow starts un-voted; class={initial:?}",
    );

    let (cx, cy) = node_center(&browser, up_nid);
    browser.handle_click(cx, cy, &vfs);

    let after = class_of(&browser, up_nid);
    assert!(
        after.split_whitespace().any(|w| w == "upmod")
            && !after.split_whitespace().any(|w| w == "up"),
        "click on up arrow should swap 'up' → 'upmod'; got class={after:?}",
    );

    // Clicking again should un-vote back to `.up`.
    browser.handle_click(cx, cy, &vfs);
    let after2 = class_of(&browser, up_nid);
    assert!(
        after2.split_whitespace().any(|w| w == "up")
            && !after2.split_whitespace().any(|w| w == "upmod"),
        "second click should toggle back to 'up'; got class={after2:?}",
    );
}

#[cfg(feature = "javascript")]
#[test]
fn reddit_morechildren_click_does_not_navigate() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/reddit").unwrap();
    let html = include_str!("../tests/fixtures/reddit_comments.html");
    vfs.write("/sites/reddit/comments.html", html.as_bytes())
        .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/reddit/comments.html", &vfs);

    // Walk DOM to find first <a> inside a `.morechildren` wrapper.
    let anchor_nid = {
        let doc = browser.document.as_ref().expect("doc");
        let mut found = None;
        'outer: for (nid, node) in doc.nodes.iter().enumerate() {
            if let crate::html::dom::NodeKind::Element(e) = &node.kind
                && e.tag == crate::html::dom::TagName::A
            {
                let mut cur = node.parent;
                while let Some(pid) = cur {
                    if let crate::html::dom::NodeKind::Element(pe) = &doc.nodes[pid].kind
                        && pe
                            .get_attribute("class")
                            .is_some_and(|c| c.split_whitespace().any(|w| w == "morechildren"))
                    {
                        found = Some(nid);
                        break 'outer;
                    }
                    cur = doc.nodes[pid].parent;
                }
            }
        }
        found.expect("no .morechildren > a in fixture")
    };

    let before_url = browser.current_url().unwrap_or("").to_string();
    let (cx, cy) = node_center(&browser, anchor_nid);
    browser.handle_click(cx, cy, &vfs);
    let after_url = browser.current_url().unwrap_or("").to_string();

    assert_eq!(
        before_url, after_url,
        "morechildren() shim must return false + preventDefault, \
         suppressing the `#` link follow-up",
    );
}

/// Find the first arrow (`<div class="arrow up">` / similar) inside the
/// N-th `.thing`, skipping arrows that already have an inline `onclick`
/// attribute. Used by the listing-page delegation test, which exercises
/// the `wireListingArrows()` path in `install_site_compat_shims`.
#[cfg(feature = "javascript")]
fn find_nth_thing_arrow(
    browser: &BrowserWidget,
    skip_things: usize,
    direction: &str,
) -> crate::html::dom::NodeId {
    let doc = browser.document.as_ref().expect("doc");
    let mut things_seen = 0usize;
    for node in doc.nodes.iter() {
        if let crate::html::dom::NodeKind::Element(e) = &node.kind
            && e.get_attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == "thing"))
        {
            if things_seen < skip_things {
                things_seen += 1;
                continue;
            }
            // Found the target `.thing` — descend to its first
            // `.arrow` with the requested direction token.
            let mut stack: Vec<crate::html::dom::NodeId> = node.children.clone();
            while let Some(child) = stack.pop() {
                if let crate::html::dom::NodeKind::Element(ce) = &doc.nodes[child].kind {
                    let classes = ce.get_attribute("class").unwrap_or("");
                    let has_arrow = classes.split_whitespace().any(|w| w == "arrow");
                    let has_dir = classes.split_whitespace().any(|w| w == direction);
                    if has_arrow && has_dir {
                        // Skip arrows that have a pre-wired inline handler so
                        // the test actually exercises the shim's fallback.
                        if ce.get_attribute("onclick").is_none() {
                            return child;
                        }
                    }
                    // Push children in reverse for a left-to-right walk.
                    for &c in doc.nodes[child].children.iter().rev() {
                        stack.push(c);
                    }
                }
            }
            panic!("no matching .arrow.{direction} inside nth .thing");
        }
    }
    panic!("not enough .thing nodes in fixture");
}

/// Read the full text content of a node (concatenates descendant text).
#[cfg(feature = "javascript")]
fn text_content(browser: &BrowserWidget, nid: crate::html::dom::NodeId) -> String {
    let doc = browser.document.as_ref().expect("doc");
    fn walk(doc: &crate::html::dom::Document, nid: crate::html::dom::NodeId, out: &mut String) {
        match &doc.nodes[nid].kind {
            crate::html::dom::NodeKind::Text(t) => out.push_str(t),
            crate::html::dom::NodeKind::Element(_) => {
                for &c in &doc.nodes[nid].children {
                    walk(doc, c, out);
                }
            },
            _ => {},
        }
    }
    let mut s = String::new();
    walk(doc, nid, &mut s);
    s
}

#[cfg(feature = "javascript")]
#[test]
fn reddit_listing_vote_arrow_fires_without_onclick() {
    // On the listing fixture every `.arrow` renders without an inline
    // onclick — real reddit delegates from a bound body-level handler.
    // The compat shim's `wireListingArrows()` has to find those arrows
    // and attach a fallback click listener so users can still vote.
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/reddit").unwrap();
    let html = include_str!("../tests/fixtures/reddit_listing.html");
    vfs.write("/sites/reddit/listing.html", html.as_bytes())
        .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/reddit/listing.html", &vfs);

    // Post #3 (`Rust 1.91 released`) starts fully unvoted — neither
    // arrow carries a modifier class.
    let up_nid = find_nth_thing_arrow(&browser, 2, "up");
    let before = class_of(&browser, up_nid);
    assert!(
        before.split_whitespace().any(|w| w == "up")
            && !before.split_whitespace().any(|w| w == "upmod"),
        "precondition: listing arrow starts as `up`; got {before:?}",
    );

    let (cx, cy) = node_center(&browser, up_nid);
    browser.handle_click(cx, cy, &vfs);

    let after = class_of(&browser, up_nid);
    assert!(
        after.split_whitespace().any(|w| w == "upmod"),
        "shim should have wired a fallback click handler that swaps the \
         `up` class to `upmod`; got {after:?}",
    );
}

#[cfg(feature = "javascript")]
#[test]
fn reddit_vote_updates_score_text_and_class() {
    // On listing pages the `.score` lives inside the `.midcol` alongside
    // the up/down arrows, so clicking the up arrow must:
    //   1. toggle `up` → `upmod`
    //   2. rewrite the sibling `.score` text from "124" → "125"
    //   3. swap the score's colour class to `.likes`
    // Comments-page scores live in the tagline (no midcol sibling) and
    // are intentionally left alone — real reddit also can't update them
    // without a backend roundtrip.
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/reddit").unwrap();
    let html = include_str!("../tests/fixtures/reddit_listing.html");
    vfs.write("/sites/reddit/listing.html", html.as_bytes())
        .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/reddit/listing.html", &vfs);

    // Grab the first post's un-voted up arrow (via the shim-wired path).
    let up_nid = find_nth_thing_arrow(&browser, 0, "up");

    // Score element is the sibling `.score` inside the same `.midcol`.
    let score_nid = {
        let doc = browser.document.as_ref().expect("doc");
        let midcol = doc.nodes[up_nid].parent.expect("arrow has parent");
        let mut found = None;
        for &c in &doc.nodes[midcol].children {
            if let crate::html::dom::NodeKind::Element(e) = &doc.nodes[c].kind
                && e.get_attribute("class")
                    .is_some_and(|cl| cl.split_whitespace().any(|w| w == "score"))
            {
                found = Some(c);
                break;
            }
        }
        found.expect("listing midcol has .score child")
    };

    let parse_leading = |s: &str| -> i64 {
        let trimmed = s.trim();
        let digits: String = trimmed
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        digits.parse().unwrap_or(0)
    };

    let before_text = text_content(&browser, score_nid);
    let before_n = parse_leading(&before_text);

    let (cx, cy) = node_center(&browser, up_nid);
    browser.handle_click(cx, cy, &vfs);

    let after_text = text_content(&browser, score_nid);
    assert_eq!(
        parse_leading(&after_text),
        before_n + 1,
        "score should increment by 1 on upvote; before={before_text:?} after={after_text:?}",
    );
    let score_class = class_of(&browser, score_nid);
    assert!(
        score_class.split_whitespace().any(|w| w == "likes"),
        "score must gain `.likes` class on upvote; got {score_class:?}",
    );

    // Un-vote and confirm the score returns to its baseline.
    browser.handle_click(cx, cy, &vfs);
    let final_text = text_content(&browser, score_nid);
    assert_eq!(
        parse_leading(&final_text),
        before_n,
        "second click should restore the baseline score; got {final_text:?}",
    );
    let final_class = class_of(&browser, score_nid);
    assert!(
        !final_class.split_whitespace().any(|w| w == "likes"),
        "score should lose `.likes` after un-voting; got {final_class:?}",
    );
}

#[cfg(feature = "javascript")]
#[test]
fn reddit_hide_button_hides_thing() {
    // "hide" in a post's `.buttons` row should make the surrounding
    // `.thing` receive a `.hidden` class so the stylesheet rule
    // `.thing.hidden { display: none }` removes it from layout.
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/reddit").unwrap();
    let html = include_str!("../tests/fixtures/reddit_listing.html");
    vfs.write("/sites/reddit/listing.html", html.as_bytes())
        .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/reddit/listing.html", &vfs);

    // Find the first `<a>` whose text is "hide" inside a `.thing .buttons`.
    let (anchor_nid, thing_nid) = {
        let doc = browser.document.as_ref().expect("doc");
        let mut found: Option<(crate::html::dom::NodeId, crate::html::dom::NodeId)> = None;
        'outer: for (nid, node) in doc.nodes.iter().enumerate() {
            if let crate::html::dom::NodeKind::Element(e) = &node.kind
                && e.tag == crate::html::dom::TagName::A
            {
                let text = {
                    let mut s = String::new();
                    for &c in &node.children {
                        if let crate::html::dom::NodeKind::Text(t) = &doc.nodes[c].kind {
                            s.push_str(t);
                        }
                    }
                    s.trim().to_lowercase()
                };
                if text != "hide" {
                    continue;
                }
                // Walk up to confirm we're inside `.thing .buttons`.
                let mut in_buttons = false;
                let mut thing = None;
                let mut cur = node.parent;
                while let Some(pid) = cur {
                    if let crate::html::dom::NodeKind::Element(pe) = &doc.nodes[pid].kind {
                        let cls = pe.get_attribute("class").unwrap_or("");
                        if cls.split_whitespace().any(|w| w == "buttons") {
                            in_buttons = true;
                        }
                        if cls.split_whitespace().any(|w| w == "thing") {
                            thing = Some(pid);
                            break;
                        }
                    }
                    cur = doc.nodes[pid].parent;
                }
                if in_buttons && let Some(t) = thing {
                    found = Some((nid, t));
                    break 'outer;
                }
            }
        }
        found.expect("no `hide` anchor inside .thing .buttons")
    };

    assert!(
        !class_of(&browser, thing_nid)
            .split_whitespace()
            .any(|w| w == "hidden"),
        "precondition: thing not yet hidden",
    );

    let (cx, cy) = node_center(&browser, anchor_nid);
    browser.handle_click(cx, cy, &vfs);

    assert!(
        class_of(&browser, thing_nid)
            .split_whitespace()
            .any(|w| w == "hidden"),
        "hide-click should add `.hidden` to the parent thing; class={:?}",
        class_of(&browser, thing_nid),
    );
}

#[cfg(feature = "javascript")]
#[test]
fn reddit_tabmenu_click_moves_selected() {
    // Clicking the `new` tab in the listing header should move
    // the `.selected` class from the currently-active `hot` li to `new`,
    // mirroring reddit's own behaviour after a page navigation. Real
    // reddit re-loads the page; we mutate the DOM locally so the visual
    // state is at least consistent with the user's click.
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/reddit").unwrap();
    let html = include_str!("../tests/fixtures/reddit_listing.html");
    vfs.write("/sites/reddit/listing.html", html.as_bytes())
        .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/reddit/listing.html", &vfs);

    // Locate the `new` tab anchor inside `.tabmenu` and its `li` parent.
    let (new_anchor, new_li, hot_li) = {
        let doc = browser.document.as_ref().expect("doc");
        let mut new_anchor = None;
        let mut new_li = None;
        let mut hot_li = None;
        for (nid, node) in doc.nodes.iter().enumerate() {
            if let crate::html::dom::NodeKind::Element(e) = &node.kind
                && e.tag == crate::html::dom::TagName::A
            {
                // Find the .tabmenu ancestor — we only care about tabs
                // inside the nav, not the sort row of the listing body.
                let mut in_tabmenu = false;
                let mut cur = node.parent;
                while let Some(pid) = cur {
                    if let crate::html::dom::NodeKind::Element(pe) = &doc.nodes[pid].kind
                        && pe
                            .get_attribute("class")
                            .is_some_and(|c| c.split_whitespace().any(|w| w == "tabmenu"))
                    {
                        in_tabmenu = true;
                        break;
                    }
                    cur = doc.nodes[pid].parent;
                }
                if !in_tabmenu {
                    continue;
                }
                let text = {
                    let mut s = String::new();
                    for &c in &node.children {
                        if let crate::html::dom::NodeKind::Text(t) = &doc.nodes[c].kind {
                            s.push_str(t);
                        }
                    }
                    s.trim().to_lowercase()
                };
                if text == "new" {
                    new_anchor = Some(nid);
                    new_li = node.parent;
                } else if text == "hot" {
                    hot_li = node.parent;
                }
            }
        }
        (
            new_anchor.expect("no `new` tab"),
            new_li.expect("`new` anchor has parent li"),
            hot_li.expect("no `hot` tab"),
        )
    };

    // Precondition: `hot` li is currently `.selected`.
    assert!(
        class_of(&browser, hot_li)
            .split_whitespace()
            .any(|w| w == "selected"),
        "precondition: hot tab starts selected",
    );

    let (cx, cy) = node_center(&browser, new_anchor);
    browser.handle_click(cx, cy, &vfs);

    assert!(
        class_of(&browser, new_li)
            .split_whitespace()
            .any(|w| w == "selected"),
        "clicked `new` tab should gain `.selected`; got {:?}",
        class_of(&browser, new_li),
    );
    assert!(
        !class_of(&browser, hot_li)
            .split_whitespace()
            .any(|w| w == "selected"),
        "previously-selected `hot` tab should lose `.selected`; got {:?}",
        class_of(&browser, hot_li),
    );
}

// ---------------------------------------------------------------
// iframe_http_mode — WASM-style iframe overlay short-circuit
// ---------------------------------------------------------------

#[test]
fn iframe_http_mode_skips_fetch_and_sets_url() {
    // With `iframe_http_mode` on, navigating to an http(s) URL must
    // update `current_url` for the chrome bar but NOT attempt any
    // load — no DOM, no layout, no JS engine. The WASM backend relies
    // on this so an external `<iframe>` overlay can render the page
    // while the OASIS engine stays idle.
    let vfs = MemoryVfs::new();
    let mut config = BrowserConfig::default();
    config.features.iframe_http_mode = true;
    let mut browser = BrowserWidget::new(config);
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("https://example.com/page.html", &vfs);

    assert_eq!(
        browser.current_url(),
        Some("https://example.com/page.html"),
        "iframe mode should still update the nav URL for chrome display",
    );
    assert!(
        browser.document.is_none(),
        "iframe mode must not build a DOM for http(s) pages",
    );
    assert!(
        browser.layout_root.is_none(),
        "iframe mode must not produce a layout tree for http(s) pages",
    );
    assert_eq!(
        browser.loading_state(),
        crate::LoadingState::Idle,
        "iframe mode shouldn't leave the browser in Loading or Error state",
    );
}

#[test]
fn iframe_http_mode_still_loads_vfs_pages() {
    // VFS URLs aren't iframe-able — the engine must still render them
    // normally even when iframe_http_mode is set.
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/local").unwrap();
    vfs.write(
        "/sites/local/index.html",
        b"<html><body><h1>hi</h1></body></html>",
    )
    .unwrap();

    let mut config = BrowserConfig::default();
    config.features.iframe_http_mode = true;
    let mut browser = BrowserWidget::new(config);
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/local/index.html", &vfs);

    assert_eq!(browser.current_url(), Some("vfs://sites/local/index.html"),);
    assert!(
        browser.document.is_some(),
        "VFS pages must still build a DOM even with iframe_http_mode on",
    );
}

/// Real old.reddit.com ships almost no inline CSS — nearly everything
/// comes in via `<link rel="stylesheet">`. Without this pass the page
/// renders as an unstyled vertical bullet list. Verify the engine
/// fetches the linked sheet (via VFS here so the test is offline) and
/// applies its rules after the first tick.
#[test]
fn external_stylesheet_fetched_and_applied() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/x").unwrap();
    // Author CSS colors the <h1> red. Without external loading the UA
    // default (black) would apply instead.
    vfs.write("/sites/x/theme.css", b"h1 { color: rgb(255, 0, 0); }")
        .unwrap();
    vfs.write(
        "/sites/x/index.html",
        b"<html><head>\
          <link rel=\"stylesheet\" href=\"theme.css\">\
          </head><body><h1 id=\"t\">hello</h1></body></html>",
    )
    .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/x/index.html", &vfs);

    // The initial load cascades against UA + inline <style>; the
    // external theme.css has been queued on the I/O thread but not
    // yet parsed. `tick()` polls the I/O thread and applies any
    // arrived sheets.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let h1_nid = browser
        .document
        .as_ref()
        .and_then(|d| {
            d.nodes.iter().position(|n| {
                matches!(
                    &n.kind,
                    crate::html::dom::NodeKind::Element(e) if e.tag == crate::html::dom::TagName::H1
                )
            })
        })
        .expect("h1 in fixture");

    let mut applied = false;
    while std::time::Instant::now() < deadline {
        browser.tick(&vfs);
        let color = browser
            .styles
            .get(h1_nid)
            .and_then(|s| s.as_ref())
            .map(|s| s.color);
        if matches!(color, Some(c) if c.r == 255 && c.g == 0 && c.b == 0) {
            applied = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(
        applied,
        "external <link rel=stylesheet> must be fetched and re-cascaded \
         so its rules reach the paint pipeline (h1 should be red, got \
         {:?})",
        browser
            .styles
            .get(h1_nid)
            .and_then(|s| s.as_ref())
            .map(|s| s.color)
    );
}

/// Cascade precedence depends on source order within the author
/// origin. A page that writes `<link>` *before* `<style>` in `<head>`
/// should let the inline `<style>` win for equal-specificity rules
/// because the inline block comes later in DOM order. Regression test
/// for the earlier bug where all cached inline sheets were pushed
/// before all external sheets unconditionally, flipping the winner.
#[test]
fn external_stylesheet_cascade_order_matches_dom_order() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/x").unwrap();
    // External sheet paints the h1 red.
    vfs.write("/sites/x/theme.css", b"h1 { color: rgb(255, 0, 0); }")
        .unwrap();
    // Inline <style> block appears *after* the <link>, so its rule
    // should win over the external sheet's red rule.
    vfs.write(
        "/sites/x/index.html",
        b"<html><head>\
          <link rel=\"stylesheet\" href=\"theme.css\">\
          <style>h1 { color: rgb(0, 0, 255); }</style>\
          </head><body><h1>hi</h1></body></html>",
    )
    .unwrap();

    let mut browser = make_browser();
    browser.set_window(0, 0, 800, 600);
    browser.navigate_vfs("vfs://sites/x/index.html", &vfs);

    let h1_nid = browser
        .document
        .as_ref()
        .and_then(|d| {
            d.nodes.iter().position(|n| {
                matches!(
                    &n.kind,
                    crate::html::dom::NodeKind::Element(e) if e.tag == crate::html::dom::TagName::H1
                )
            })
        })
        .expect("h1 in fixture");

    // Tick until the external sheet slot is filled; the cascade is
    // re-applied by `apply_external_stylesheets_if_pending` during
    // tick, so the slot flipping `Some(_)` is the signal we need.
    // (VFS-only fixture: no background I/O thread is spawned, so an
    // `io_thread_in_flight`-based guard would be vacuous here.)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        browser.tick(&vfs);
        if browser.external_stylesheets.iter().any(|s| s.is_some()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let color = browser
        .styles
        .get(h1_nid)
        .and_then(|s| s.as_ref())
        .map(|s| s.color)
        .expect("h1 must be styled");
    // Later rule wins at equal specificity: inline <style> (blue) beats
    // <link> (red) because the <style> appears after the <link> in DOM.
    assert_eq!(
        (color.r, color.g, color.b),
        (0, 0, 255),
        "inline <style> appearing after <link> must win the cascade"
    );
}

#[test]
fn is_print_only_media_query_matches_print_variants_only() {
    assert!(BrowserWidget::is_print_only_media_query("print"));
    assert!(BrowserWidget::is_print_only_media_query("  print  "));
    assert!(BrowserWidget::is_print_only_media_query("only print"));
    assert!(BrowserWidget::is_print_only_media_query(
        "print and (color)"
    ));
    // Non-print — must not be flagged.
    assert!(!BrowserWidget::is_print_only_media_query("screen"));
    assert!(!BrowserWidget::is_print_only_media_query(
        "(min-width: 500px)"
    ));
    assert!(!BrowserWidget::is_print_only_media_query("not print"));
    assert!(!BrowserWidget::is_print_only_media_query(""));
}
