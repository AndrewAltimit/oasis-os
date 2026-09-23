//! Web Storage (`localStorage`) for the JavaScript engine.
//!
//! [`LocalStorage`] is a single storage area with a byte quota;
//! [`OriginStorage`] keys one area per origin so that hosts embedding
//! pages from several sites (the browser) can keep them isolated.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::rc::Rc;

use rquickjs::{Ctx, Function, Result as JsResult};

/// Default per-area quota: 5 MiB, matching mainstream browsers.
///
/// Usage is measured as the UTF-8 byte length of every key plus its value.
pub const DEFAULT_STORAGE_QUOTA_BYTES: usize = 5 * 1024 * 1024;

/// Error returned by [`LocalStorage::try_set_item`] when a write would
/// exceed the area's quota. Surfaced to JS as a `QuotaExceededError`
/// `DOMException`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaExceeded;

impl fmt::Display for QuotaExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("storage quota exceeded")
    }
}

impl std::error::Error for QuotaExceeded {}

/// In-memory implementation of the Web Storage API (`localStorage`).
///
/// Stores key-value pairs as `String -> String` in a `BTreeMap` so that
/// `key(index)` returns items in sorted order (matching most browsers).
/// Tracks its byte usage against a quota (see
/// [`DEFAULT_STORAGE_QUOTA_BYTES`]).
#[derive(Debug, Clone)]
pub struct LocalStorage {
    data: BTreeMap<String, String>,
    used_bytes: usize,
    quota_bytes: usize,
}

impl Default for LocalStorage {
    fn default() -> Self {
        Self::with_quota(DEFAULT_STORAGE_QUOTA_BYTES)
    }
}

impl LocalStorage {
    /// Create an empty `LocalStorage` with the default 5 MiB quota.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty `LocalStorage` with an explicit byte quota.
    pub fn with_quota(quota_bytes: usize) -> Self {
        Self {
            data: BTreeMap::new(),
            used_bytes: 0,
            quota_bytes,
        }
    }

    /// Retrieve the value associated with `key`, or `None`.
    ///
    /// A stored empty string is returned as `Some("")`.
    pub fn get_item(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    /// Whether `key` is present (even with an empty value).
    pub fn contains_key(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }

    /// Set the value for `key`, overwriting any previous value.
    ///
    /// Host-side (trusted) write: the quota is **not** enforced, although
    /// the usage counter is still updated. JS-originated writes go through
    /// [`try_set_item`](Self::try_set_item).
    pub fn set_item(&mut self, key: &str, value: &str) {
        let old = self.entry_bytes(key);
        self.data.insert(key.to_owned(), value.to_owned());
        self.used_bytes = self.used_bytes - old + key.len() + value.len();
    }

    /// Set the value for `key` unless doing so would push usage past the
    /// quota, in which case storage is left unchanged.
    ///
    /// Writes that do not grow usage (e.g. shrinking an existing value)
    /// always succeed, so an area that is over quota can still be trimmed.
    pub fn try_set_item(&mut self, key: &str, value: &str) -> Result<(), QuotaExceeded> {
        let old = self.entry_bytes(key);
        let new_used = self.used_bytes - old + key.len() + value.len();
        if new_used > self.quota_bytes && new_used > self.used_bytes {
            return Err(QuotaExceeded);
        }
        self.data.insert(key.to_owned(), value.to_owned());
        self.used_bytes = new_used;
        Ok(())
    }

    /// Remove the entry for `key` (no-op if absent).
    pub fn remove_item(&mut self, key: &str) {
        let old = self.entry_bytes(key);
        if self.data.remove(key).is_some() {
            self.used_bytes -= old;
        }
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.data.clear();
        self.used_bytes = 0;
    }

    /// Return the key at the given index in sorted order, or `None`.
    pub fn key(&self, index: usize) -> Option<String> {
        self.data.keys().nth(index).cloned()
    }

    /// Return the number of stored entries.
    pub fn length(&self) -> usize {
        self.data.len()
    }

    /// Bytes currently used (UTF-8 length of all keys and values).
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// The byte quota enforced by [`try_set_item`](Self::try_set_item).
    pub fn quota_bytes(&self) -> usize {
        self.quota_bytes
    }

    /// Bytes charged for the entry currently stored under `key`.
    fn entry_bytes(&self, key: &str) -> usize {
        self.data.get(key).map_or(0, |v| key.len() + v.len())
    }
}

/// A set of [`LocalStorage`] areas keyed by origin (e.g.
/// `"https://example.com"`), each with its own quota.
///
/// Used by `oasis-browser` so that pages from different sites never see
/// each other's `localStorage` / `sessionStorage`.
#[derive(Debug, Clone)]
pub struct OriginStorage {
    areas: HashMap<String, LocalStorage>,
    quota_bytes: usize,
}

impl Default for OriginStorage {
    fn default() -> Self {
        Self::with_quota(DEFAULT_STORAGE_QUOTA_BYTES)
    }
}

impl OriginStorage {
    /// Create an empty set of areas with the default per-origin quota.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty set of areas with an explicit per-origin quota.
    pub fn with_quota(quota_bytes: usize) -> Self {
        Self {
            areas: HashMap::new(),
            quota_bytes,
        }
    }

