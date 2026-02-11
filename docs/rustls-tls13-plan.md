# Rustls TLS 1.3 Integration Plan

## Status Quo

OASIS OS has **zero TLS support**. The browser's HTTP client (`oasis-core/src/browser/loader/http.rs`)
operates over plain `std::net::TcpStream` and explicitly rejects HTTPS with an error page. The Gemini
protocol module (`browser/gemini/`) defines request/response parsing but has no network fetcher -- and
Gemini mandates TLS. The remote terminal (`net/`) also runs over unencrypted TCP.

Three networking implementations exist, all plain TCP:

| Backend | Implementation | File |
|---------|---------------|------|
| Desktop/SDL | `std::net::TcpStream` | `oasis-core/src/net/std_backend.rs` |
| PSP | Raw `sceNetInet*` syscalls | `oasis-backend-psp/src/network.rs` |
| UE5 | No networking | — |

No TLS library (rustls, openssl, native-tls) appears anywhere in the dependency tree.

---

## Research Findings

### Can rustls run on MIPS / PSP?

**Rustls itself is pure Rust and architecture-agnostic.** The constraint is the **crypto backend**:

| Backend | MIPS Support | PSP Feasibility | Production Ready |
|---------|-------------|-----------------|------------------|
| **aws-lc-rs** (rustls default) | No MIPS support at all | Not viable | Yes |
| **ring** | mipsel: works; mips: fixed in 0.17 | Risky -- C/asm build system may not handle `mipsel-sony-psp` custom target, no standard C runtime | Yes |
| **rustls-rustcrypto** | Pure Rust, any target | Best portability | **No** (experimental, "USE AT YOUR OWN RISK") |

### The `ring` Problem on PSP

ring compiles C and assembly from BoringSSL using the `cc` crate. The PSP target (`mipsel-sony-psp`)
is a custom target defined by the `psp` crate, not a standard Rust tier. ring's build.rs checks
`target_arch`, `target_os`, and `target_env` -- the PSP has `target_os = "psp"` which ring does not
recognize. Even if we patch the build script:

