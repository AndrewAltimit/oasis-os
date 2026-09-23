//! Tests for the JS DOM bindings.

use super::*;
use oasis_js::JsEngine;

/// Build a tiny DOM:
///
/// ```text
/// Document(0)
///   html(1)
///     head(2)
///       title(3)
///         "Test"(4)
///     body(5)
///       div#main(6)
///         p(7)
///           "hello"(8)
/// ```
fn sample_doc() -> Document {
    let mut doc = Document::new();
    let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
    doc.append_child(doc.root, html);

    let head = doc.add_node(NodeKind::Element(ElementData::new(TagName::Head)));
    doc.append_child(html, head);
    let title = doc.add_node(NodeKind::Element(ElementData::new(TagName::Title)));
    doc.append_child(head, title);
    let title_text = doc.add_node(NodeKind::Text("Test".into()));
    doc.append_child(title, title_text);

    let body = doc.add_node(NodeKind::Element(ElementData::new(TagName::Body)));
    doc.append_child(html, body);

    let mut div_data = ElementData::new(TagName::Div);
    div_data.set_attribute("id", "main");
    let div = doc.add_node(NodeKind::Element(div_data));
    doc.append_child(body, div);

    let p = doc.add_node(NodeKind::Element(ElementData::new(TagName::P)));
    doc.append_child(div, p);
    let text = doc.add_node(NodeKind::Text("hello".into()));
    doc.append_child(p, text);

    doc
}

/// Helper: create engine + shared doc, install document global.
fn setup(doc: Document) -> (JsEngine, SharedDoc) {
    let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
    let shared: SharedDoc = Rc::new(RefCell::new(doc));
    let s = Rc::clone(&shared);
    engine
        .with_context(|ctx| install_document_global(&ctx, &s))
        .unwrap();
    (engine, shared)
}

#[test]
fn get_element_by_id_returns_proxy() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval("document.getElementById('main').tagName")
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("DIV".into()));
}

#[test]
fn get_element_by_id_returns_null_for_missing() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval("document.getElementById('nope') === null")
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::Bool(true));
}

#[test]
fn set_text_content_from_js() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval("document.getElementById('main').textContent = 'new text'")
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").unwrap();
    assert_eq!(doc.text_content(main), "new text");
}

#[test]
fn set_attribute_from_js() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval("document.getElementById('main').setAttribute('class', 'foo')")
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").unwrap();
    let elem = doc.element(main).unwrap();
    assert_eq!(elem.get_attribute("class"), Some("foo"));
}

#[test]
fn remove_attribute_from_js() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "var el = document.getElementById('main'); \
                 el.setAttribute('data-x', '1'); \
                 el.removeAttribute('data-x')",
        )
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").unwrap();
    let elem = doc.element(main).unwrap();
    assert_eq!(elem.get_attribute("data-x"), None);
}

#[test]
fn create_element_and_append() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "var span = document.createElement('span'); \
                 span.textContent = 'added'; \
                 document.getElementById('main').appendChild(span)",
        )
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").unwrap();
    let text = doc.text_content(main);
    assert!(text.contains("added"));
}

#[test]
fn create_text_node_and_append() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "var t = document.createTextNode(' world'); \
                 document.body.appendChild(t)",
        )
        .unwrap();
    let doc = shared.borrow();
    let body = doc.body().unwrap();
    let text = doc.text_content(body);
    assert!(text.contains(" world"));
}

#[test]
fn set_id_from_js() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval("document.getElementById('main').id = 'changed'")
        .unwrap();
    let doc = shared.borrow();
    assert!(doc.get_element_by_id("main").is_none());
    assert!(doc.get_element_by_id("changed").is_some());
}

#[test]
fn document_title_get_and_set() {
    let (engine, shared) = setup(sample_doc());
    let val = engine.eval("document.title").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("Test".into()));

    engine.eval("document.title = 'New Title'").unwrap();
    assert_eq!(shared.borrow().title(), Some("New Title".into()));
}

#[test]
fn append_child_moves_node() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "var sec = document.createElement('section'); \
                 document.body.appendChild(sec); \
                 var p = document.getElementById('main').children[0]; \
                 sec.appendChild(p)",
        )
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").unwrap();
    let main_children: Vec<_> = doc.get(main).children.to_vec();
    assert!(
        main_children.iter().all(|&c| !matches!(
            doc.get(c).kind,
            NodeKind::Element(ref e) if e.tag == TagName::P
        )),
        "p should no longer be under div#main"
    );
}

#[test]
fn dom_mutations_persist_after_engine_drop() {
    let shared: SharedDoc = Rc::new(RefCell::new(sample_doc()));
    {
        let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
        let s = Rc::clone(&shared);
        engine
            .with_context(|ctx| install_document_global(&ctx, &s))
            .unwrap();
        engine
            .eval("document.getElementById('main').textContent = 'persisted'")
            .unwrap();
    }
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").unwrap();
    assert_eq!(doc.text_content(main), "persisted");
}

// ---------------------------------------------------------------
// Event listener + dispatch tests
// ---------------------------------------------------------------

#[test]
fn add_event_listener_and_dispatch() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var clicked = false; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { clicked = true; }); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("clicked").unwrap();
    assert_eq!(val, oasis_js::JsValue::Bool(true));
}

#[test]
fn multiple_listeners_on_same_element() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { count++; }); \
                 el.addEventListener('click', function() { count += 10; }); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(11));
}

#[test]
fn remove_event_listener() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1); \
                 el.removeEventListener('click', fn1); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(0));
}

#[test]
fn dispatch_with_bubbling_child_to_parent() {
    let (engine, _doc) = setup(sample_doc());
    // p(7) is child of div#main(6).
    engine
        .eval(
            "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 p.addEventListener('click', function() { order.push('p'); }); \
                 div.addEventListener('click', function() { order.push('div'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("order.join(',')").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("p,div".into()));
}

#[test]
fn stop_propagation_prevents_bubbling() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 p.addEventListener('click', function(e) { \
                     order.push('p'); e.stopPropagation(); }); \
                 div.addEventListener('click', function() { order.push('div'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("order.join(',')").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("p".into()));
}

#[test]
fn bubbling_event_target_vs_current_target() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var targetTag = ''; var currentTag = ''; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function(e) { \
                     targetTag = e.target.tagName; \
                     currentTag = e.currentTarget.tagName; \
                 }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let target = engine.eval("targetTag").unwrap();
    let current = engine.eval("currentTag").unwrap();
    assert_eq!(target, oasis_js::JsValue::String("P".into()));
    assert_eq!(current, oasis_js::JsValue::String("DIV".into()));
}

#[test]
fn dispatch_event_receives_detail() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var received = null; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('custom', function(e) { \
                     received = e.detail; \
                 }); \
                 __oasis_dispatch_event(\
                     el.__oasis_node_id, 'custom', 'payload')",
        )
        .unwrap();
    let val = engine.eval("received").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("payload".into()));
}

