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
    let main_children: Vec<_> = doc.get(main).children.iter().copied().collect();
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