- ring expects a standard C library (PSP provides Sony's proprietary syscalls)
- ring's MIPS assembly is MIPS64-only; 32-bit falls back to C generics
- Cross-compilation for this non-standard triple is brittle and hard to maintain

### The Alternative: `embedded-tls`

Paul Sajna (sajattack, author of rust-psp) has **already demonstrated TLS 1.3 on PSP** using
`drogue-tls` (now renamed `embedded-tls`). This is the only library with a proven PSP success story.

**embedded-tls advantages:**
- Pure Rust via RustCrypto (`p256`, `sha2`, `aes-gcm`, `chacha20poly1305`)
- `no_std` + `alloc` support (works on any target Rust compiles to)
- TLS 1.3 only (smaller, simpler, modern)
- ~16 KB record buffer (minimal memory)
- No C build system, no ring, no `cc` crate headaches
- ~100-300 KB total memory footprint (binary + runtime + certs)

**embedded-tls limitations:**
- TLS 1.3 only (no TLS 1.2 fallback -- most modern servers support 1.3)
- Less battle-tested than rustls
- Smaller community/ecosystem

### Recommended Approach: Dual-Stack

Use **rustls** (with `ring` backend) for desktop/SDL builds where ring works perfectly, and
**embedded-tls** for the PSP build where portability matters. Abstract TLS behind a trait so
`oasis-core` is backend-agnostic.

This gives us:
- Production-quality TLS on desktop (rustls + ring, battle-tested)
- Working TLS on PSP (embedded-tls, proven on this exact hardware)
- Clean abstraction that doesn't leak implementation details into the browser

### Memory Budget (PSP, 32 MB total)

| Component | Estimate |
|-----------|----------|
| embedded-tls binary code | ~80-150 KB |
| RustCrypto primitives | ~50-100 KB |
| webpki-roots CA bundle | ~200-300 KB (or ~5 KB if pinning specific certs) |
| Runtime per connection | ~20-50 KB |
| **Total** | **~350 KB - 600 KB** |

This is ~1-2% of PSP RAM. Acceptable.

### Certificate Verification

PSP has no OS certificate store. Options:
- **`webpki-roots`**: Embeds Mozilla's root CAs (~250 KB). Works everywhere, no filesystem needed.
- **Certificate pinning**: Embed only specific root CAs you need (~5 KB). Best for PSP memory.
- **`rustls-platform-verifier`**: Requires OS cert store. **Not suitable for PSP.**

Recommendation: Use `webpki-roots` on desktop, offer a `tls-minimal-certs` feature for PSP that
embeds only Let's Encrypt + a few major roots to save ~200 KB.

---

## Implementation Plan

### Phase 1: TLS Abstraction Layer (oasis-core)

**Goal:** Define a `TlsProvider` trait in oasis-core so the browser and networking code are
backend-agnostic.

**Files to create/modify:**

1. **New: `oasis-core/src/net/tls.rs`**
   - `TlsProvider` trait:
     ```rust
     pub trait TlsProvider: Send + Sync {
         /// Wrap a plain TCP stream in TLS, performing the handshake.
         /// `server_name` is used for SNI and certificate verification.
         fn connect_tls(
             &self,
             tcp: Box<dyn NetworkStream>,
             server_name: &str,
         ) -> Result<Box<dyn NetworkStream>>;
     }
     ```
   - `TlsStream` wrapper that implements `NetworkStream` over an encrypted channel
   - A `NoopTlsProvider` that returns an error (for builds without TLS)

2. **Modify: `oasis-core/src/backend.rs`**
   - Add optional `fn tls_provider(&self) -> Option<&dyn TlsProvider>` to `NetworkBackend`
     with a default `None` implementation (backwards compatible)

3. **Modify: `oasis-core/src/net/mod.rs`**
   - Export the new `tls` module

### Phase 2: Desktop TLS via rustls (oasis-backend-sdl)

**Goal:** Add rustls+ring TLS to the SDL/desktop backend.

**Files to modify:**

1. **`Cargo.toml` (workspace root)**
   - Add workspace dependencies:
     ```toml
     rustls = { version = "0.23", default-features = false, features = ["ring", "logging", "tls12"] }
     webpki-roots = "0.26"
     ```

2. **`crates/oasis-core/Cargo.toml`**
   - Add optional deps behind a `tls-rustls` feature:
     ```toml
     [features]
     default = []
     tls-rustls = ["rustls", "webpki-roots"]
     ```

3. **New: `oasis-core/src/net/tls_rustls.rs`**
   - `RustlsTlsProvider` implementing `TlsProvider`
   - Constructs `rustls::ClientConfig` with webpki-roots
   - Wraps `rustls::ClientConnection` + `NetworkStream` into a `NetworkStream`-compatible type
   - Only compiled under `#[cfg(feature = "tls-rustls")]`

4. **`crates/oasis-backend-sdl/Cargo.toml`**
   - Enable `oasis-core/tls-rustls` feature

5. **`crates/oasis-backend-sdl/src/lib.rs`**
   - Return `RustlsTlsProvider` from `tls_provider()` on `StdNetworkBackend`

### Phase 3: Browser HTTPS Support

**Goal:** Make the browser use TLS when a `TlsProvider` is available.

**Files to modify:**

1. **`oasis-core/src/browser/loader/http.rs`**
   - Change `http_get()` signature to accept an optional `&dyn TlsProvider`
   - For `https` scheme: connect TCP to port 443, then call `tls_provider.connect_tls()`
   - Remove the `https_error_page` early return when a provider is available
   - Keep the error page as fallback when no TLS provider is configured

2. **`oasis-core/src/browser/loader/mod.rs`**
   - Thread the `TlsProvider` through `load_resource()` → `load_from_network()` → `http_get()`
   - The provider comes from the browser's backend reference

3. **`oasis-core/src/browser/mod.rs`** (or wherever the browser app holds its backend ref)
   - Pass the TLS provider from the backend down to the loader

### Phase 4: Gemini Protocol (TLS-mandatory)

**Goal:** Enable Gemini fetching, which requires TLS for every connection.

**Files to modify:**

1. **New: `oasis-core/src/browser/loader/gemini_fetch.rs`**
   - `gemini_get(url, tls_provider)` function
   - Connect TCP to port 1965, wrap in TLS, send `url\r\n`, read response
   - Parse using existing `browser/gemini/parse_response()`
   - Certificate handling: Gemini uses TOFU (Trust On First Use) -- implement a simple
     cert fingerprint cache

2. **`oasis-core/src/browser/loader/mod.rs`**
   - Add `"gemini"` to the scheme match in `load_from_network()`
   - Route to `gemini_fetch::gemini_get()`

### Phase 5: PSP TLS via embedded-tls

**Goal:** Add TLS to the PSP backend using embedded-tls.

**Files to modify:**

1. **`crates/oasis-backend-psp/Cargo.toml`**
   - Add dependencies:
     ```toml
     embedded-tls = { version = "0.17", default-features = false, features = ["alloc"] }
     ```
   - May also need specific RustCrypto crate versions that compile on mipsel-sony-psp

2. **New: `crates/oasis-backend-psp/src/tls.rs`**
   - `PspTlsProvider` implementing `TlsProvider`
   - Wraps embedded-tls's `TlsConnection` around the PSP raw socket fd
   - Adapts embedded-tls's `Read`/`Write` expectations to PSP's `sceNetInet*` syscalls
   - Embeds a minimal CA root set (Let's Encrypt ISRG Root X1 + X2 at minimum)

3. **`crates/oasis-backend-psp/src/network.rs`**
   - Return `PspTlsProvider` from `tls_provider()` on `PspNetworkBackend`

### Phase 6: Testing & Hardening

1. **Unit tests** for TLS trait + rustls adapter (mock `NetworkStream`)
2. **Integration test**: fetch `https://example.com` in the browser on desktop
3. **PSP test**: fetch HTTPS page via PPSSPP headless test in CI
4. **Gemini test**: fetch a Gemini page (e.g., `gemini://geminispace.info/`)
5. **Error handling**: graceful fallback when TLS handshake fails, certificate errors
   shown as browser error pages (not panics)
6. **Feature gating**: ensure `cargo build` without TLS features still compiles (UE5 backend)

---

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| embedded-tls fails to compile on mipsel-sony-psp | Low (pure Rust, proven by sajattack) | Fall back to vendoring/forking embedded-tls with PSP-specific patches |
| embedded-tls's RustCrypto deps have MIPS issues | Low | Pin known-good versions; RustCrypto is pure Rust |
| TLS 1.3-only limitation on PSP (some old servers need 1.2) | Medium | Accept limitation; most servers support 1.3 by 2026. Desktop gets 1.2 via rustls |
| Memory pressure on PSP with CA bundle | Low | Use minimal cert set, or pin specific roots |
| Performance: TLS handshake slow on PSP 333 MHz | Medium | Accept ~1-3 second handshake; cache sessions where possible |
| rustls-rustcrypto not production-ready | N/A | We're NOT using it -- ring for desktop, embedded-tls for PSP |

---

## Dependency Summary

### Desktop (oasis-backend-sdl)
```
rustls 0.23 (with ring backend)
webpki-roots 0.26
```

### PSP (oasis-backend-psp)
```
embedded-tls 0.17 (pure Rust, RustCrypto backend)
```

### UE5 (oasis-backend-ue5)
```
(no TLS -- rendering only, no networking)
```

---

## What This Unlocks

- **HTTPS browsing** on all platforms (the modern web is ~95% HTTPS)
- **Gemini protocol** fully functional (TLS is mandatory for Gemini)
- **Encrypted remote terminal** (upgrade from plaintext PSK to TLS)
- **Future**: HTTPS API calls, secure plugin downloads, encrypted VFS sync