#[test]
fn retained_engine_fires_events_after_script_exec() {
    // Simulate what widget/pipeline.rs does: create engine, run
    // scripts, then dispatch events later.
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var clicked = false; \
                 document.getElementById('main')\
                     .addEventListener('click', function() { \
                         clicked = true; \
                     })",
        )
        .unwrap();

    // Later, Rust dispatches an event.
    engine
        .eval("__oasis_dispatch_with_bubbling(6, 'click', null)")
        .unwrap();
    let val = engine.eval("clicked").unwrap();
    assert_eq!(val, oasis_js::JsValue::Bool(true));
}

// ---------------------------------------------------------------
// addEventListener options tests
// ---------------------------------------------------------------

#[test]
fn once_option_removes_after_first_call() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { count++; }, {once: true}); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(1));
}

#[test]
fn capture_option_fires_in_capture_phase() {
    let (engine, _doc) = setup(sample_doc());
    // p(7) is child of div#main(6).
    // Capture listener on div fires before bubble listener on p.
    engine
        .eval(
            "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function() { order.push('div-cap'); }, true); \
                 p.addEventListener('click', function() { order.push('p'); }); \
                 div.addEventListener('click', function() { order.push('div-bub'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("order.join(',')").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("div-cap,p,div-bub".into()));
}

#[test]
fn remove_listener_must_match_capture_flag() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1, true); \
                 el.removeEventListener('click', fn1, false); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    // Listener was added with capture=true, removed with capture=false,
    // so it should NOT be removed.
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(1));
}

#[test]
fn remove_listener_with_matching_capture() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1, true); \
                 el.removeEventListener('click', fn1, true); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(0));
}

#[test]
fn boolean_capture_arg_works() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function() { order.push('cap'); }, true); \
                 div.addEventListener('click', function() { order.push('bub'); }, false); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("order.join(',')").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("cap,bub".into()));
}

#[test]
fn once_with_bubbling() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 var el = document.getElementById('main'); \
                 el.addEventListener('click', function() { count++; }, {once: true}); \
                 __oasis_dispatch_with_bubbling(\
                     el.__oasis_node_id, 'click', null); \
                 __oasis_dispatch_with_bubbling(\
                     el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(1));
}

#[test]
fn document_once_listener() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 document.addEventListener('custom', function() { count++; }, {once: true}); \
                 document.dispatchEvent({type: 'custom'}); \
                 document.dispatchEvent({type: 'custom'})",
        )
        .unwrap();
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(1));
}

#[test]
fn duplicate_listener_prevented() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var count = 0; \
                 var el = document.getElementById('main'); \
                 var fn1 = function() { count++; }; \
                 el.addEventListener('click', fn1); \
                 el.addEventListener('click', fn1); \
                 __oasis_dispatch_event(el.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    // Per spec, adding the same fn+capture combo twice is a no-op.
    let val = engine.eval("count").unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(1));
}

#[test]
fn stop_propagation_in_capture_phase() {
    let (engine, _doc) = setup(sample_doc());
    engine
        .eval(
            "var order = []; \
                 var p = document.getElementById('main').children[0]; \
                 var div = document.getElementById('main'); \
                 div.addEventListener('click', function(e) { \
                     order.push('div-cap'); e.stopPropagation(); \
                 }, true); \
                 p.addEventListener('click', function() { order.push('p'); }); \
                 div.addEventListener('click', function() { order.push('div-bub'); }); \
                 __oasis_dispatch_with_bubbling(\
                     p.__oasis_node_id, 'click', null)",
        )
        .unwrap();
    let val = engine.eval("order.join(',')").unwrap();
    // Capture listener stops propagation, so target and bubble never fire.
    assert_eq!(val, oasis_js::JsValue::String("div-cap".into()));
}

// ---------------------------------------------------------------
// innerHTML tests
// ---------------------------------------------------------------

#[test]
fn inner_html_get() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval("document.getElementById('main').innerHTML")
        .unwrap();
    // div#main contains <p>hello</p>
    if let oasis_js::JsValue::String(s) = val {
        assert!(
            s.contains("<p>") && s.contains("hello"),
            "unexpected innerHTML: {s}"
        );
    } else {
        panic!("expected string");
    }
}

#[test]
fn inner_html_set() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "document.getElementById('main').innerHTML = \
                 '<span>new</span>'",
        )
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").expect("main");
    let text = doc.text_content(main);
    assert_eq!(text, "new");
    // Should have one child: <span>
    let children: Vec<_> = doc.nodes[main]
        .children
        .iter()
        .copied()
        .filter(|&c| matches!(doc.nodes[c].kind, NodeKind::Element(_)))
        .collect();
    assert_eq!(children.len(), 1);
    let child_elem = doc.element(children[0]).expect("elem");
    assert_eq!(child_elem.tag, TagName::Span);
}

#[test]
fn inner_html_set_empty() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval("document.getElementById('main').innerHTML = ''")
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").expect("main");
    assert!(doc.nodes[main].children.is_empty());
}

// ---------------------------------------------------------------
// querySelector / querySelectorAll tests
// ---------------------------------------------------------------

#[test]
fn query_selector_by_tag() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine.eval("document.querySelector('p').tagName").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("P".into()));
}

#[test]
fn query_selector_by_id() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval("document.querySelector('#main').tagName")
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("DIV".into()));
}

#[test]
fn query_selector_by_class() {
    let (engine, _doc) = setup(sample_doc());
    // Add a class first, then query by it.
    engine
        .eval(
            "document.getElementById('main')\
                 .classList.add('highlight')",
        )
        .unwrap();
    let val = engine
        .eval("document.querySelector('.highlight').id")
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("main".into()));
}

#[test]
fn query_selector_returns_null_for_no_match() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval("document.querySelector('.nope') === null")
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::Bool(true));
}

#[test]
fn query_selector_all_returns_array() {
    let (engine, _doc) = setup(sample_doc());
    // Add another div to body for multiple matches.
    engine
        .eval(
            "var d = document.createElement('div'); \
                 document.body.appendChild(d)",
        )
        .unwrap();
    let val = engine
        .eval("document.querySelectorAll('div').length")
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::Int(2));
}

#[test]
fn query_selector_compound() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval("document.querySelector('div#main').tagName")
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("DIV".into()));
}

#[test]
fn element_query_selector() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval(
            "document.getElementById('main')\
                 .querySelector('p').textContent",
        )
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("hello".into()));
}

// ---------------------------------------------------------------
// classList tests
// ---------------------------------------------------------------

#[test]
fn classlist_add_and_contains() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "var el = document.getElementById('main'); \
                 el.classList.add('foo'); \
                 el.classList.add('bar')",
        )
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").expect("main");
    let elem = doc.element(main).expect("elem");
    assert!(elem.has_class("foo"));
    assert!(elem.has_class("bar"));
}

