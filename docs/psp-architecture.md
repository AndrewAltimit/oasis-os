# PSP Architecture

Target-specific gotchas and design decisions for `mipsel-sony-psp`.
This document consolidates five sections that used to live inline in
`CLAUDE.md`. Pair this with [`psp-plugin.md`](psp-plugin.md) for the
kernel-mode PRX and [`adr/004-psp-two-binary-architecture.md`](adr/004-psp-two-binary-architecture.md)
for the architecture decision record.

## Two-Binary Architecture

The PSP deployment uses two binaries:

- **`oasis-backend-psp`** (EBOOT.PBP) — the full shell application,
  runs standalone.
- **`oasis-plugin-psp`** (PRX) — lightweight companion module loaded
  by CFW (ARK-4 / PRO) via `PLUGINS.TXT`. Stays resident in kernel
  memory alongside games.

The PRX hooks `sceDisplaySetFrameBuf` to draw overlay UI into the
game's framebuffer and claims a PSP audio channel for background MP3
playback. No dependency on `oasis-core` — direct framebuffer rendering
only (<64 KB binary).

## GU Rendering Constraints

The PSP GU (Graphics Unit) has a fixed-size command buffer
(`DISPLAY_LIST`, 1 MB in BSS). Each `fill_rect`, glyph blit, clip
push/pop, and blend-mode change appends commands. Browser pages with
many elements can generate hundreds of KB of GU commands per frame.

### Critical rules

- **Never call `reinit_gu_frame()` after `swap_buffers_inner()`.**
  `swap_buffers_inner` already starts a new GU frame via `sceGuStart`.
  A second `sceGuStart` without `sceGuFinish` corrupts the command
  buffer and hangs `sceGuSync` on the next frame. Only use
  `reinit_gu_frame()` after utility dialogs (OSK, `psp::dialog`) which
  run their own GU frames.

- **`std::time::Instant` works on PSP Allegrex** (verified on real
  hardware 2026-04-13). The earlier "crashes" claim was a
  misdiagnosis — actual cause was that the rust-psp std overlay had no
  `target_os = "psp"` arm in the new `sys/time/mod.rs`, so PSP fell
  through to `unsupported::Instant::now` which `panic!()`s. Fixed in
  rust-psp branch `fix/psp-hardware-std-overlay-alignment-and-time`
  by adding `sys/time/psp.rs` and wiring it through. Browser `tick()`
  is currently still gated off on PSP for legacy reasons, but the
  gate can be removed whenever browser perf needs it.

## JavaScript (QuickJS-NG)

QuickJS bring-up and cross-compile details live in
[`javascript-engine.md`](javascript-engine.md) — summarised here:
pspdev toolchain via the `cc` crate, `-msingle-float` mandatory,
final link through `psp-ld`, and a hand-rolled libc/libm shim because
pspdev's prebuilt newlib can't link with Rust's o32/mdouble-float
code. Lazy `Option<BrowserWidget>` init keeps boot cost at zero.

## TLS 1.3

The PSP firmware's built-in SSL uses root CAs from 2008 and SSL 3.0,
which can't connect to modern HTTPS servers. The PSP backend
implements native TLS 1.3 via `embedded-tls` (pure Rust, no C/asm)
with `UnsecureProvider` (no certificate validation).

- **`alloc` feature required** to advertise RSA signature schemes —
  archive.org uses RSA certs.
- Raw TCP sockets (`sceNetInet*`) wrapped with `embedded_io::Read +
  Write` adapters.
- **RNG seeded from `sceKernelGetSystemTimeLow`.** `mfc0 $9` (COP0
  Count register) is privileged on PSP Allegrex and crashes in user
  mode.
- **DNS resolution** via `psp::net::resolve_hostname` with
  `to_ne_bytes()` (network byte-order fix for little-endian MIPS).
- **HTTP→HTTPS redirect loops detected automatically**, triggering
  TLS fallback. HTTPS redirects (archive.org → CDN node) are followed
  within the TLS path.
- **I/O thread stack** increased to 512 KB (from 256 KB) for TLS
  crypto type headroom.
- **`sceHttpDisableRedirect(template_id)` is REQUIRED** — without it
  sceHttp auto-follows HTTP→HTTPS redirects which fail. The
  `0x80431079` channel-switching error was actually this, not
  connection-pool corruption.
