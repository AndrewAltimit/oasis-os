//! DOM bindings for JavaScript via rquickjs.
//!
//! This module lives in `oasis-browser` (not `oasis-js`) so that it can
//! directly access `Document` and other DOM types without creating a
//! circular dependency.  It uses `oasis_js::rquickjs` for the JS FFI
//! types.
//!
//! ## Design
//!
//! Rust-side functions (`__oasis_*`) operate on node-id integers and
//! return only primitives (`String`, `i32`, `Vec<i32>`).  This avoids
//! rquickjs lifetime issues with closures returning `Object<'js>`.
//!
//! A JavaScript snippet (installed via `ctx.eval`) defines the
//! `Element` constructor and `document` global, bridging the low-level
//! Rust helpers into the familiar DOM API.

use std::cell::RefCell;
use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

use crate::html::dom::{Document, ElementData, NodeId, NodeKind, TagName};

/// Shared, interior-mutable document used during JS execution.
pub type SharedDoc = Rc<RefCell<Document>>;

/// Sentinel returned when a DOM lookup produces no result.
const NO_NODE: i32 = -1;

/// Install the `document` global and `Element` prototype into the JS
/// context, backed by the given shared `Document`.
pub fn install_document_global(ctx: &Ctx<'_>, doc: &SharedDoc) -> JsResult<()> {
    let globals = ctx.globals();

    // -- __oasis_tagname(nid) -> String --------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_tagname",
            Function::new(ctx.clone(), move |nid: i32| -> String {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return String::new();
                }
                match &doc.nodes[id].kind {
                    NodeKind::Element(e) => e.tag.as_str().to_ascii_uppercase(),
                    NodeKind::Text(_) => "#text".into(),
                    _ => String::new(),
                }
            })?,
        )?;
    }

    // -- __oasis_getattr(nid, name) -> String|"" ----------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_getattr",
            Function::new(
                ctx.clone(),
                move |nid: i32, name: String| -> Option<String> {
                    let doc = d.borrow();
                    let id = nid as NodeId;
                    if id >= doc.nodes.len() {
                        return None;
                    }
                    match &doc.nodes[id].kind {
                        NodeKind::Element(e) => e.get_attribute(&name).map(String::from),
                        _ => None,
                    }
                },
            )?,
        )?;
    }

    // -- __oasis_setattr(nid, name, value) ----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_setattr",
            Function::new(ctx.clone(), move |nid: i32, name: String, value: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len()
                    && let NodeKind::Element(ref mut e) = doc.nodes[id].kind
                {
                    e.set_attribute(&name, &value);
                }
            })?,
        )?;
    }

    // -- __oasis_rmattr(nid, name) -> bool ----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_rmattr",
            Function::new(ctx.clone(), move |nid: i32, name: String| -> bool {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len()
                    && let NodeKind::Element(ref mut e) = doc.nodes[id].kind
                {
                    return e.remove_attribute(&name);
                }
                false
            })?,
        )?;
    }

    // -- __oasis_text(nid) -> String ----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_text",
            Function::new(ctx.clone(), move |nid: i32| -> String {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return String::new();
                }
                doc.text_content(id)
            })?,
        )?;
    }

    // -- __oasis_settext(nid, text) -----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_settext",
            Function::new(ctx.clone(), move |nid: i32, text: String| {
                let mut doc = d.borrow_mut();
                let id = nid as NodeId;
                if id < doc.nodes.len() {
                    doc.set_text_content(id, &text);
                }
            })?,
        )?;
    }

    // -- __oasis_children(nid) -> Vec<i32> ----------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_children",
            Function::new(ctx.clone(), move |nid: i32| -> Vec<i32> {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return Vec::new();
                }
                doc.nodes[id]
                    .children
                    .iter()
                    .copied()
                    .filter(|&cid| matches!(doc.nodes[cid].kind, NodeKind::Element(_)))
                    .map(|cid| cid as i32)
                    .collect()
            })?,
        )?;
    }

    // -- __oasis_parent(nid) -> i32 -----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_parent",
            Function::new(ctx.clone(), move |nid: i32| -> i32 {
                let doc = d.borrow();
                let id = nid as NodeId;
                if id >= doc.nodes.len() {
                    return NO_NODE;
                }
                doc.nodes[id]
                    .parent
                    .filter(|&pid| matches!(doc.nodes[pid].kind, NodeKind::Element(_)))
                    .map_or(NO_NODE, |pid| pid as i32)
            })?,
        )?;
    }

    // -- __oasis_getbyid(id) -> i32 -----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_getbyid",
            Function::new(ctx.clone(), move |id: String| -> i32 {
                d.borrow()
                    .get_element_by_id(&id)
                    .map_or(NO_NODE, |nid| nid as i32)
            })?,
        )?;
    }

    // -- __oasis_create(tag) -> i32 -----------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_create",
            Function::new(ctx.clone(), move |tag: String| -> i32 {
                let mut doc = d.borrow_mut();
                let data = ElementData::new(TagName::from_str(&tag));
                doc.add_node(NodeKind::Element(data)) as i32
            })?,
        )?;
    }

    // -- __oasis_createtext(text) -> i32 ------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_createtext",
            Function::new(ctx.clone(), move |text: String| -> i32 {
                let mut doc = d.borrow_mut();
                doc.add_node(NodeKind::Text(text)) as i32
            })?,
        )?;
    }

    // -- __oasis_append(parent_nid, child_nid) ------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_append",
            Function::new(ctx.clone(), move |parent_nid: i32, child_nid: i32| {
                let mut doc = d.borrow_mut();
                let pid = parent_nid as NodeId;
                let cid = child_nid as NodeId;
                if pid < doc.nodes.len() && cid < doc.nodes.len() {
                    doc.remove_child(cid);
                    doc.append_child(pid, cid);
                }
            })?,
        )?;
    }

    // -- __oasis_body() -> i32 ----------------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_body",
            Function::new(ctx.clone(), move || -> i32 {
                d.borrow().body().map_or(NO_NODE, |nid| nid as i32)
            })?,
        )?;
    }

    // -- __oasis_title() -> String ------------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_title",
            Function::new(ctx.clone(), move || -> String {
                d.borrow().title().unwrap_or_default()
            })?,
        )?;
    }

    // -- __oasis_settitle(text) ---------------------------------------
    {
        let d = Rc::clone(doc);
        globals.set(
            "__oasis_settitle",
            Function::new(ctx.clone(), move |val: String| {
                let mut doc = d.borrow_mut();
                if let Some(tid) = doc.title_element() {
                    doc.set_text_content(tid, &val);
                }
            })?,
        )?;
    }

    // -- JavaScript-side Element class + document global ---------------
    let _: () = ctx.eval(JS_DOM_BOOTSTRAP)?;

    Ok(())
}