#[test]
fn classlist_remove() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "var el = document.getElementById('main'); \
                 el.classList.add('foo'); \
                 el.classList.add('bar'); \
                 el.classList.remove('foo')",
        )
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").expect("main");
    let elem = doc.element(main).expect("elem");
    assert!(!elem.has_class("foo"));
    assert!(elem.has_class("bar"));
}

#[test]
fn classlist_toggle() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval(
            "var el = document.getElementById('main'); \
                 var r1 = el.classList.toggle('active'); \
                 var r2 = el.classList.toggle('active'); \
                 '' + r1 + ',' + r2",
        )
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("true,false".into()));
}

#[test]
fn classlist_contains() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval(
            "var el = document.getElementById('main'); \
                 el.classList.add('yes'); \
                 '' + el.classList.contains('yes') + ',' + \
                 el.classList.contains('no')",
        )
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("true,false".into()));
}

// ---------------------------------------------------------------
// style tests
// ---------------------------------------------------------------

#[test]
fn style_set_and_get() {
    let (engine, shared) = setup(sample_doc());
    engine
        .eval(
            "var el = document.getElementById('main'); \
                 el.style.setProperty('color', 'red'); \
                 el.style.setProperty('font-size', '14px')",
        )
        .unwrap();
    let doc = shared.borrow();
    let main = doc.get_element_by_id("main").expect("main");
    let elem = doc.element(main).expect("elem");
    let style = elem.get_attribute("style").expect("style");
    assert!(style.contains("color: red"));
    assert!(style.contains("font-size: 14px"));
}

#[test]
fn style_get_property_value() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval(
            "var el = document.getElementById('main'); \
                 el.style.setProperty('color', 'blue'); \
                 el.style.getPropertyValue('color')",
        )
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("blue".into()));
}

#[test]
fn style_overwrite_property() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine
        .eval(
            "var el = document.getElementById('main'); \
                 el.style.setProperty('color', 'red'); \
                 el.style.setProperty('color', 'green'); \
                 el.style.getPropertyValue('color')",
        )
        .unwrap();
    assert_eq!(val, oasis_js::JsValue::String("green".into()));
}

// ---------------------------------------------------------------
// window.location tests
// ---------------------------------------------------------------

#[test]
fn location_href_default() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine.eval("location.href").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("".into()));
}

#[test]
fn location_href_with_url() {
    let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
    let doc = sample_doc();
    let shared: SharedDoc = Rc::new(RefCell::new(doc));
    let s = Rc::clone(&shared);
    engine
        .with_context(|ctx| install_document_global_with_url(&ctx, &s, "https://example.com/page"))
        .unwrap();
    let val = engine.eval("location.href").unwrap();
    assert_eq!(
        val,
        oasis_js::JsValue::String("https://example.com/page".into())
    );
}

#[test]
fn window_is_global_this() {
    let (engine, _doc) = setup(sample_doc());
    let val = engine.eval("window === globalThis").unwrap();
    assert_eq!(val, oasis_js::JsValue::Bool(true));
}

// ---------------------------------------------------------------
// Navigation action tests
// ---------------------------------------------------------------

/// Helper: create engine + shared doc + nav actions queue.
fn setup_with_nav(doc: Document, url: &str) -> (JsEngine, SharedDoc, SharedNavActions) {
    let engine = JsEngine::new(8 * 1024 * 1024).unwrap();
    let shared: SharedDoc = Rc::new(RefCell::new(doc));
    let nav_actions: SharedNavActions = Rc::new(RefCell::new(Vec::new()));
    let s = Rc::clone(&shared);
    let n = Rc::clone(&nav_actions);
    engine
        .with_context(|ctx| install_document_global_with_nav(&ctx, &s, url, &n))
        .unwrap();
    (engine, shared, nav_actions)
}

#[test]
fn location_assign_queues_navigate() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("location.assign('https://other.com')").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0],
        JsNavAction::Navigate("https://other.com".into())
    );
}

#[test]
fn location_href_setter_queues_navigate() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("location.href = 'https://new.com'").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], JsNavAction::Navigate("https://new.com".into()));
}

#[test]
fn location_replace_queues_navigate() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine
        .eval("location.replace('https://replaced.com')")
        .unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0],
        JsNavAction::Navigate("https://replaced.com".into())
    );
}

#[test]
fn history_back_queues_action() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("history.back()").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], JsNavAction::Back);
}

#[test]
fn history_forward_queues_action() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("history.forward()").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], JsNavAction::Forward);
}

#[test]
fn history_go_negative_is_back() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("history.go(-1)").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], JsNavAction::Back);
}

#[test]
fn history_go_positive_is_forward() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("history.go(1)").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], JsNavAction::Forward);
}

#[test]
fn history_go_zero_reloads() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("history.go(0)").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0],
        JsNavAction::Navigate("https://example.com".into())
    );
}

#[test]
fn location_href_getter_with_nav() {
    let (engine, _doc, _nav) = setup_with_nav(sample_doc(), "https://example.com/page");
    let val = engine.eval("location.href").unwrap();
    assert_eq!(
        val,
        oasis_js::JsValue::String("https://example.com/page".into())
    );
}

#[test]
fn location_tostring() {
    let (engine, _doc, _nav) = setup_with_nav(sample_doc(), "https://example.com");
    let val = engine.eval("location.toString()").unwrap();
    assert_eq!(val, oasis_js::JsValue::String("https://example.com".into()));
}

#[test]
fn drain_nav_actions_clears_queue() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine.eval("history.back()").unwrap();
    engine.eval("history.forward()").unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 2);
    // Second drain should be empty.
    let actions2 = drain_nav_actions(&nav);
    assert!(actions2.is_empty());
}

#[test]
fn window_location_assign_works() {
    let (engine, _doc, nav) = setup_with_nav(sample_doc(), "https://example.com");
    engine
        .eval("window.location.assign('https://via-window.com')")
        .unwrap();
    let actions = drain_nav_actions(&nav);
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0],
        JsNavAction::Navigate("https://via-window.com".into())
    );
}

// ------------------------------------------------------------------
// fetch() + Web Storage (origin policy, promises, quota)
// ------------------------------------------------------------------

type RecordedCall = (String, String, Vec<(String, String)>, Option<String>);
type CannedResponse = (u16, Vec<(String, String)>, String);

/// Transport double: canned responses by URL, records every request.
#[derive(Clone, Default)]
struct MockTransport {
    calls: Rc<RefCell<Vec<RecordedCall>>>,
    responses: Rc<RefCell<std::collections::HashMap<String, CannedResponse>>>,
}

impl MockTransport {
    fn respond(&self, url: &str, status: u16, headers: &[(&str, &str)], body: &str) {
        let headers = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.responses
            .borrow_mut()
            .insert(url.to_string(), (status, headers, body.to_string()));
    }