    /// The area for `origin`, if one has been created.
    pub fn area(&self, origin: &str) -> Option<&LocalStorage> {
        self.areas.get(origin)
    }

    /// The area for `origin`, created empty on first use.
    pub fn area_mut(&mut self, origin: &str) -> &mut LocalStorage {
        let quota = self.quota_bytes;
        self.areas
            .entry(origin.to_owned())
            .or_insert_with(|| LocalStorage::with_quota(quota))
    }

    /// Drop every area.
    pub fn clear_all(&mut self) {
        self.areas.clear();
    }

    /// Iterate over the origins that currently have an area.
    pub fn origins(&self) -> impl Iterator<Item = &str> {
        self.areas.keys().map(String::as_str)
    }
}

/// JS snippet defining a minimal `DOMException` when the engine lacks
/// one (QuickJS-NG does not ship it). Installed by
/// [`JsEngine::new`](crate::JsEngine::new) so storage wrappers can throw
/// `QuotaExceededError`.
pub const DOM_EXCEPTION_SHIM: &str = r#"
if (typeof globalThis.DOMException === 'undefined') {
    globalThis.DOMException = function DOMException(message, name) {
        var e = new Error(message === undefined ? '' : String(message));
        Object.setPrototypeOf(e, DOMException.prototype);
        e.name = name === undefined ? 'Error' : String(name);
        e.code = e.name === 'QuotaExceededError' ? 22
            : e.name === 'SecurityError' ? 18 : 0;
        return e;
    };
    DOMException.prototype = Object.create(Error.prototype);
    DOMException.prototype.constructor = DOMException;
}
"#;

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
            s.borrow().contains_key(&key)
        })?,
    )?;

    let s = Rc::clone(&storage);
    globals.set(
        "__oasis_storage_set",
        Function::new(ctx.clone(), move |key: String, value: String| -> bool {
            s.borrow_mut().try_set_item(&key, &value).is_ok()
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
    ctx.eval::<(), _>(DOM_EXCEPTION_SHIM)?;
    ctx.eval::<(), _>(
        br#"
globalThis.localStorage = {
    getItem: function(key) {
        if (!__oasis_storage_has(String(key))) return null;
        return __oasis_storage_get(String(key));
    },
    setItem: function(key, value) {
        if (!__oasis_storage_set(String(key), String(value))) {
            throw new DOMException(
                "Failed to execute 'setItem' on 'Storage': quota exceeded",
                'QuotaExceededError');
        }
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
    fn empty_string_value_is_not_null() {
        let mut s = LocalStorage::new();
        s.set_item("e", "");
        assert_eq!(s.get_item("e"), Some(String::new()));
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("localStorage.setItem('e', '')").unwrap();
        let val = engine.eval("localStorage.getItem('e')").unwrap();
        assert_eq!(val, crate::JsValue::String(String::new()));
    }

    #[test]
    fn quota_is_enforced_and_tracked() {
        let mut s = LocalStorage::with_quota(10);
        assert!(s.try_set_item("abc", "defg").is_ok()); // 7 bytes
        assert_eq!(s.used_bytes(), 7);
        assert_eq!(s.try_set_item("x", "yyyy"), Err(QuotaExceeded)); // would be 12
        assert!(s.get_item("x").is_none());
        // Overwrite that fits once the old value is discounted.
        assert!(s.try_set_item("abc", "1234567").is_ok()); // 10 bytes
        assert_eq!(s.used_bytes(), 10);
        s.remove_item("abc");
        assert_eq!(s.used_bytes(), 0);
    }

    #[test]
    fn origin_storage_areas_are_isolated() {
        let mut o = OriginStorage::new();
        o.area_mut("https://a.example").set_item("k", "a");
        o.area_mut("https://b.example").set_item("k", "b");
        assert_eq!(
            o.area("https://a.example").and_then(|a| a.get_item("k")),
            Some("a".into())
        );
        assert_eq!(
            o.area("https://b.example").and_then(|a| a.get_item("k")),
            Some("b".into())
        );
        assert!(o.area("https://c.example").is_none());
    }

    #[test]
    fn js_quota_exceeded_throws_dom_exception() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        *engine.local_storage().borrow_mut() = LocalStorage::with_quota(8);
        let val = engine
            .eval(
                "var r; try { localStorage.setItem('key', 'value-too-long'); r = 'stored'; } \
                 catch (e) { r = e.name + ':' + (e instanceof DOMException); } r",
            )
            .unwrap();
        assert_eq!(
            val,
            crate::JsValue::String("QuotaExceededError:true".into())
        );
    }

    #[test]
    fn js_values_coerced_to_string() {
        let engine = crate::JsEngine::new(8 * 1024 * 1024).unwrap();
        engine.eval("localStorage.setItem('n', 42)").unwrap();
        let val = engine.eval("localStorage.getItem('n')").unwrap();
        assert_eq!(val, crate::JsValue::String("42".into()));
    }
}
