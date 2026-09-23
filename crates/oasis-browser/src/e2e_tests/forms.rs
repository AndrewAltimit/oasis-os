//! Form interaction: focus, typing, checkboxes, selects, labels, and
//! GET / POST submission as seen by the server.

use oasis_types::input::{Button, InputEvent};

use super::harness::{Reply, Session, TestServer, form_pairs};
use crate::layout::box_model::ReplacedContent;

const FORM_PAGE: &str = "<html><head><title>Form</title></head><body>\
    <form action=\"/search\" method=\"get\">\
      <p><label for=\"q\">SearchLabel</label> \
         <input type=\"text\" id=\"q\" name=\"q\" value=\"\" style=\"width:200px\"></p>\
      <p><select id=\"color\" name=\"color\">\
           <option value=\"red\">Red</option>\
           <option value=\"green\" selected>Green</option>\
           <option value=\"blue\">Blue</option>\
         </select></p>\
      <p><input type=\"checkbox\" id=\"agree\" name=\"agree\" value=\"yes\"> \
         <label for=\"agree\">AgreeLabel</label></p>\
      <p><label for=\"wrap\"><input type=\"checkbox\" id=\"wrap\" name=\"wrap\" value=\"on\"> \
         WrapLabel</label></p>\
      <p><label><input type=\"checkbox\" id=\"nofor\" name=\"nofor\" value=\"1\"> \
         NoForLabel</label></p>\
      <p><input type=\"checkbox\" id=\"bare\" name=\"bare\" checked> Bare</p>\
      <p><input type=\"submit\" id=\"go\" value=\"Go\"></p>\
    </form>\
    <form action=\"/post\" method=\"post\">\
      <p><input type=\"text\" id=\"msg\" name=\"msg\" style=\"width:200px\">\
         <input type=\"hidden\" name=\"tok\" value=\"a&amp;b\">\
         <input type=\"submit\" id=\"send\" name=\"act\" value=\"Send\"></p>\
    </form>\
    </body></html>";

fn form_server() -> TestServer {
    TestServer::start(|req| match req.path() {
        "/form" => Reply::html(FORM_PAGE),
        "/search" => Reply::html("<html><body><p>SearchResults</p></body></html>"),
        "/post" if req.method == "POST" => Reply::html(format!(
            "<html><body><p>Posted{}</p></body></html>",
            req.body.len()
        )),
        _ => Reply::not_found(),
    })
}

fn field<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

#[test]
fn get_form_label_focus_typing_checkbox_select_submit() {
    let server = form_server();
    let mut s = Session::new();
    s.open(&server.url("/form"));

    // Clicking the label focuses the associated input.
    s.click_text("SearchLabel");
    assert!(s.browser.accepts_text(), "label click should focus #q");
    s.type_str("hello world&x=1");
    // The typed value is painted in the field.
    s.assert_shows("hello");
    s.input(InputEvent::Backspace);

    // Checkbox via its label; the painted control reflects the state.
    assert!(matches!(
        s.replaced("agree"),
        ReplacedContent::Checkbox { checked: false }
    ));
    s.click_text("AgreeLabel");
    assert!(matches!(
        s.replaced("agree"),
        ReplacedContent::Checkbox { checked: true }
    ));
    assert!(
        !s.browser.accepts_text(),
        "checkbox focus is not a text field"
    );
    // Select: click to focus, then arrow down to the next option.
    s.assert_shows("Green");
    let scroll_before = s.browser.scroll().scroll_y;
    s.click_element("color");
    s.press(Button::Down);
    assert_eq!(
        s.browser.scroll().scroll_y,
        scroll_before,
        "arrow went to the select"
    );
    s.assert_shows("Blue");
    assert!(!s.shows("Green"));

    s.click_element("go");
    s.assert_shows("SearchResults");

    let reqs = server.requests_to("/search");
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "GET");
    let pairs = form_pairs(reqs[0].query().unwrap_or(""));
    assert_eq!(field(&pairs, "q"), Some("hello world&x="), "{pairs:?}");
    assert_eq!(field(&pairs, "agree"), Some("yes"), "{pairs:?}");
    assert_eq!(field(&pairs, "color"), Some("blue"), "{pairs:?}");
    assert_eq!(field(&pairs, "wrap"), None, "{pairs:?}");
    assert_eq!(field(&pairs, "nofor"), None, "{pairs:?}");
    // A checkbox without a value attribute submits "on"; the unnamed
    // submit button contributes nothing.
    assert_eq!(field(&pairs, "bare"), Some("on"), "{pairs:?}");
    assert_eq!(pairs.len(), 4, "{pairs:?}");
    // Raw encoding: reserved characters in values are escaped.
    let q = reqs[0].query().unwrap();
    assert!(q.contains("q=hello+world%26x%3D"), "{q}");
}