    fn urls(&self) -> Vec<String> {
        self.calls.borrow().iter().map(|c| c.1.clone()).collect()
    }
}

impl fetch::FetchTransport for MockTransport {
    fn resolve_host(&self, _host: &str) -> Vec<std::net::IpAddr> {
        Vec::new()
    }

    fn supports_https(&self) -> bool {
        true
    }

    fn send(
        &self,
        req: &fetch::TransportRequest<'_>,
        _redirect_ok: &dyn Fn(&crate::loader::Url) -> bool,
        _addr_ok: &dyn Fn(std::net::IpAddr) -> bool,
    ) -> Result<fetch::RawResponse, String> {
        let url = req.url.to_string();
        self.calls.borrow_mut().push((
            req.method.to_string(),
            url.clone(),
            req.headers.to_vec(),
            req.body.map(|b| String::from_utf8_lossy(b).into_owned()),
        ));
        let (status, headers, body) = self.responses.borrow().get(&url).cloned().unwrap_or((
            404,
            Vec::new(),
            "Not Found".into(),
        ));
        Ok(fetch::RawResponse {
            status,
            headers,
            body: body.into_bytes(),
            url,
        })
    }
}

/// Install the full page bindings for `url`, optionally with a shared
/// storage backend and a mock fetch transport.
fn setup_page(
    url: &str,
    store: Option<&SharedLocalStorage>,
    transport: Option<&MockTransport>,
) -> JsEngine {
    let engine = JsEngine::new(32 * 1024 * 1024).unwrap();
    let shared: SharedDoc = Rc::new(RefCell::new(sample_doc()));
    let nav: SharedNavActions = Rc::new(RefCell::new(Vec::new()));
    engine
        .with_context(|ctx| {
            install_document_global_full(&ctx, &shared, url, &nav, None, None, None, store, None)?;
            if let Some(t) = transport {
                let handler = fetch::BrowserFetchHandler::new(url, None, Box::new(t.clone()));
                oasis_js::fetch::bind_fetch_handler(&ctx, Box::new(handler))?;
            }
            Ok(())
        })
        .unwrap();
    engine
}

fn js_str(engine: &JsEngine, expr: &str) -> String {
    match engine.eval(expr).unwrap() {
        oasis_js::JsValue::String(s) => s,
        other => other.to_string(),
    }
}

#[test]
fn fetch_then_then_chaining_yields_parsed_data() {
    let t = MockTransport::default();
    t.respond(
        "https://example.com/api/data",
        200,
        &[("Content-Type", "application/json")],
        r#"{"n":42}"#,
    );
    let engine = setup_page("https://example.com/app/index.html", None, Some(&t));
    engine
        .eval(
            "globalThis.out = 'pending';\
             fetch('/api/data')\
               .then(function(r) { globalThis.meta = r.status + ' ' + r.ok + ' ' \
                   + r.statusText + ' ' + r.headers.get('content-type'); return r.json(); })\
               .then(function(d) { globalThis.out = d.n; })",
        )
        .unwrap();
    assert_eq!(
        engine.eval("out === 42").unwrap(),
        oasis_js::JsValue::Bool(true)
    );
    assert_eq!(js_str(&engine, "meta"), "200 true OK application/json");
    assert_eq!(
        engine.eval("fetch('/x') instanceof Promise").unwrap(),
        oasis_js::JsValue::Bool(true)
    );
}

#[test]
fn fetch_resolves_relative_urls_against_document() {
    let t = MockTransport::default();
    let engine = setup_page("https://example.com/app/page.html?q=1", None, Some(&t));
    engine
        .eval("fetch('data.json'); fetch('../up.txt'); fetch('/abs'); fetch('?x=2#frag')")
        .unwrap();
    assert_eq!(
        t.urls(),
        vec![
            "https://example.com/app/data.json",
            "https://example.com/up.txt",
            "https://example.com/abs",
            "https://example.com/app/page.html?x=2",
        ]
    );
}

#[test]
fn fetch_reports_real_status_and_rejects_network_errors() {
    let t = MockTransport::default();
    let engine = setup_page("https://example.com/", None, Some(&t));
    engine
        .eval(
            "fetch('/missing').then(function(r) { \
               globalThis.st = r.status + ':' + r.ok + ':' + r.statusText; })",
        )
        .unwrap();
    assert_eq!(js_str(&engine, "st"), "404:false:Not Found");
    engine
        .eval("fetch('ftp://example.com/f').catch(function(e) { globalThis.err = e.name; })")
        .unwrap();
    assert_eq!(js_str(&engine, "err"), "TypeError");
}

#[test]
fn fetch_blocks_loopback_and_private_targets_from_web_origin() {
    let t = MockTransport::default();
    let engine = setup_page("http://example.com/", None, Some(&t));
    engine
        .eval(
            "globalThis.errs = [];\
             function rec(p) { return p.then(function() { errs.push('ok'); }, \
               function(e) { errs.push(e.name); }); }\
             rec(fetch('http://127.0.0.1:7345/mcp', { method: 'POST', body: '{}', \
               headers: { 'Content-Type': 'application/json' } }));\
             rec(fetch('http://localhost:7345/mcp'));\
             rec(fetch('http://[::1]/'));\
             rec(fetch('http://2130706433/'));\
             rec(fetch('http://192.168.1.1/admin'));\
             rec(fetch('http://10.0.0.5/'));\
             rec(fetch('http://169.254.169.254/latest/meta-data'));\
             rec(fetch('http://[fd00::1]/'));",
        )
        .unwrap();
    assert_eq!(
        js_str(&engine, "errs.join(',')"),
        "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError"
    );
    assert!(t.calls.borrow().is_empty(), "no request may reach the wire");
}

#[test]
fn fetch_loopback_page_may_reach_loopback() {
    let t = MockTransport::default();
    t.respond("http://127.0.0.1:8080/api", 200, &[], "local");
    let engine = setup_page("http://127.0.0.1:8080/", None, Some(&t));
    engine
        .eval(
            "fetch('/api', { method: 'POST', body: 'x' })\
               .then(function(r) { return r.text(); })\
               .then(function(t) { globalThis.out = t; })",
        )
        .unwrap();
    assert_eq!(js_str(&engine, "out"), "local");
    let calls = t.calls.borrow();
    assert_eq!(calls[0].0, "POST");
    assert_eq!(calls[0].3.as_deref(), Some("x"));
}

