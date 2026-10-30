//! Build script: defines the `native_net` cfg alias.
//!
//! `native_net` is set when the crate is built for a target with the
//! desktop networking stack: the background I/O thread, the std TCP /
//! rustls HTTP client, the cookie jar and the DNS cache. That is every
//! target except `wasm32` (no sockets) and the PSP build (`psp` feature,
//! which uses `loader/http_psp.rs`). Write `#[cfg(native_net)]` instead
//! of `#[cfg(not(any(target_arch = "wasm32", feature = "psp")))]`.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(native_net)");
    println!("cargo::rerun-if-changed=build.rs");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|arch| arch == "wasm32");
    let psp = std::env::var_os("CARGO_FEATURE_PSP").is_some();
    if !wasm && !psp {
        println!("cargo::rustc-cfg=native_net");
    }
}
