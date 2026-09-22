//! Rendering sanity, viewport resizing and scrolling via input.

use oasis_types::backend::Color;
use oasis_types::input::{Button, InputEvent, Trigger};

use super::harness::{Reply, Session, TestServer};
use crate::test_utils::DrawCall;

#[test]
fn colored_boxes_paint_at_expected_pixels() {
    let server = TestServer::start(|_| {
        Reply::html(
            "<html><body style=\"margin:0; background:#ffffff\">\
             <div style=\"width:100px; height:50px; background:#ff0000\"></div>\
             <div style=\"margin-left:20px; width:40px; height:30px; background:rgb(0,0,255)\">\
             </div></body></html>",
        )
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    let top = s.browser.config.url_bar_height as i32;
    let fills: Vec<(i32, i32, u32, u32, Color)> = s
        .frame
        .calls
        .iter()
        .filter_map(|c| match c {
            DrawCall::FillRect { x, y, w, h, color } => Some((*x, *y, *w, *h, *color)),
            _ => None,
        })
        .collect();
    let red = Color::rgb(255, 0, 0);
    let blue = Color::rgb(0, 0, 255);
    assert!(
        fills.contains(&(0, top, 100, 50, red)),
        "red 100x50 box at the content origin; fills: {fills:?}"
    );
    assert!(
        fills.contains(&(20, top + 50, 40, 30, blue)),
        "blue 40x30 box below it, indented 20px; fills: {fills:?}"
    );
}

#[test]
fn hover_and_focus_restyles_keep_linked_stylesheet_rules() {
    let server = TestServer::start(|req| match req.path() {
        "/s.css" => Reply::with(
            200,
            &[("Content-Type", "text/css")],
            "a { color: #00aa00; }",
        ),
        _ => Reply::html(
            "<html><head><link rel=\"stylesheet\" href=\"/s.css\"></head><body>\
             <p><a href=\"/x\">GreenLink</a></p></body></html>",
        ),
    });
    let green = Color::rgb(0, 0xaa, 0);
    let link_color = |s: &Session| {
        s.frame.calls.iter().find_map(|c| match c {
            DrawCall::DrawText { text, color, .. } if text.contains("GreenLink") => Some(*color),
            _ => None,
        })
    };
    let mut s = Session::new();
    s.open(&server.url("/"));
    assert_eq!(link_color(&s), Some(green));
    // Hover it.
    let (x, y) = s.text_pos("GreenLink").unwrap();
    s.input(InputEvent::CursorMove { x, y });
    assert_eq!(
        link_color(&s),
        Some(green),
        "hover restyle dropped linked CSS"
    );
    // Keyboard-focus it.
    s.input(InputEvent::Tab);
    assert_eq!(
        link_color(&s),
        Some(green),
        "focus restyle dropped linked CSS"
    );
}

#[test]
fn resizing_the_window_reevaluates_media_queries() {
    let server = TestServer::start(|req| match req.path() {
        "/m.css" => Reply::with(
            200,
            &[("Content-Type", "text/css")],
            ".lwide { display: none; } \
             @media (min-width: 600px) { .lwide { display: block; } }",
        ),
        _ => Reply::html(
            "<html><head><style>\
             .wide { display: none; }\
             @media (min-width: 600px) { .wide { display: block; } .narrow { display: none; } }\
             </style><link rel=\"stylesheet\" href=\"/m.css\"></head><body>\
             <p class=\"wide\">WideLayout</p><p class=\"narrow\">NarrowLayout</p>\
             <p class=\"lwide\">LinkedWide</p>\
             </body></html>",
        ),
    });
    let mut s = Session::sized(480, 272);
    s.open(&server.url("/"));
    s.assert_shows("NarrowLayout");
    assert!(!s.shows("WideLayout"));
    assert!(!s.shows("LinkedWide"));

    // The window manager enlarges the window.
    s.browser.set_window(0, 0, 800, 480);
    s.settle();
    s.assert_shows("WideLayout");
    s.assert_shows("LinkedWide");
    assert!(!s.shows("NarrowLayout"));

    // And shrinks it again.
    s.browser.set_window(0, 0, 400, 300);
    s.settle();
    s.assert_shows("NarrowLayout");
    assert!(!s.shows("WideLayout"));
    assert!(!s.shows("LinkedWide"));
}

#[test]
fn resizing_reflows_text_to_the_new_width() {
    let server = TestServer::start(|_| {
        let words = "lorem ipsum dolor sit amet ".repeat(40);
        Reply::html(format!(
            "<html><body><p>{words}</p><p>EndMarker</p></body></html>"
        ))
    });
    let mut s = Session::sized(640, 400);
    s.open(&server.url("/"));
    let wide_y = s.text_pos("EndMarker").unwrap().1;
    s.browser.set_window(0, 0, 320, 400);
    s.settle();
    // Narrower window → more lines → the marker moves down (or off
    // screen entirely).
    let narrow_y = s.text_pos("EndMarker").map_or(i32::MAX, |p| p.1);
    assert!(narrow_y > wide_y, "{narrow_y} should be below {wide_y}");
}

#[test]
fn scrolling_a_long_page_with_buttons_wheel_and_triggers() {
    let server = TestServer::start(|_| {
        let mut body = String::new();
        for i in 0..300 {
            body.push_str(&format!("<p>row{i}</p>"));
        }
        Reply::html(format!("<html><body>{body}</body></html>"))
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.assert_shows("row0");
    assert!(!s.shows("row299"));

    s.press(Button::Down);
    let after_line = s.browser.scroll().scroll_y;
    assert!(after_line > 0);
    s.input(InputEvent::MouseWheel { delta: 3 });
    let after_wheel = s.browser.scroll().scroll_y;
    assert!(after_wheel > after_line);
    s.input(InputEvent::TriggerPress(Trigger::Right));
    assert!(s.browser.scroll().scroll_y > after_wheel);
    assert!(!s.shows("row0"), "first row scrolled out of view");

    // Page down to the end: scrolling is clamped and the last row shows.
    for _ in 0..200 {
        if s.browser.scroll().at_bottom() {
            break;
        }
        s.input(InputEvent::TriggerPress(Trigger::Right));
    }
    assert!(s.browser.scroll().at_bottom());
    let bottom = s.browser.scroll().scroll_y;
    s.input(InputEvent::TriggerPress(Trigger::Right));
    assert_eq!(s.browser.scroll().scroll_y, bottom, "clamped at the bottom");
    s.assert_shows("row299");

    // And back up to the top.
    for _ in 0..200 {
        if s.browser.scroll().at_top() {
            break;
        }
        s.input(InputEvent::TriggerPress(Trigger::Left));
    }
    s.input(InputEvent::MouseWheel { delta: -5 });
    assert_eq!(s.browser.scroll().scroll_y, 0);
    s.assert_shows("row0");
}

#[test]
fn wheel_over_nested_scroll_container_scrolls_it_then_the_page() {
    let server = TestServer::start(|_| {
        let mut inner = String::new();
        for i in 0..40 {
            inner.push_str(&format!("<p>inner{i}</p>"));
        }
        let mut outer = String::new();
        for i in 0..80 {
            outer.push_str(&format!("<p>outer{i}</p>"));
        }
        Reply::html(format!(
            "<html><body style=\"margin:0\">\
             <div id=\"box\" style=\"height:120px; overflow:auto\">{inner}</div>\
             {outer}</body></html>"
        ))
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.assert_shows("inner0");
    assert!(!s.shows("inner39"));

    // Hover the box, then wheel: the box scrolls, the page does not.
    let (x, y) = s.text_pos("inner0").unwrap();
    s.input(InputEvent::CursorMove { x, y });
    s.input(InputEvent::MouseWheel { delta: 2 });
    assert_eq!(s.browser.scroll().scroll_y, 0, "page must not scroll");
    assert!(!s.shows("inner0"), "box content scrolled");
    s.assert_shows("outer0");

    // Keep wheeling: once the box hits its end, the page scrolls.
    for _ in 0..60 {
        if s.browser.scroll().scroll_y > 0 {
            break;
        }
        s.input(InputEvent::MouseWheel { delta: 2 });
    }
    assert!(
        s.browser.scroll().scroll_y > 0,
        "wheel bubbles to the page at the limit"
    );
}