#[test]
fn fetch_cross_origin_only_simple_get_with_cors() {
    let t = MockTransport::default();
    t.respond(
        "https://api.other.com/open",
        200,
        &[("Access-Control-Allow-Origin", "*")],
        "open",
    );
    t.respond("https://api.other.com/closed", 200, &[], "secret");
    let engine = setup_page("https://example.com/", None, Some(&t));
    engine
        .eval(
            "globalThis.r = [];\
             fetch('https://api.other.com/open').then(function(x) { return x.text(); })\
               .then(function(s) { r.push(s); }, function(e) { r.push('E1'); });\
             fetch('https://api.other.com/closed').then(function() { r.push('leak'); },\
               function(e) { r.push('cors'); });\
             fetch('https://api.other.com/open', { method: 'POST', body: 'x' })\
               .then(function() { r.push('bad'); }, function(e) { r.push('post'); });\
             fetch('https://api.other.com/open', { headers: { 'X-Token': 'a' } })\
               .then(function() { r.push('bad'); }, function(e) { r.push('hdr'); });",
        )
        .unwrap();
    let mut got: Vec<String> = js_str(&engine, "r.join(',')")
        .split(',')
        .map(String::from)
        .collect();
    got.sort();
    assert_eq!(got, vec!["cors", "hdr", "open", "post"]);
    // Only the two simple GETs went out, each carrying the page Origin.
    let calls = t.calls.borrow();
    assert_eq!(calls.len(), 2);
    for c in calls.iter() {
        assert_eq!(c.0, "GET");
        assert!(
            c.2.iter()
                .any(|(k, v)| k == "origin" && v == "https://example.com")
        );
    }
}

#[test]
fn fetch_same_origin_allows_any_method_and_drops_forbidden_headers() {
    let t = MockTransport::default();
    t.respond("https://example.com/api", 201, &[], "made");
    let engine = setup_page("https://example.com/", None, Some(&t));
    engine
        .eval(
            "fetch('/api', { method: 'put', body: 'b', \
               headers: { 'X-Custom': '1', 'Host': 'evil', 'Cookie': 'c=1' } })\
             .then(function(r) { globalThis.st = r.status; })",
        )
        .unwrap();
    assert_eq!(engine.eval("st").unwrap(), oasis_js::JsValue::Int(201));
    let calls = t.calls.borrow();
    assert_eq!(calls[0].0, "PUT");
    let names: Vec<&str> = calls[0].2.iter().map(|(k, _)| k.as_str()).collect();
    assert!(names.contains(&"x-custom"));
    assert!(!names.contains(&"host") && !names.contains(&"cookie"));
}

#[test]
fn storage_is_isolated_per_origin_and_persists_per_origin() {
    let store: SharedLocalStorage = Rc::default();
    let a = setup_page("https://a.example/page", Some(&store), None);
    a.eval("localStorage.setItem('k', 'from-a'); sessionStorage.setItem('s', 'sa')")
        .unwrap();
    let b = setup_page("https://b.example/", Some(&store), None);
    assert_eq!(
        b.eval("localStorage.getItem('k')").unwrap(),
        oasis_js::JsValue::Null
    );
    assert_eq!(
        b.eval("sessionStorage.getItem('s')").unwrap(),
        oasis_js::JsValue::Null
    );
    assert_eq!(
        b.eval("localStorage.length").unwrap(),
        oasis_js::JsValue::Int(0)
    );
    b.eval("localStorage.setItem('k', 'from-b'); localStorage.clear()")
        .unwrap();
    // Same origin (default port spelled out) sees a's data after navigation.
    let a2 = setup_page("https://A.example:443/other", Some(&store), None);
    assert_eq!(js_str(&a2, "localStorage.getItem('k')"), "from-a");
    assert_eq!(js_str(&a2, "sessionStorage.getItem('s')"), "sa");
    assert_eq!(js_str(&a2, "localStorage.key(0)"), "k");
    assert_eq!(
        a2.eval("localStorage.key(5)").unwrap(),
        oasis_js::JsValue::Null
    );
}

#[test]
fn storage_empty_string_value_is_not_null() {
    let engine = setup_page("https://example.com/", None, None);
    engine.eval("localStorage.setItem('e', '')").unwrap();
    assert_eq!(
        engine.eval("localStorage.getItem('e')").unwrap(),
        oasis_js::JsValue::String(String::new())
    );
    assert_eq!(
        engine.eval("localStorage.getItem('missing')").unwrap(),
        oasis_js::JsValue::Null
    );
}

#[test]
fn storage_quota_exceeded_throws_dom_exception() {
    let engine = setup_page("https://example.com/", None, None);
    let r = js_str(
        &engine,
        "var big = 'x'.repeat(3 * 1024 * 1024); var r;\
         localStorage.setItem('a', big);\
         try { localStorage.setItem('b', big); r = 'stored'; }\
         catch (e) { r = e.name + ':' + (e instanceof DOMException) + ':' + e.code; }\
         r + ':' + (localStorage.getItem('b') === null) + ':' + localStorage.length",
    );
    assert_eq!(r, "QuotaExceededError:true:22:true:1");
}

#[test]
fn net_class_and_ip_literal_parsing() {
    use fetch::{NetClass, classify_ip, parse_ip_host};
    let class = |h: &str| parse_ip_host(h).map(classify_ip);
    assert_eq!(class("127.0.0.1"), Some(NetClass::Loopback));
    assert_eq!(class("127.1"), Some(NetClass::Loopback));
    assert_eq!(class("0x7f.0.0.1"), Some(NetClass::Loopback));
    assert_eq!(class("0177.0.0.1"), Some(NetClass::Loopback));
    assert_eq!(class("0.0.0.0"), Some(NetClass::Loopback));
    assert_eq!(class("[::1]"), Some(NetClass::Loopback));
    assert_eq!(class("[::ffff:192.168.0.1]"), Some(NetClass::Private));
    assert_eq!(class("172.16.5.4"), Some(NetClass::Private));
    assert_eq!(class("[fe80::1]"), Some(NetClass::Private));
    assert_eq!(class("[fc00::1]"), Some(NetClass::Private));
    assert_eq!(class("8.8.8.8"), Some(NetClass::Public));
    assert_eq!(class("[2606:4700::1111]"), Some(NetClass::Public));
    assert_eq!(class("example.com"), None);
    assert_eq!(class("256.1.1.1"), None);
}

// ------------------------------------------------------------------
// DOM API completeness (wrapper identity, text nodes, tree mutation,
// events, lifecycle)
// ------------------------------------------------------------------

/// Parse `html` into a document and install the DOM bindings.
fn setup_html(html: &str) -> (JsEngine, SharedDoc) {
    use crate::html::tokenizer::Tokenizer;
    use crate::html::tree_builder::TreeBuilder;
    setup(TreeBuilder::build(Tokenizer::new(html).tokenize()))
}

/// Evaluate `expr` and assert it produces the string `want`.
fn check(engine: &JsEngine, expr: &str, want: &str) {
    assert_eq!(js_str(engine, expr), want, "expression: {expr}");
}

