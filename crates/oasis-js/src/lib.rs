//! JavaScript engine for OASIS_OS, built on QuickJS-NG via `rquickjs`.
//!
//! The crate exposes a single engine — [`JsEngine`] — plus the
//! engine-agnostic [`JsValue`] / [`JsError`] types returned from
//! `eval`. The engine ships the full browser-facing API that
//! `oasis-browser` needs: `console`, `fetch`, `localStorage`, timers,
//! retained event dispatch, and a raw `with_context` escape hatch that
//! `oasis-browser`'s `js_dom.rs` uses to install DOM bindings.
//!
//! The same engine now runs on every supported target, including PSP.
//! On `mipsel-sony-psp` the QuickJS C sources are compiled through
//! pspdev's `psp-gcc`; the build wiring lives in
//! `crates/oasis-backend-psp/.cargo/config.toml`
//! (`CC_mipsel_sony_psp` / `AR_mipsel_sony_psp` / `PSPDEV`). No
//! alternative pure-Rust interpreter is shipped — the earlier
//! `BoaJsEngine` fallback has been removed now that the cross
//! toolchain is available.

mod types;
pub use types::{JsError, JsValue};

mod console;
mod engine;
pub mod fetch;
mod storage;
mod timers;

pub use console::{ConsoleEntry, ConsoleLevel};
pub use engine::JsEngine;
pub use fetch::{FetchHandler, FetchRequest, FetchResponse, MockFetchHandler};
pub use rquickjs;
pub use storage::LocalStorage;
pub use timers::TimerQueue;