/// JavaScript code that defines the `Element` wrapper and `document`
/// global using the `__oasis_*` Rust-backed helper functions.
const JS_DOM_BOOTSTRAP: &str = r#"
(function() {
  "use strict";

  function Element(nid) {
    this.__oasis_node_id = nid;
  }

  Object.defineProperties(Element.prototype, {
    tagName: {
      get: function() { return __oasis_tagname(this.__oasis_node_id); },
      enumerable: true
    },
    id: {
      get: function() { return __oasis_getattr(this.__oasis_node_id, "id") || ""; },
      set: function(v) {
        if (v) __oasis_setattr(this.__oasis_node_id, "id", v);
        else __oasis_rmattr(this.__oasis_node_id, "id");
      },
      enumerable: true
    },
    textContent: {
      get: function() { return __oasis_text(this.__oasis_node_id); },
      set: function(v) { __oasis_settext(this.__oasis_node_id, String(v)); },
      enumerable: true
    },
    children: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        var result = [];
        for (var i = 0; i < ids.length; i++) result.push(new Element(ids[i]));
        return result;
      },
      enumerable: true
    },
    parentElement: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        return pid >= 0 ? new Element(pid) : null;
      },
      enumerable: true
    }
  });

  Element.prototype.getAttribute = function(name) {
    var v = __oasis_getattr(this.__oasis_node_id, name);
    return v === undefined ? null : v;
  };
  Element.prototype.setAttribute = function(name, value) {
    __oasis_setattr(this.__oasis_node_id, name, String(value));
  };
  Element.prototype.removeAttribute = function(name) {
    __oasis_rmattr(this.__oasis_node_id, name);
  };
  Element.prototype.appendChild = function(child) {
    __oasis_append(this.__oasis_node_id, child.__oasis_node_id);
    return child;
  };

  var document = {
    getElementById: function(id) {
      var nid = __oasis_getbyid(id);
      return nid >= 0 ? new Element(nid) : null;
    },
    createElement: function(tag) {
      return new Element(__oasis_create(tag));
    },
    createTextNode: function(text) {
      return new Element(__oasis_createtext(String(text)));
    }
  };

  Object.defineProperties(document, {
    body: {
      get: function() {
        var nid = __oasis_body();
        return nid >= 0 ? new Element(nid) : null;
      },
      enumerable: true
    },
    title: {
      get: function() { return __oasis_title(); },
      set: function(v) { __oasis_settitle(String(v)); },
      enumerable: true
    }
  });

  globalThis.document = document;
  globalThis.Element = Element;
})();
"#;

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