const TREE_HTML: &str = "<html><head><title>T</title></head><body>\
    <div id=\"outer\" class=\"outer\">\
      <p id=\"p\">a<b id=\"b\">b</b>c</p>\
      <ul id=\"list\"><li class=\"x\">1</li> <li class=\"x y\">2</li> <li>3</li></ul>\
    </div><section id=\"other\"></section></body></html>";

#[test]
fn wrappers_are_cached_per_node_id() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var b = document.getElementById('b'); var p = document.getElementById('p'); \
         [b.parentNode === p, p.parentNode === document.getElementById('outer'), \
          document.querySelector('#b') === b, p.children[0] === b, \
          new Element(b.__oasis_node_id) === b, document.body.parentNode === \
          document.documentElement, document.documentElement.parentNode === document, \
          p.firstChild === p.firstChild].join()",
        "true,true,true,true,true,true,true,true",
    );
    // Expando properties survive re-lookup.
    check(
        &engine,
        "document.getElementById('b').__mark = 7; document.querySelector('b').__mark",
        "7",
    );
}

#[test]
fn freed_nodes_are_evicted_from_wrapper_cache() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var outer = document.getElementById('outer'); var old = outer.firstElementChild; \
         var hits = 0; old.addEventListener('click', function() { hits++; }); \
         outer.innerHTML = '<i>x</i><i>y</i><i>z</i>'; \
         var fresh = outer.children; \
         fresh.forEach(function(n) { n.dispatchEvent(new Event('click')); }); \
         [fresh[0] !== old, fresh[0].tagName, hits, fresh[0] === outer.firstChild].join()",
        "true,I,0,true",
    );
}

#[test]
fn child_nodes_include_text_nodes() {
    let (engine, shared) = setup_html(TREE_HTML);
    check(
        &engine,
        "var p = document.getElementById('p'); var t = p.firstChild; \
         [p.childNodes.length, p.children.length, t.nodeType, t.nodeName, t.data, \
          t.nodeValue, t.length, t.nextSibling.tagName, p.lastChild.textContent, \
          t.nextSibling.nextSibling.previousSibling.id, t.parentNode === p].join()",
        "3,1,3,#text,a,a,1,B,c,b,true",
    );
    engine
        .eval("var p = document.getElementById('p'); p.firstChild.data = 'z'; p.lastChild.nodeValue = 'q'")
        .unwrap();
    let doc = shared.borrow();
    let p = doc.get_element_by_id("p").unwrap();
    assert_eq!(doc.text_content(p), "zbq");
}

#[test]
fn node_type_and_name_cover_every_kind() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "[document.nodeType, document.nodeName, document.body.nodeType, \
          document.body.nodeName, document.body.localName, \
          document.createComment('c').nodeType, document.createComment('c').nodeName, \
          document.createDocumentFragment().nodeType, \
          document.createDocumentFragment().nodeName, document.createTextNode('t').nodeName, \
          Node.ELEMENT_NODE, document.body.TEXT_NODE, \
          document.body instanceof Node, document.createTextNode('') instanceof Text, \
          document.body instanceof HTMLElement].join()",
        "9,#document,1,BODY,body,8,#comment,11,#document-fragment,#text,1,3,true,true,true",
    );
}

#[test]
fn closest_and_matches() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var b = document.getElementById('b'); \
         [b.closest('.outer').id, b.closest('b') === b, b.closest('section'), \
          b.matches('#p > b'), b.matches('i'), b.webkitMatchesSelector('b'), \
          (function() { try { b.matches('!!'); return 'no'; } \
                        catch (e) { return e.name; } })()].join()",
        "outer,true,,true,false,true,SyntaxError",
    );
}

#[test]
fn attribute_helpers_and_dataset() {
    let (engine, shared) = setup_html(
        "<html><body><div id=\"d\" data-foo-bar=\"1\" data-x=\"2\"></div></body></html>",
    );
    check(
        &engine,
        "var d = document.getElementById('d'); \
         [d.hasAttribute('data-x'), d.hasAttribute('nope'), d.toggleAttribute('hidden'), \
          d.hasAttribute('hidden'), d.toggleAttribute('hidden'), \
          d.toggleAttribute('open', true), d.toggleAttribute('open', true), \
          d.dataset.fooBar, d.dataset.missing, 'x' in d.dataset, \
          Object.keys(d.dataset).join('+'), d.getAttributeNames().length, \
          d.dataset === d.dataset].join()",
        "true,false,true,true,false,true,true,1,,true,fooBar+x,4,true",
    );
    engine
        .eval(
            "var d = document.getElementById('d'); d.dataset.newKey = 'v'; \
             delete d.dataset.x; d.className = 'a b';",
        )
        .unwrap();
    let doc = shared.borrow();
    let d = doc.element(doc.get_element_by_id("d").unwrap()).unwrap();
    assert_eq!(d.get_attribute("data-new-key"), Some("v"));
    assert_eq!(d.get_attribute("data-x"), None);
    assert_eq!(d.get_attribute("open"), Some(""));
    assert_eq!(d.get_attribute("class"), Some("a b"));
}

#[test]
fn element_sibling_and_child_navigation() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var ul = document.getElementById('list'); var f = ul.firstElementChild; \
         [ul.childNodes.length, ul.children.length, ul.childElementCount, \
          f.textContent, f.nextElementSibling.textContent, \
          f.nextSibling.nodeType, ul.lastElementChild.textContent, \
          ul.lastElementChild.previousElementSibling.textContent, \
          f.previousElementSibling, ul.children.item(1).textContent, \
          ul.children.item(9)].join()",
        "5,3,3,1,2,3,3,2,,2,",
    );
}

#[test]
fn remove_keeps_subtree_for_reinsertion() {
    let (engine, shared) = setup_html(TREE_HTML);
    check(
        &engine,
        "var p = document.getElementById('p'); p.remove(); \
         var gone = [p.isConnected, document.getElementById('p'), p.parentNode].join(); \
         document.getElementById('other').appendChild(p); \
         [gone, p.isConnected, p.textContent, document.getElementById('p') === p, \
          p.parentNode.id].join()",
        "false,,,true,abc,true,other",
    );
    let doc = shared.borrow();
    let other = doc.get_element_by_id("other").unwrap();
    assert_eq!(doc.text_content(other), "abc");
}

#[test]
fn append_prepend_before_after_replace_with() {
    let (engine, _doc) =
        setup_html("<html><body><div id=\"c\"><span id=\"s\">s</span></div></body></html>");
    check(
        &engine,
        "var c = document.getElementById('c'); var s = document.getElementById('s'); \
         c.append('x', document.createElement('i')); c.prepend('0'); \
         s.before('<'); s.after('>', document.createElement('b')); c.innerHTML",
        "0&lt;<span id=\"s\">s</span>&gt;<b></b>x<i></i>",
    );
    check(
        &engine,
        "var s = document.getElementById('s'); s.replaceWith('R', document.createElement('u')); \
         [document.getElementById('c').innerHTML, s.isConnected].join('|')",
        "0&lt;R<u></u>&gt;<b></b>x<i></i>|false",
    );
    check(
        &engine,
        "var c = document.getElementById('c'); c.replaceChildren('only'); c.innerHTML",
        "only",
    );
}

