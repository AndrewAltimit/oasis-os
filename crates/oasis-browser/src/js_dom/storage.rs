//! Page storage bindings: `document.cookie` and
//! `localStorage` / `sessionStorage`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use oasis_js::rquickjs::{Ctx, Function, Result as JsResult};

use super::SharedLocalStorage;

/// Install `__oasis_cookie_get` / `__oasis_cookie_set` backing
/// `document.cookie` (page-scoped, in-memory).
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
/// (kind 0) and `sessionStorage` (kind 1).
pub(super) fn install_storage_bindings(
    ctx: &Ctx<'_>,
    persistent_local_storage: Option<&SharedLocalStorage>,
) -> JsResult<()> {
    let globals = ctx.globals();

    // -- localStorage / sessionStorage -----------------------------------
    // localStorage uses a persistent backing store (shared across page
    // navigations) when provided, otherwise falls back to a fresh map.
    // sessionStorage always uses a fresh map (page-scoped).
    {
        let local_store = match persistent_local_storage {
            Some(store) => Rc::clone(store),
            None => Rc::new(RefCell::new(HashMap::<String, String>::new())),
        };
        let session_store = Rc::new(RefCell::new(HashMap::<String, String>::new()));

        let l1 = Rc::clone(&local_store);
        let ss1 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_get",
            Function::new(ctx.clone(), move |kind: i32, key: String| -> String {
                let store = if kind == 0 { &l1 } else { &ss1 };
                store.borrow().get(&key).cloned().unwrap_or_default()
            })?,
        )?;
        let l2 = Rc::clone(&local_store);
        let ss2 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_set",
            Function::new(ctx.clone(), move |kind: i32, key: String, value: String| {
                let store = if kind == 0 { &l2 } else { &ss2 };
                store.borrow_mut().insert(key, value);
            })?,
        )?;
        let l3 = Rc::clone(&local_store);
        let ss3 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_remove",
            Function::new(ctx.clone(), move |kind: i32, key: String| {
                let store = if kind == 0 { &l3 } else { &ss3 };
                store.borrow_mut().remove(&key);
            })?,
        )?;
        let l4 = Rc::clone(&local_store);
        let ss4 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_clear",
            Function::new(ctx.clone(), move |kind: i32| {
                let store = if kind == 0 { &l4 } else { &ss4 };
                store.borrow_mut().clear();
            })?,
        )?;
        let l5 = Rc::clone(&local_store);
        let ss5 = Rc::clone(&session_store);
        globals.set(
            "__oasis_storage_length",
            Function::new(ctx.clone(), move |kind: i32| -> i32 {
                let store = if kind == 0 { &l5 } else { &ss5 };
                store.borrow().len() as i32
            })?,
        )?;
    }

    Ok(())
}