- **`embedded_io::Write::flush()` must be called after `write_all()`** —
  TLS buffers data internally.
- **Import stub weak flag** — `psp_extern!` flag `0x0008` means weak
  import. `sceAudiocodec` uses `0x4009` (weak) and works;
  `sceVideocodec` was `0x4001` (strong) and broke module loading on
  real hardware. Fixed on branch `fix/weak-videocodec-import` in the
  rust-psp fork.

## Video Streaming

TV Guide on PSP uses in-memory streaming (no disk I/O). The I/O
thread downloads HTTP(S) data, buffers the MP4 `moov` atom (~1-3 MB),
parses track tables via `demux_lite::Mp4Lite`, then extracts
interleaved audio/video samples from the `mdat` stream in file-offset
order.

- **Video** — H.264 frames decoded via ME hardware (`sceMpeg` NAL
  direct path with `mpeg_vsh370.prx`).
- **Audio** — AAC frames decoded via `sceAudiocodec` hardware, output
  through `AudioChannel::output_blocking`.
- **Content selection** prefers ≤480p via
  `select_smallest_with_max_width(max_width=480)` — the ME handles
  ≤480p decode indefinitely (tested 8300+ frames, 0 errors). >480p
  triggers firmware deadlocks.

### Key mechanisms

- **Frame delivery** — pre-decode queue wait: video thread checks
  `VIDEO_FRAME_QUEUE` (capacity 2) has space before committing to
  ~50 ms ME decode. Reduces frame drops from ~72% to ~3%. Main thread
  drains all queued frames per render, uploads only the latest.
- **Frameskip** — texture upload (~80 ms for 322 KB copy + cache
  flush) only runs every 3rd main loop frame, giving audio DMA
  uninterrupted CPU time to prevent stuttering.
- **Strided CSC** — `decode_into_strided()` writes CSC output at
  512 px stride matching the GU texture buffer, eliminating two
  stride-conversion copies per frame.
- **Audio buffering** — 64-slot audio command queue (~1.5 s at
  44.1 kHz / 1024 AAC frames) absorbs network I/O jitter that
  previously caused audible stuttering.
- **Fullscreen blit** — during video playback, all UI chrome
  (wallpaper, status bar, bottom bar, SDI overlay) is skipped; only
  the video blit + title overlay renders.
- **ME safety** — `sceMpegDelete` crashes after prolonged decode;
  the decoder is intentionally leaked on ME deadlock recovery
  (watchdog signals sceMpeg internal semaphore). `ME_LEAKED` flag
  prevents reinit until reboot.
- **WiFi retry** — TCP command server retries WiFi connection
  indefinitely instead of giving up after 2 attempts.

## PSP Constraints (general)

- Manual byte loops needed for memcpy / memset (LLVM recursion on
  MIPS).
- Textures must be power-of-2, 16-byte aligned.
- VRAM stride = 512 px (for 480 px display).
- Uncached memory: `ptr | 0x4000_0000` (or use `psp::cache` types).
- Module declaration: `psp::module_kernel!()` for kernel mode.
- ME core: no syscalls, no cached memory, no heap — pure integer /
  float math only.

## Related docs

- [`psp-plugin.md`](psp-plugin.md) — kernel-mode PRX.
- [`adr/004-psp-two-binary-architecture.md`](adr/004-psp-two-binary-architecture.md)
  — ADR.
- [`psp-me-direct-plan.md`](psp-me-direct-plan.md),
  [`psp-me-driver-map.md`](psp-me-driver-map.md),
  [`psp-me-firmware-analysis.md`](psp-me-firmware-analysis.md),
  [`psp-me-rpc-api.md`](psp-me-rpc-api.md) — ME driver deep dives.
- [`psp-video-decode-plan.md`](psp-video-decode-plan.md) — video
  decode design doc.
- [`psp-usb-hardware-reference.md`](psp-usb-hardware-reference.md),
  [`psp-usb-vbus-findings.md`](psp-usb-vbus-findings.md) — USB host
  mode experiments.
- [`scripts/psp-scenarios.md`](../scripts/psp-scenarios.md) — test
  scenarios.
- [`video-streaming.md`](video-streaming.md) — desktop counterpart.