#[test]
fn clone_node_shallow_and_deep() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var p = document.getElementById('p'); var deep = p.cloneNode(true); \
         var shallow = p.cloneNode(); \
         [deep !== p, deep.id, deep.childNodes.length, deep.textContent, \
          shallow.childNodes.length, deep.isConnected, deep.parentNode, \
          document.getElementById('p') === p, \
          document.createTextNode('t').cloneNode().data].join()",
        "true,p,3,abc,0,false,,true,t",
    );
}

#[test]
fn replace_child_insert_before_and_hierarchy_errors() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var ul = document.getElementById('list'); var n = document.createElement('li'); \
         n.textContent = 'N'; var old = ul.firstElementChild; \
         var ret = ul.replaceChild(n, old); \
         var m = document.createElement('li'); m.textContent = 'M'; \
         ul.insertBefore(m, n); \
         var err1 = (function() { try { document.getElementById('b')\
           .appendChild(document.getElementById('outer')); return 'no'; } \
           catch (e) { return e.name; } })(); \
         var err2 = (function() { try { ul.removeChild(document.body); return 'no'; } \
           catch (e) { return e.name; } })(); \
         [ret === old, old.isConnected, ul.firstElementChild.textContent, \
          ul.children[1].textContent, err1, err2].join()",
        "true,false,M,N,HierarchyRequestError,NotFoundError",
    );
}

#[test]
fn insert_adjacent_html_and_element() {
    let (engine, _doc) =
        setup_html("<html><body><div id=\"w\"><p id=\"t\">t</p></div></body></html>");
    check(
        &engine,
        "var t = document.getElementById('t'); \
         t.insertAdjacentHTML('beforebegin', '<a>1</a>'); \
         t.insertAdjacentHTML('afterbegin', '<b>2</b>'); \
         t.insertAdjacentHTML('beforeend', '<i>3</i>'); \
         t.insertAdjacentHTML('afterend', '<u>4</u>'); \
         var em = document.createElement('em'); \
         var r = t.insertAdjacentElement('afterEnd', em); \
         t.insertAdjacentText('beforeEnd', '!'); \
         var err = (function() { try { t.insertAdjacentHTML('nowhere', 'x'); return 'no'; } \
           catch (e) { return e.name; } })(); \
         [document.getElementById('w').innerHTML, r === em, err].join('|')",
        "<a>1</a><p id=\"t\"><b>2</b>t<i>3</i>!</p><em></em><u>4</u>|true|SyntaxError",
    );
}

#[test]
fn outer_html_get_and_set() {
    let (engine, shared) =
        setup_html("<html><body><div id=\"w\"><p id=\"t\" class=\"k\">hi</p></div></body></html>");
    check(
        &engine,
        "document.getElementById('t').outerHTML",
        "<p id=\"t\" class=\"k\">hi</p>",
    );
    engine
        .eval("document.getElementById('t').outerHTML = '<span id=\"n\">new</span>!'")
        .unwrap();
    check(
        &engine,
        "[document.getElementById('w').innerHTML, document.getElementById('t')].join('|')",
        "<span id=\"n\">new</span>!|",
    );
    let doc = shared.borrow();
    assert!(doc.get_element_by_id("n").is_some());
}

#[test]
fn text_content_set_replaces_and_clears_children() {
    let (engine, shared) = setup_html(TREE_HTML);
    check(
        &engine,
        "var ul = document.getElementById('list'); ul.textContent = 'plain'; \
         var a = [ul.childNodes.length, ul.firstChild.nodeType].join(); \
         ul.textContent = ''; [a, ul.childNodes.length, ul.innerText].join('|')",
        "1,3|0|",
    );
    let doc = shared.borrow();
    let ul = doc.get_element_by_id("list").unwrap();
    assert!(doc.get(ul).children.is_empty());
}

#[test]
fn form_control_value_and_checked() {
    let (engine, shared) = setup_html(
        "<html><body><form>\
         <input id=\"i\" value=\"v0\"><input id=\"cb\" type=\"checkbox\">\
         <input id=\"r1\" type=\"radio\" name=\"g\" checked><input id=\"r2\" type=\"radio\" name=\"g\">\
         <select id=\"s\"><option value=\"a\">A</option><option selected>B</option></select>\
         <textarea id=\"ta\">txt</textarea></form></body></html>",
    );
    check(
        &engine,
        "var $ = function(id) { return document.getElementById(id); }; \
         var before = [$('i').value, $('cb').checked, $('cb').value, $('s').value, \
                       $('s').selectedIndex, $('ta').value, $('i').type].join(); \
         $('i').value = 'v1'; $('cb').checked = true; $('r2').checked = true; \
         $('s').value = 'a'; $('ta').value = 'new'; \
         [before, $('i').value, $('cb').checked, $('r1').checked, $('r2').checked, \
          $('s').value, $('s').selectedIndex, $('ta').value].join('|')",
        "v0,false,on,B,1,txt,text|v1|true|false|true|a|0|new",
    );
    let doc = shared.borrow();
    let i = doc.element(doc.get_element_by_id("i").unwrap()).unwrap();
    assert_eq!(i.get_attribute("value"), Some("v1"));
    let cb = doc.element(doc.get_element_by_id("cb").unwrap()).unwrap();
    assert_eq!(cb.get_attribute("checked"), Some(""));
}

#[test]
fn get_elements_by_class_and_tag_name() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "[document.getElementsByClassName('x').length, \
          document.getElementsByClassName('y x').length, \
          document.getElementsByTagName('li').length, \
          document.getElementsByTagName('LI')[2].textContent, \
          document.getElementById('outer').getElementsByTagName('*').length, \
          document.getElementById('list').getElementsByClassName('x')[1].textContent, \
          document.getElementsByTagName('title')[0].textContent].join()",
        "2,1,3,3,6,2,T",
    );
}

#[test]
fn document_accessors_and_factories() {
    let (engine, shared) = setup_html(TREE_HTML);
    check(
        &engine,
        "var frag = document.createDocumentFragment(); \
         frag.appendChild(document.createElement('em')); \
         frag.appendChild(document.createTextNode('tail')); \
         var other = document.getElementById('other'); other.appendChild(frag); \
         [document.documentElement.tagName, document.head.tagName, document.body.tagName, \
          document.readyState, frag.childNodes.length, other.innerHTML, \
          document.querySelector('title').textContent, \
          document.createElement('DIV').tagName, \
          document.createComment('x').data, document.defaultView === window].join()",
        "HTML,HEAD,BODY,loading,0,<em></em>tail,T,DIV,x,true",
    );
    let doc = shared.borrow();
    let other = doc.get_element_by_id("other").unwrap();
    assert_eq!(doc.text_content(other), "tail");
}

