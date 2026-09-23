//! Page storage bindings: `document.cookie` and
//! `localStorage` / `sessionStorage`.
//!
//! Web Storage is partitioned by origin: each page only ever sees the
//! [`oasis_js::LocalStorage`] area for its own `scheme://host[:port]`,
//! and every area carries the 5 MiB quota from
//! [`oasis_js::DEFAULT_STORAGE_QUOTA_BYTES`] (exceeding it throws a
//! `QuotaExceededError` `DOMException`). Pages with an opaque origin
//! (empty / `about:` URLs) get a private, page-lifetime store.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};
use oasis_js::{LocalStorage, OriginStorage};

use super::SharedLocalStorage;

/// Per-origin Web Storage for one browser widget (tab).
///
/// Both maps live as long as the widget. `sessionStorage` therefore
/// survives same-tab navigations (as on the web); `localStorage` is
/// in-memory too, so persisting it across restarts is the host's job.
#[derive(Debug, Clone, Default)]
pub struct WebStorage {
    /// `localStorage` areas keyed by origin.
    pub local: OriginStorage,
    /// `sessionStorage` areas keyed by origin.
    pub session: OriginStorage,
}

impl WebStorage {
    /// The storage area for `origin`; `kind` 0 = local, 1 = session.
    fn area_mut(&mut self, kind: i32, origin: &str) -> &mut LocalStorage {
        if kind == 0 {
            self.local.area_mut(origin)
        } else {
            self.session.area_mut(origin)
        }
    }

    fn area(&self, kind: i32, origin: &str) -> Option<&LocalStorage> {
        if kind == 0 {
            self.local.area(origin)
        } else {
            self.session.area(origin)
        }
    }
}

/// Install `__oasis_cookie_get` / `__oasis_cookie_set` backing
/// `document.cookie`.
///
/// The jar is created fresh for every page load and never shared, so
/// script-set cookies cannot leak across origins (they are also not sent
/// on network requests; HTTP `Set-Cookie` handling lives in the loader's
/// per-domain `CookieJar`).
pub(super) fn install_cookie_bindings(ctx: &Ctx<'_>) -> JsResult<()> {
    let globals = ctx.globals();

    // -- __oasis_cookie_get() / __oasis_cookie_set(raw) ------------------
    {
        let cookie_map: Rc<RefCell<HashMap<String, String>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let cm1 = Rc::clone(&cookie_map);
        globals.set(
            "__oasis_cookie_get",
            Function::new(ctx.clone(), move || -> String {
                let map = cm1.borrow();
                let mut pairs: Vec<String> = map.iter().map(|(k, v)| format!("{k}={v}")).collect();
                pairs.sort();
                pairs.join("; ")
            })?,
        )?;
        let cm2 = Rc::clone(&cookie_map);
        globals.set(
            "__oasis_cookie_set",
            Function::new(ctx.clone(), move |raw: String| {
                // Parse "name=value; path=/; ..." — only extract the first name=value.
                if let Some(pair) = raw.split(';').next() {
                    let pair = pair.trim();
                    if let Some(eq) = pair.find('=') {
                        let name = pair[..eq].trim().to_string();
                        let value = pair[eq + 1..].trim().to_string();
                        if !name.is_empty() {
                            cm2.borrow_mut().insert(name, value);
                        }
                    }
                }
            })?,
        )?;
    }

    Ok(())
}

/// Install the `__oasis_storage_*` helpers backing `localStorage`
/// (kind 0) and `sessionStorage` (kind 1) for a page at `page_url`.
///
/// `persistent` is the widget-lifetime [`WebStorage`]; when `None` (tests,
/// one-off contexts) a fresh store is used.
pub(super) fn install_storage_bindings(
    ctx: &Ctx<'_>,
    page_url: &str,
    persistent: Option<&SharedLocalStorage>,
) -> JsResult<()> {
    let globals = ctx.globals();

    let origin = crate::loader::Url::parse(page_url)
        .as_ref()
        .and_then(super::fetch::origin_key);
    let (store, origin): (SharedLocalStorage, String) = match (origin, persistent) {
        (Some(o), Some(p)) => (Rc::clone(p), o),
        (Some(o), None) => (Rc::default(), o),
        // Opaque origin: private store, never shared with other pages.
        (None, _) => (Rc::default(), "null".to_string()),
    };

    let (s, o) = (Rc::clone(&store), origin.clone());
    globals.set(
        "__oasis_storage_get",
        Function::new(
            ctx.clone(),
            move |kind: i32, key: String| -> Option<String> {
                s.borrow().area(kind, &o).and_then(|a| a.get_item(&key))
            },
        )?,
    )?;
    let (s, o) = (Rc::clone(&store), origin.clone());
    globals.set(
        "__oasis_storage_set",
        Function::new(
            ctx.clone(),
            move |kind: i32, key: String, value: String| -> bool {
                s.borrow_mut()
                    .area_mut(kind, &o)
                    .try_set_item(&key, &value)
                    .is_ok()
            },
        )?,
    )?;
    let (s, o) = (Rc::clone(&store), origin.clone());
    globals.set(
        "__oasis_storage_remove",
        Function::new(ctx.clone(), move |kind: i32, key: String| {
            s.borrow_mut().area_mut(kind, &o).remove_item(&key);
        })?,
    )?;
    let (s, o) = (Rc::clone(&store), origin.clone());
    globals.set(
        "__oasis_storage_clear",
        Function::new(ctx.clone(), move |kind: i32| {
            s.borrow_mut().area_mut(kind, &o).clear();
        })?,
    )?;
    let (s, o) = (Rc::clone(&store), origin.clone());
    globals.set(
        "__oasis_storage_key",
        Function::new(
            ctx.clone(),
            move |kind: i32, index: i32| -> Option<String> {
                let i = usize::try_from(index).ok()?;
                s.borrow().area(kind, &o).and_then(|a| a.key(i))
            },
        )?,
    )?;
    let (s, o) = (store, origin);
    globals.set(
        "__oasis_storage_length",
        Function::new(ctx.clone(), move |kind: i32| -> i32 {
            s.borrow()
                .area(kind, &o)
                .map_or(0, |a| i32::try_from(a.length()).unwrap_or(i32::MAX))
        })?,
    )?;

    Ok(())
}
