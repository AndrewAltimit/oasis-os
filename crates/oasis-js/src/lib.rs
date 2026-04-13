//! JavaScript engines for OASIS_OS.
//!
//! This crate wraps two pluggable JS backends behind a shared
//! [`JsValue`] / [`JsError`] surface, gated by mutually-optional
//! feature flags:
//!
//! - `JsEngine` — the default, built on QuickJS-NG via `rquickjs`.
//!   Ships the full browser-facing API: `console`, `fetch`,
//!   `localStorage`, timers, retained event dispatch, and a raw
//!   `with_context` escape hatch that `oasis-browser`'s `js_dom.rs`
//!   uses to install DOM bindings. Enabled by the `rquickjs-engine`
//!   feature (on by default), which depends on `rquickjs` compiling
//!   its C source through the `cc` crate.
//!
//! - `BoaJsEngine` — a minimal, pure-Rust alternative built on
//!   `boa_engine`. Exposes just `new` and `eval` returning the same
//!   [`JsValue`] enum. No DOM glue, no timer queue, no shared state.
//!   Used on `mipsel-sony-psp`, where compiling the rquickjs C
//!   sources requires a pspdev cross-toolchain we don't have.
//!   Enabled by the `boa` feature — not linked as an intra-doc item
//!   here because it only exists when that feature is on.
//!
//! The two features are **mutually optional** — most consumers enable
//! exactly one. Desktop, WASM, and UE5 take the default
//! (`rquickjs-engine`); PSP uses
//! `default-features = false, features = ["boa"]`. Enabling both at
//! once is supported for host-side testing but doubles the compile
//! footprint.
//!
//! ## Why two backends?
//!
//! The short version: `rquickjs` needs `cc` to compile QuickJS's C
//! source, and the `mipsel-sony-psp` target has no installed C
//! cross-compiler. `boa_engine` is pure safe Rust with no native
//! dependencies, so it compiles for PSP the moment `std::collections`,
//! `std::time::Instant`, and the global allocator work on that
//! target (verified on real hardware via the rust-psp branch
//! `fix/psp-hardware-std-overlay-alignment-and-time`). See
//! `docs/browser-backlog.md` §"PSP JavaScript integration" for the
//! full rationale and trade-offs.

// Engine-agnostic value + error types. Always compiled regardless of
// which backend(s) are enabled — the rquickjs and boa engines both
// return these types from their `eval` methods.
mod types;
pub use types::{JsError, JsValue};

// rquickjs-backed `JsEngine` and its glue (console, fetch, storage,
// timers). Gated behind the `rquickjs-engine` feature so PSP builds
// can omit the C-source rquickjs dependency entirely.
#[cfg(feature = "rquickjs-engine")]
mod console;
#[cfg(feature = "rquickjs-engine")]
mod engine;
#[cfg(feature = "rquickjs-engine")]
pub mod fetch;
#[cfg(feature = "rquickjs-engine")]
mod storage;
#[cfg(feature = "rquickjs-engine")]
mod timers;

#[cfg(feature = "rquickjs-engine")]
pub use console::{ConsoleEntry, ConsoleLevel};
#[cfg(feature = "rquickjs-engine")]
pub use engine::JsEngine;
#[cfg(feature = "rquickjs-engine")]
pub use fetch::{FetchHandler, FetchRequest, FetchResponse, MockFetchHandler};
#[cfg(feature = "rquickjs-engine")]
pub use rquickjs;
#[cfg(feature = "rquickjs-engine")]
pub use storage::LocalStorage;
#[cfg(feature = "rquickjs-engine")]
pub use timers::TimerQueue;

// Pure-Rust boa-backed `BoaJsEngine` for targets that can't compile
// the rquickjs C sources (PSP). See `boa_backend.rs` for the API
// surface and the rationale behind the smaller method set.
#[cfg(feature = "boa")]
mod boa_backend;
#[cfg(feature = "boa")]
pub use boa_backend::BoaJsEngine;