#[test]
fn lifecycle_fires_in_order_and_sets_ready_state() {
    let (engine, _doc) = setup_html(TREE_HTML);
    engine
        .eval(
            "var log = []; \
             document.addEventListener('readystatechange', function() { \
               log.push('rs:' + document.readyState); }); \
             document.addEventListener('DOMContentLoaded', function(e) { \
               log.push('dcl:doc:' + e.bubbles); }); \
             window.addEventListener('DOMContentLoaded', function() { log.push('dcl:win'); }); \
             window.addEventListener('load', function() { \
               log.push('load:' + document.readyState); }); \
             window.onload = function() { log.push('onload'); };",
        )
        .unwrap();
    fire_document_lifecycle(&engine);
    // A second call is a no-op.
    fire_document_lifecycle(&engine);
    check(
        &engine,
        "log.join(' ')",
        "rs:interactive dcl:doc:true dcl:win rs:complete load:complete onload",
    );
}

#[test]
fn body_onload_attribute_runs_on_window_load() {
    let (engine, shared) = setup_html(
        "<html><body onload=\"window.__loaded = (window.__loaded || 0) + 1\"></body></html>",
    );
    register_inline_handlers(&engine, &shared.borrow());
    fire_document_lifecycle(&engine);
    check(&engine, "String(window.__loaded)", "1");
}

#[test]
fn custom_event_bubbles_flag_and_default_prevented() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var b = document.getElementById('b'); var log = []; \
         document.getElementById('outer').addEventListener('ping', function(e) { \
           log.push('outer:' + e.detail.n + ':' + (e.target === b) + ':' + e.eventPhase); }); \
         b.addEventListener('ping', function(e) { log.push('b'); e.preventDefault(); }); \
         var r1 = b.dispatchEvent(new CustomEvent('ping', {detail: {n: 1}})); \
         var r2 = b.dispatchEvent(new CustomEvent('ping', \
           {detail: {n: 2}, bubbles: true, cancelable: true})); \
         var ce = new CustomEvent('x'); \
         [log.join(' '), r1, r2, ce.detail, ce instanceof Event].join('|')",
        "b b outer:2:true:3|true|false||true",
    );
}

#[test]
fn stop_immediate_propagation_and_once() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var b = document.getElementById('b'); var log = []; \
         document.getElementById('p').addEventListener('go', function() { log.push('p'); }); \
         b.addEventListener('go', function(e) { log.push('1'); \
           if (e.detail === 'imm') e.stopImmediatePropagation(); \
           else e.stopPropagation(); }); \
         b.addEventListener('go', function() { log.push('2'); }); \
         b.addEventListener('go', function() { log.push('once'); }, {once: true}); \
         b.dispatchEvent(new CustomEvent('go', {bubbles: true, detail: 'stop'})); \
         b.dispatchEvent(new CustomEvent('go', {bubbles: true, detail: 'imm'})); \
         b.dispatchEvent(new CustomEvent('go', {bubbles: true, detail: 'stop'})); \
         log.join(' ')",
        "1 2 once 1 1 2",
    );
}

#[test]
fn listener_exception_does_not_block_others_and_handler_props() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var b = document.getElementById('b'); var log = []; \
         b.addEventListener('click', function() { throw new Error('boom'); }); \
         b.addEventListener('click', function(e) { log.push(e instanceof MouseEvent); }); \
         b.onclick = function() { log.push('prop'); return false; }; \
         b.click(); \
         var prevented = __oasis_dispatch_with_bubbling(b.__oasis_node_id, 'click', null); \
         [log.join(' '), prevented].join('|')",
        "true prop true prop|true",
    );
}

#[test]
fn host_click_bubbles_to_document_and_window() {
    let (engine, _doc) = setup_html(TREE_HTML);
    check(
        &engine,
        "var log = []; \
         document.addEventListener('click', function(e) { log.push('doc:' + e.target.id); }); \
         window.addEventListener('click', function() { log.push('win'); }); \
         __oasis_dispatch_with_bubbling(document.getElementById('b').__oasis_node_id, \
           'click', {clientX: 3, clientY: 4}); \
         log.join(' ')",
        "doc:b win",
    );
}

#[test]
fn request_animation_frame_maps_to_timers() {
    let (engine, _doc) = setup_html(TREE_HTML);
    engine
        .eval(
            "var frames = []; var id1 = requestAnimationFrame(function(t) { \
               frames.push(typeof t); }); \
             var id2 = requestAnimationFrame(function() { frames.push('cancelled'); }); \
             cancelAnimationFrame(id2);",
        )
        .unwrap();
    check(&engine, "String(frames.length)", "0");
    engine.tick_timers(20.0);
    check(&engine, "frames.join() + '|' + (id1 > 0)", "number|true");
}

#[test]
fn class_list_full_api() {
    let (engine, shared) =
        setup_html("<html><body><div id=\"d\" class=\"a b\"></div></body></html>");
    check(
        &engine,
        "var cl = document.getElementById('d').classList; \
         cl.add('c', 'd'); cl.remove('a', 'zz'); \
         var r1 = cl.replace('b', 'B'); var r2 = cl.replace('nope', 'x'); \
         var t1 = cl.toggle('c', true); var t2 = cl.toggle('e', false); \
         var err = (function() { try { cl.add('has space'); return 'no'; } \
           catch (e) { return e.name; } })(); \
         [cl.length, cl.item(0), cl.item(7), r1, r2, t1, t2, cl.value, \
          Array.from(cl).join('+'), err, \
          cl === document.getElementById('d').classList].join()",
        "3,B,,true,false,true,false,B c d,B+c+d,InvalidCharacterError,true",
    );
    let doc = shared.borrow();
    let d = doc.element(doc.get_element_by_id("d").unwrap()).unwrap();
    assert_eq!(d.get_attribute("class"), Some("B c d"));
}

#[test]
fn reddit_shim_identity_check_skips_clicked_arrow() {
    // togglevote's `s !== el` guard relies on wrapper identity: the
    // clicked arrow must not be treated as the "opposite" arrow.
    let (engine, _doc) = setup_html(
        "<html><body><div class=\"midcol\">\
         <div id=\"up\" class=\"arrow up downmod\"></div>\
         <div id=\"down\" class=\"arrow downmod\"></div>\
         <div class=\"score\">5 points</div></div></body></html>",
    );
    install_site_compat_shims(&engine);
    check(
        &engine,
        "var up = document.getElementById('up'); togglevote(up, 1); \
         [up.className, document.getElementById('down').className, \
          document.querySelector('.score').textContent].join('|')",
        "arrow downmod upmod|arrow down|6 points",
    );
}