#[test]
fn checkbox_inside_label_toggles_once() {
    let server = form_server();
    let mut s = Session::new();
    s.open(&server.url("/form"));
    // Clicking the checkbox itself, which sits inside its own
    // `<label for>`, must toggle it exactly once.
    s.click_element("wrap");
    // Clicking the text of a wrapping label without `for` toggles the
    // wrapped control (HTML label activation behaviour).
    s.click_text("NoForLabel");
    s.click_element("go");
    let reqs = server.requests_to("/search");
    assert_eq!(reqs.len(), 1);
    let pairs = form_pairs(reqs[0].query().unwrap_or(""));
    assert_eq!(field(&pairs, "wrap"), Some("on"), "{pairs:?}");
    assert_eq!(field(&pairs, "nofor"), Some("1"), "{pairs:?}");
    // Untouched select keeps its `selected` default.
    assert_eq!(field(&pairs, "color"), Some("green"), "{pairs:?}");
}

#[test]
fn post_form_enter_and_button_submit_encoded_body() {
    let server = form_server();
    let mut s = Session::new();
    s.open(&server.url("/form"));

    s.click_element("msg");
    assert!(s.browser.accepts_text());
    s.type_str("a b=c");
    // Enter submits the focused field's form.
    s.press(Button::Confirm);
    let posts = server.requests_to("/post");
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].method, "POST");
    assert_eq!(
        posts[0].header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    let pairs = form_pairs(std::str::from_utf8(&posts[0].body).unwrap());
    assert_eq!(field(&pairs, "msg"), Some("a b=c"), "{pairs:?}");
    assert_eq!(field(&pairs, "tok"), Some("a&b"), "{pairs:?}");
    s.assert_shows("Posted");
    assert_eq!(s.url(), server.url("/post"));

    // Back to the form, submit with the named button this time.
    s.press(Button::Cancel);
    assert_eq!(s.url(), server.url("/form"));
    s.click_element("msg");
    s.type_str("zz");
    s.click_element("send");
    let posts = server.requests_to("/post");
    assert_eq!(posts.len(), 2, "button submit must re-POST, not use cache");
    let pairs = form_pairs(std::str::from_utf8(&posts[1].body).unwrap());
    assert_eq!(field(&pairs, "msg"), Some("zz"), "{pairs:?}");
    assert_eq!(field(&pairs, "act"), Some("Send"), "{pairs:?}");
}

/// Clicking into a pre-filled field put the caret at the start (the
/// form's caret was never reset on click focus), so typing prepended.
#[test]
fn typing_into_a_prefilled_field_appends() {
    let server = TestServer::start(|_| {
        Reply::html(
            "<html><body><form action=\"/x\"><p><input type=\"text\" id=\"p\" \
             name=\"p\" value=\"pre\" style=\"width:200px\"></p></form></body></html>",
        )
    });
    let mut s = Session::new();
    s.open(&server.url("/"));
    s.click_element("p");
    s.type_str("X");
    match s.replaced("p") {
        ReplacedContent::TextInput { value, .. } => assert_eq!(value, "preX"),
        other => panic!("not a text input: {other:?}"),
    }
}
