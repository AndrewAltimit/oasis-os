//! OASIS_OS C-ABI FFI boundary.
//!
//! Exports an opaque handle API for UE5 (or any C/C++ host) to create,
//! drive, and render OASIS_OS instances. All internal Rust state is behind
//! an opaque `OasisInstance` pointer. UE5 never sees Rust types.
//!
//! # Safety contract
//!
//! - **Handles.** Every function taking an `*mut OasisInstance` accepts
//!   null (it does nothing and returns its documented default) or a live
//!   handle from `oasis_create` / `oasis_create_full`. A destroyed handle
//!   is undefined behavior; so is destroying one twice.
//! - **Pointers in.** C strings must be null or NUL-terminated (invalid
//!   UTF-8 is rejected like null); `data` / `buf` pointers must be valid for
//!   the length passed with them. Nothing passed in is retained after the
//!   call returns.
//! - **Pointers out.** Strings from `oasis_send_command` are owned by the
//!   caller until passed to `oasis_free_string` (exactly once). The
//!   framebuffer from `oasis_get_buffer` is borrowed: valid until the next
//!   `oasis_tick` or `oasis_destroy` on that handle.
//! - **Callbacks** (`oasis_register_callback`, `oasis_set_audio_callback`
//!   and their `_ex` forms, which also pass back a host `user_data` pointer)
//!   may be null, which unregisters. They run synchronously inside the
//!   `oasis_*` call that fired them, on the caller's thread. A call they
//!   make back into the API with the same handle (the instance is
//!   exclusively borrowed while they run) is refused: it returns its
//!   default and logs an error; `oasis_destroy` from a callback is refused
//!   too. The `detail` string is only valid for the duration of the
//!   callback.
//! - **Panics** never unwind into C: every export catches them and returns
//!   its default (null / false / -1 / `UINT64_MAX`), provided the library
//!   is built with `panic = "unwind"` (the `release-ffi` profile). The
//!   handle stays usable afterwards.
//!
//! # Thread Safety
//!
//! `OasisInstance` is **not** thread-safe. All calls on one handle must be
//! serialized (typically all on the UE5 game thread); calling any `oasis_*`
//! function concurrently on the same handle is undefined behavior. Distinct
//! handles are independent and may live on different threads. The only
//! internal thread is the video decoder's, which `oasis_video_stop` and
//! `oasis_destroy` join.
//!
//! Recommended UE5 integration pattern: own the handle in one component
//! and make every call from the game thread, so no locking is needed.
//! ```cpp
//! // BeginPlay:
//! OasisInstance* Oasis = oasis_create(480, 272, nullptr, nullptr, nullptr);
//! // TickComponent:
//! oasis_tick(Oasis, DeltaTime);
//! // EndPlay:
//! oasis_destroy(Oasis);
//! ```
//! A host that must call from several threads serializes every call on a
//! handle behind one lock it owns.
//!
//! # Feature flags
//!
//! - `video-decode` — `oasis_video_*` exports with openh264 decode
//!   (`oasis-video/h264`).
//! - `video-decode-ffmpeg` — the same exports with statically linked ffmpeg
//!   (`oasis-video/ffmpeg`).
//!
//! Without either feature the `oasis_video_*` exports are not compiled.

// Modules organized by functionality.
mod audio;
mod callbacks;
mod commands;
mod handle;
mod input;
mod lifecycle;
mod render;
mod types;
mod vfs;
mod video;

// Re-export all public C-ABI symbols and types so the crate root exports them.
pub use audio::*;
pub use callbacks::*;
pub use commands::*;
pub use handle::OasisInstance;
pub use input::*;
pub use lifecycle::*;
pub use render::*;
pub use types::*;
pub use vfs::*;
#[cfg(feature = "_video")]
pub use video::*;

#[cfg(test)]
mod tests;
