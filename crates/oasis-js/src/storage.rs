use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use rquickjs::{Ctx, Function, Result as JsResult};

/// In-memory implementation of the Web Storage API (`localStorage`).
///
/// Stores key-value pairs as `String -> String` in a `BTreeMap` so that
/// `key(index)` returns items in sorted order (matching most browsers).
#[derive(Debug, Clone, Default)]
pub struct LocalStorage {
    data: BTreeMap<String, String>,
}

impl LocalStorage {
    /// Create an empty `LocalStorage`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieve the value associated with `key`, or `None`.
    pub fn get_item(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    /// Set the value for `key`, overwriting any previous value.
    pub fn set_item(&mut self, key: &str, value: &str) {
        self.data.insert(key.to_owned(), value.to_owned());
    }

    /// Remove the entry for `key` (no-op if absent).
    pub fn remove_item(&mut self, key: &str) {
        self.data.remove(key);
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Return the key at the given index in sorted order, or `None`.
    pub fn key(&self, index: usize) -> Option<String> {
        self.data.keys().nth(index).cloned()
    }

    /// Return the number of stored entries.
    pub fn length(&self) -> usize {
        self.data.len()
    }
}

/// Shared, interior-mutable `LocalStorage` for use from JS closures.
pub(crate) type SharedStorage = Rc<RefCell<LocalStorage>>;

/// Install the `localStorage` global object into the given JS context.
pub(crate) fn install(ctx: &Ctx<'_>, storage: SharedStorage) -> JsResult<()> {
    let globals = ctx.globals();

    // -- Low-level Rust helpers (primitives only) -----------------------

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_get",
        Function::new(ctx.clone(), move |key: String| -> String {
            s.borrow().get_item(&key).unwrap_or_default()
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_has",
        Function::new(ctx.clone(), move |key: String| -> bool {
            s.borrow().data.contains_key(&key)
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_set",
        Function::new(ctx.clone(), move |key: String, value: String| {
            s.borrow_mut().set_item(&key, &value);
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_remove",
        Function::new(ctx.clone(), move |key: String| {
            s.borrow_mut().remove_item(&key);
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_clear",
        Function::new(ctx.clone(), move || {
            s.borrow_mut().clear();
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_key",
        Function::new(ctx.clone(), move |index: usize| -> String {
            s.borrow().key(index).unwrap_or_default()
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_key_exists",
        Function::new(ctx.clone(), move |index: usize| -> bool {
            s.borrow().key(index).is_some()
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_length",
        Function::new(ctx.clone(), move || -> usize { s.borrow().length() })?,
    )?;

    // -- JS wrapper that builds the localStorage object -----------------
    ctx.eval::<(), _>(
        br#"
globalThis.localStorage = {
    getItem: function(key) {
        if (!__oasis_storage_has(String(key))) return null;
        return __oasis_storage_get(String(key));
    },
    setItem: function(key, value) {
        __oasis_storage_set(String(key), String(value));
    },
    removeItem: function(key) {
        __oasis_storage_remove(String(key));
    },
    clear: function() {
        __oasis_storage_clear();
    },
    key: function(index) {
        var i = Number(index) | 0;
        if (!__oasis_storage_key_exists(i)) return null;
        return __oasis_storage_key(i);
    },
    get length() {
        return __oasis_storage_length();
    }
};
"#,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Unit tests for LocalStorage struct -----------------------------

    #[test]
    fn new_storage_is_empty() {
        let s = LocalStorage::new();
        assert_eq!(s.length(), 0);
        assert!(s.get_item("any").is_none());
    }

    #[test]
    fn set_and_get() {
        let mut s = LocalStorage::new();
        s.set_item("color", "blue");
        assert_eq!(s.get_item("color"), Some("blue".into()));
    }

    #[test]
    fn overwrite_value() {
        let mut s = LocalStorage::new();
        s.set_item("k", "v1");
        s.set_item("k", "v2");
        assert_eq!(s.get_item("k"), Some("v2".into()));
        assert_eq!(s.length(), 1);
    }

    #[test]
    fn remove_item() {
        let mut s = LocalStorage::new();
        s.set_item("k", "v");
        s.remove_item("k");
        assert!(s.get_item("k").is_none());
        assert_eq!(s.length(), 0);
    }

    #[test]
    fn remove_missing_is_noop() {
        let mut s = LocalStorage::new();
        s.remove_item("nope"); // should not panic
        assert_eq!(s.length(), 0);
    }

    #[test]
    fn clear() {
        let mut s = LocalStorage::new();
        s.set_item("a", "1");
        s.set_item("b", "2");
        s.clear();
        assert_eq!(s.length(), 0);
        assert!(s.get_item("a").is_none());
    }

    #[test]
    fn key_by_index() {
        let mut s = LocalStorage::new();
        s.set_item("banana", "yellow");
        s.set_item("apple", "red");
        // BTreeMap sorted order: apple, banana
        assert_eq!(s.key(0), Some("apple".into()));
        assert_eq!(s.key(1), Some("banana".into()));
        assert_eq!(s.key(2), None);
    }

    #[test]
    fn length_tracks_entries() {
        let mut s = LocalStorage::new();
        assert_eq!(s.length(), 0);
        s.set_item("a", "1");
        assert_eq!(s.length(), 1);
        s.set_item("b", "2");
        assert_eq!(s.length(), 2);
        s.remove_item("a");
        assert_eq!(s.length(), 1);
    }

    // -- Integration tests via JsEngine --------------------------------

    #[test]
    fn js_set_and_get() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("localStorage.setItem('foo', 'bar')").unwrap();
        let val = engine.eval("localStorage.getItem('foo')").unwrap();
        assert_eq!(val, crate::JsValue::String("bar".into()));
    }

    #[test]
    fn js_get_missing_returns_null() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        let val = engine.eval("localStorage.getItem('missing')").unwrap();
        assert_eq!(val, crate::JsValue::Null);
    }

    #[test]
    fn js_remove_item() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval("localStorage.setItem('x', '1'); localStorage.removeItem('x')")
            .unwrap();
        let val = engine.eval("localStorage.getItem('x')").unwrap();
        assert_eq!(val, crate::JsValue::Null);
    }

    #[test]
    fn js_clear() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "localStorage.setItem('a', '1'); \
                 localStorage.setItem('b', '2'); \
                 localStorage.clear()",
            )
            .unwrap();
        let val = engine.eval("localStorage.length").unwrap();
        assert_eq!(val, crate::JsValue::Int(0));
    }

    #[test]
    fn js_length() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "localStorage.setItem('a', '1'); \
                 localStorage.setItem('b', '2')",
            )
            .unwrap();
        let val = engine.eval("localStorage.length").unwrap();
        assert_eq!(val, crate::JsValue::Int(2));
    }

    #[test]
    fn js_key_by_index() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine
            .eval(
                "localStorage.setItem('banana', 'y'); \
                 localStorage.setItem('apple', 'r')",
            )
            .unwrap();
        // Sorted: apple=0, banana=1
        let k0 = engine.eval("localStorage.key(0)").unwrap();
        assert_eq!(k0, crate::JsValue::String("apple".into()));
        let k1 = engine.eval("localStorage.key(1)").unwrap();
        assert_eq!(k1, crate::JsValue::String("banana".into()));
        let k2 = engine.eval("localStorage.key(2)").unwrap();
        assert_eq!(k2, crate::JsValue::Null);
    }

    #[test]
    fn js_values_coerced_to_string() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("localStorage.setItem('n', 42)").unwrap();
        let val = engine.eval("localStorage.getItem('n')").unwrap();
        assert_eq!(val, crate::JsValue::String("42".into()));
    }
}
