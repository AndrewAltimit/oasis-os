# FFI Integration Guide

This guide explains how to embed OASIS_OS in a C/C++ application (including Unreal Engine 5) using the C-ABI shared library.

## Building the Library

```bash
cargo build --profile release-ffi -p oasis-ffi
```

This produces a shared library:
- Linux: `target/release-ffi/liboasis_ffi.so`
- macOS: `target/release-ffi/liboasis_ffi.dylib`
- Windows: `target/release-ffi/oasis_ffi.dll` (plus the `oasis_ffi.dll.lib` import library)

### Panic safety -- always use the `release-ffi` profile

Every exported `oasis_*` function wraps its body in a `catch_unwind` guard
(`ffi_guard` in `crates/oasis-ffi/src/handle.rs`). A Rust panic inside an
export is logged (`log::error!`, "FFI: panic caught in <function>") and the
function returns its documented failure value instead of unwinding into the
host:

| Return type | Value on panic |
|-------------|----------------|
| `void` | returns normally |
| `bool` | `false` |
| pointer (`oasis_create*`, `oasis_send_command`, `oasis_get_buffer`) | `NULL` |
| `uint64_t` (`oasis_audio_load`) | `UINT64_MAX` |
| `uint8_t` (`oasis_audio_get_volume`) | `0` |
| `int32_t` (`oasis_video_*`) | `-1` (`oasis_video_is_playing`: `0`) |

The handle is **not poisoned** by a caught panic: it stays valid and later
calls (including `oasis_destroy`) work normally. At worst one frame is left
partially drawn and the next `oasis_tick` redraws it.

`catch_unwind` only works when panics unwind. The workspace `release` profile
sets `panic = "abort"` (smaller, faster desktop binaries), under which a panic
kills the whole process -- including the UE5 editor/game -- before the guard
runs. The `release-ffi` profile inherits `release` but sets
`panic = "unwind"`; **always build the shared library with
`--profile release-ffi`** when embedding it in a host process.

The crate is also built as an `rlib`, so Rust hosts can depend on it directly.

### Cargo features

| Feature | Default | Effect |
|---------|---------|--------|
| `video-decode` | No | Exports the `oasis_video_*` functions using the software decoder (openh264 + symphonia; needs only a C/C++ compiler) |
| `video-decode-ffmpeg` | No | Exports the `oasis_video_*` functions using ffmpeg (needs the ffmpeg dev libraries + `pkg-config`, see [getting-started.md](getting-started.md#build-dependencies)) |

Without either feature, the `oasis_video_*` symbols are **not exported**.

```bash
cargo build --profile release-ffi -p oasis-ffi --features video-decode
```

A complete, runnable C program exercising the API lives in
[`examples/ffi_demo.c`](../examples/ffi_demo.c) (build instructions in its
header comment and in [`examples/README.md`](../examples/README.md)).

## C Header Reference

Below is the equivalent C header for the exported API. The exports are defined across
`crates/oasis-ffi/src/` (`lifecycle.rs`, `render.rs`, `input.rs`, `commands.rs`, `vfs.rs`,
`callbacks.rs`, `audio.rs`, `video.rs`); the constants live in `types.rs`.

```c
#pragma once
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle to an OASIS_OS instance. */
typedef struct OasisInstance OasisInstance;

/* ----------------------------------------------------------------
 * Lifecycle
 * ---------------------------------------------------------------- */

/* Create a new instance with the default theme.
 *
 * width/height:  Virtual screen dimensions, each 1..=4096 (typically 480x272;
 *                the theme and layout are scaled to this size).
 * skin_toml:     Optional skin manifest (the skin's skin.toml).
 * layout_toml:   Optional layout (layout.toml).
 * features_toml: Optional feature gates (features.toml).
 *
 * The skin is only applied when all three strings are non-NULL; otherwise
 * the built-in defaults are used. The strings are parsed during the call and
 * may be freed afterwards.
 *
 * Returns an opaque handle, or NULL on failure (e.g. invalid dimensions).
 */
OasisInstance* oasis_create(
    uint32_t width,
    uint32_t height,
    const char* skin_toml,
    const char* layout_toml,
    const char* features_toml
);

/* Create a new instance from a skin's full TOML set.
 *
 * Like oasis_create(), but also takes the skin's theme.toml (color scheme)
 * and strings.toml (display strings) so the instance renders with the skin's
 * real theme instead of the default. theme_toml and strings_toml may be NULL
 * (defaults are used); the skin itself still requires manifest, layout and
 * features to all be non-NULL.
 */
OasisInstance* oasis_create_full(
    uint32_t width,
    uint32_t height,
    const char* manifest_toml,
    const char* layout_toml,
    const char* features_toml,
    const char* theme_toml,
    const char* strings_toml
);

/* Destroy an instance and free all memory.
 * Safe to call with NULL.
 */
void oasis_destroy(OasisInstance* handle);
/* (Destroying fires the audio SHUTDOWN callback. The handle is invalid
 * afterwards: destroying it twice is undefined behaviour.) */

/* Advance the OS by one frame.
 *
 * delta_seconds: Time since last tick (e.g. 1.0/60.0 for 60 FPS).
 * Processes input, updates scene graph, renders to internal buffer.
 */
void oasis_tick(OasisInstance* handle, float delta_seconds);

/* ----------------------------------------------------------------
 * Framebuffer
 * ---------------------------------------------------------------- */

/* Get a pointer to the RGBA framebuffer.
 *
 * out_width/out_height: Optional output parameters (may be NULL).
 * Returns pointer to width*height*4 bytes of RGBA pixel data.
 * The pointer is valid until the next oasis_tick() or oasis_destroy().
 */
const uint8_t* oasis_get_buffer(
    OasisInstance* handle,
    uint32_t* out_width,
    uint32_t* out_height
);

/* Check if the framebuffer has changed since the last read.
 * Clears the dirty flag after reading.
 */
bool oasis_get_dirty(OasisInstance* handle);

/* ----------------------------------------------------------------
 * Input
 * ---------------------------------------------------------------- */

/* Event types */
#define OASIS_EVENT_CURSOR_MOVE     1
#define OASIS_EVENT_BUTTON_PRESS    2
#define OASIS_EVENT_BUTTON_RELEASE  3
#define OASIS_EVENT_TRIGGER_PRESS   4
#define OASIS_EVENT_TRIGGER_RELEASE 5
#define OASIS_EVENT_TEXT_INPUT      6
#define OASIS_EVENT_POINTER_CLICK   7
#define OASIS_EVENT_POINTER_RELEASE 8
#define OASIS_EVENT_FOCUS_GAINED    9
#define OASIS_EVENT_FOCUS_LOST     10
#define OASIS_EVENT_QUIT           11
#define OASIS_EVENT_BACKSPACE      12
#define OASIS_EVENT_MOUSE_WHEEL    13  /* delta in y; positive = scroll down */
#define OASIS_EVENT_TOGGLE_FULLSCREEN 14
#define OASIS_EVENT_TAB            15
#define OASIS_EVENT_SHIFT_TAB      16
#define OASIS_EVENT_KEY            17  /* raw key: key=OASIS_KEY_*, x=OASIS_MOD_* bits,
                                          character=codepoint for OASIS_KEY_CHAR */

/* Button codes (PSP layout) */
#define OASIS_BUTTON_UP       0
#define OASIS_BUTTON_DOWN     1
#define OASIS_BUTTON_LEFT     2
#define OASIS_BUTTON_RIGHT    3
#define OASIS_BUTTON_CONFIRM  4  /* X on PSP */
#define OASIS_BUTTON_CANCEL   5  /* O on PSP */
#define OASIS_BUTTON_TRIANGLE 6
#define OASIS_BUTTON_SQUARE   7
#define OASIS_BUTTON_START    8
#define OASIS_BUTTON_SELECT   9

/* Trigger codes */
#define OASIS_TRIGGER_LEFT    0
#define OASIS_TRIGGER_RIGHT   1

/* Key codes (OASIS_EVENT_KEY) */
#define OASIS_KEY_CHAR        0   /* printable key; character = codepoint */
#define OASIS_KEY_SPACE       1
#define OASIS_KEY_ENTER       2
#define OASIS_KEY_ESCAPE      3
#define OASIS_KEY_TAB         4
#define OASIS_KEY_BACKSPACE   5
#define OASIS_KEY_DELETE      6
#define OASIS_KEY_INSERT      7
#define OASIS_KEY_HOME        8
#define OASIS_KEY_END         9
#define OASIS_KEY_PAGE_UP    10
#define OASIS_KEY_PAGE_DOWN  11
#define OASIS_KEY_UP         12
#define OASIS_KEY_DOWN       13
#define OASIS_KEY_LEFT       14
#define OASIS_KEY_RIGHT      15
#define OASIS_KEY_F1        101  /* F2..F12 = 102..112 */
#define OASIS_KEY_F12       112

/* Modifier bits (OASIS_EVENT_KEY, carried in x) */
#define OASIS_MOD_SHIFT  1
#define OASIS_MOD_CTRL   2
#define OASIS_MOD_ALT    4
#define OASIS_MOD_SUPER  8

typedef struct {
    uint32_t event_type;
    int32_t x, y;         /* Cursor/pointer coordinates; x = modifier bits
                             for KEY, y = delta for MOUSE_WHEEL */
    uint32_t key;         /* Button/trigger code, or OASIS_KEY_* for KEY */
    uint32_t character;   /* Unicode codepoint (TEXT_INPUT, KEY with OASIS_KEY_CHAR) */
} OasisInputEvent;

/* Deliver an input event to the instance. It is processed on the next
 * oasis_tick(). On the dashboard (skinned instances): d-pad buttons move
 * the selection, CONFIRM launches the selected app, the L/R triggers
 * change page, a POINTER_CLICK on an icon selects and launches it, and
 * CURSOR_MOVE drives the icon hover effect. Launching fires
 * OASIS_CB_APP_LAUNCH with the app title. */
void oasis_send_input(OasisInstance* handle, const OasisInputEvent* event);

/* ----------------------------------------------------------------
 * Terminal Commands
 * ---------------------------------------------------------------- */

/* Execute a terminal command and return the output as a C string.
 *
 * The returned string must be freed with oasis_free_string().
 * Returns NULL on invalid input.
 */
char* oasis_send_command(OasisInstance* handle, const char* cmd);

/* Free a string returned by oasis_send_command(). Safe with NULL; each
 * string must be freed exactly once. Interior NUL bytes in command output
 * (e.g. `cat` of a binary file) are replaced with U+FFFD. */
void oasis_free_string(char* ptr);

/* ----------------------------------------------------------------
 * Virtual File System
 * ---------------------------------------------------------------- */

/* Reset the VFS to a clean state (empty /home, /etc, /tmp) and the cwd to
 * "/". `path` is currently ignored. Note: this also drops the /apps
 * directories seeded at creation, so the dashboard has no icons afterwards
 * unless the host re-adds them. */
void oasis_set_vfs_root(OasisInstance* handle, const char* path);

/* Add a file to the VFS base layer.
 *
 * path:     Virtual path (e.g. "/home/readme.txt").
 * data:     File content bytes.
 * data_len: Size in bytes.
 */
void oasis_add_vfs_file(
    OasisInstance* handle,
    const char* path,
    const uint8_t* data,
    uint32_t data_len
);

/* ----------------------------------------------------------------
 * Callbacks
 * ---------------------------------------------------------------- */

/* Callback event types. Currently fired: OASIS_CB_COMMAND_EXEC (detail =
 * the command line, from oasis_send_command) and OASIS_CB_APP_LAUNCH
 * (detail = app title). The other codes are reserved and never fire yet. */
#define OASIS_CB_FILE_ACCESS   1
#define OASIS_CB_COMMAND_EXEC  2
#define OASIS_CB_APP_LAUNCH    3
#define OASIS_CB_LOGIN         4
#define OASIS_CB_NETWORK_SEND  5
#define OASIS_CB_PLUGIN_LOAD   6

typedef void (*OasisCallback)(uint32_t event, const char* detail);

/* Register a callback for OS events. */
/* Registering the same event again replaces the previous callback. */
void oasis_register_callback(
    OasisInstance* handle,
    uint32_t event,
    OasisCallback cb
);

/* ----------------------------------------------------------------
 * Audio
 * ---------------------------------------------------------------- */

/* The instance only tracks audio state; the host performs actual output.
 * The callback fires on every state change so the host can mirror it. */

/* Audio callback event codes (AudioEvent in oasis-backend-ue5) */
#define OASIS_AUDIO_PLAY           0
#define OASIS_AUDIO_PAUSE          1
#define OASIS_AUDIO_RESUME         2
#define OASIS_AUDIO_STOP           3
#define OASIS_AUDIO_VOLUME_CHANGE  4  /* value = new volume (0-100) */
#define OASIS_AUDIO_TRACK_LOADED   5
#define OASIS_AUDIO_TRACK_UNLOADED 6
#define OASIS_AUDIO_SHUTDOWN       7

/* track_id is 0 when not applicable. */
typedef void (*OasisAudioCallback)(uint32_t event, uint64_t track_id, uint32_t value);

void oasis_set_audio_callback(OasisInstance* handle, OasisAudioCallback cb);

/* Load audio data; returns a track ID, or UINT64_MAX on failure.
 * Volume is clamped to 0-100 (VOLUME_CHANGE reports the clamped value);
 * oasis_audio_resume() fails unless a track is paused. */
uint64_t oasis_audio_load(OasisInstance* handle, const uint8_t* data, uint32_t data_len);
/* The following return true on success. */
bool oasis_audio_play(OasisInstance* handle, uint64_t track_id);
bool oasis_audio_pause(OasisInstance* handle);
bool oasis_audio_resume(OasisInstance* handle);
bool oasis_audio_stop(OasisInstance* handle);
bool oasis_audio_set_volume(OasisInstance* handle, uint8_t volume);  /* 0-100 */
uint8_t oasis_audio_get_volume(OasisInstance* handle);
bool oasis_audio_is_playing(OasisInstance* handle);

/* ----------------------------------------------------------------
 * Video Playback (only exported with the video-decode or
 * video-decode-ffmpeg Cargo feature)
 * ---------------------------------------------------------------- */

/* Start software video playback from a local file path.
 *
 * path: Path to an MP4 file on disk.
 * Spawns a background decode thread. Poll frames with oasis_video_next_frame.
 * Returns 0 on success, -1 on error.
 */
int32_t oasis_video_play(OasisInstance* handle, const char* path);

/* Stop video playback, join the decode thread, and clean up resources. */
void oasis_video_stop(OasisInstance* handle);

/* Check if video is currently playing.
 * Returns 1 if playing, 0 if not.
 */
int32_t oasis_video_is_playing(OasisInstance* handle);

/* Poll the latest decoded video frame.
 *
 * Takes the most recent frame (single-buffered; skips intermediate frames).
 * Copies RGBA pixels into buf, sets out_w/out_h to video dimensions.
 * buf_size must be >= out_w * out_h * 4 for the decoded frame dimensions.
 * Returns: 1 = new frame copied, 0 = no new frame available,
 *          -1 = error (including buffer too small).
 */
int32_t oasis_video_next_frame(OasisInstance* handle, uint8_t* buf,
                               uint32_t buf_size,
                               uint32_t* out_w, uint32_t* out_h);

/* Drain decoded audio samples into a host buffer.
 *
 * Copies interleaved f32 PCM into buf, up to max_samples floats.
 * Returns: number of samples copied, or -1 on error.
 */
int32_t oasis_video_get_audio(OasisInstance* handle, float* buf,
                              uint32_t max_samples);

#ifdef __cplusplus
}
#endif
```

## Integration Walkthrough

### 1. Create an Instance

```c
OasisInstance* os = oasis_create(480, 272, NULL, NULL, NULL);
if (!os) {
    fprintf(stderr, "Failed to create OASIS_OS instance\n");
    return 1;
}
```

The native resolution is 480x272. Larger resolutions (up to 4096x4096) work too: the
theme and skin layout are scaled to the requested buffer size.

To render with a specific skin, pass its TOML files (the contents of
`skins/<name>/skin.toml`, `layout.toml`, `features.toml`, `theme.toml`, `strings.toml`)
to `oasis_create_full`:

```c
OasisInstance* os = oasis_create_full(480, 272,
    manifest, layout, features, theme, strings);  /* any may be NULL */
```

The instance starts with an empty `/home`, `/etc`, `/tmp` and one `/apps/<name>`
directory per default dashboard app.

### 2. Populate the Virtual File System

Before ticking, add any files the OS should have access to:

```c
const char* readme = "Welcome to OASIS_OS!";
oasis_add_vfs_file(os, "/home/readme.txt",
    (const uint8_t*)readme, strlen(readme));
```

### 3. Main Loop

Each frame, feed input, tick, and read the framebuffer:

```c
while (running) {
    /* Feed input events from your framework */
    OasisInputEvent ev = {0};
    ev.event_type = OASIS_EVENT_BUTTON_PRESS;
    ev.key = OASIS_BUTTON_CONFIRM;
    oasis_send_input(os, &ev);

    /* Advance one frame */
    oasis_tick(os, 1.0f / 60.0f);

    /* Read framebuffer only when dirty */
    if (oasis_get_dirty(os)) {
        uint32_t w, h;
        const uint8_t* pixels = oasis_get_buffer(os, &w, &h);
        /* Upload pixels to your rendering surface.
         * Format: RGBA, w*h*4 bytes, row-major. */
        upload_texture(pixels, w, h);
    }
}
```

### 4. Execute Commands Programmatically

```c
char* output = oasis_send_command(os, "ls /home");
if (output) {
    printf("Command output:\n%s\n", output);
    oasis_free_string(output);
}
```

### 5. Register Callbacks

```c
void on_app_launch(uint32_t event, const char* detail) {
    printf("App launched: %s\n", detail);
}

oasis_register_callback(os, OASIS_CB_APP_LAUNCH, on_app_launch);
```

### 6. Cleanup

```c
oasis_destroy(os);
```

## Unreal Engine 5 Integration

For UE5 specifically, the `oasis-backend-ue5` crate provides a software RGBA framebuffer renderer. The typical integration pattern:

1. Build `oasis-ffi` as a shared library
2. Place in your UE5 project's `Binaries/` or `ThirdParty/` directory
3. Create a `UOasisComponent` that:
   - Calls `oasis_create()` in `BeginPlay`
   - Calls `oasis_tick()` in `TickComponent`
   - Copies `oasis_get_buffer()` to a `UTexture2D` when dirty
   - Renders the texture on a UI widget or in-world material
   - Forwards UE5 input events via `oasis_send_input()`
   - Calls `oasis_destroy()` in `EndPlay`

The instance's VFS is a `GameAssetVfs` (from `oasis-vfs`), which supports overlay writes on top of read-only UE5 game assets, enabling VFS files to be backed by packaged content.

## Thread Safety

Each `OasisInstance` must be accessed from a single thread. The FFI functions are not thread-safe. If you need multi-threaded access, synchronize externally.

## Memory Management

- `oasis_create()` allocates; `oasis_destroy()` frees
- `oasis_send_command()` allocates; `oasis_free_string()` frees
- All other functions use borrowed pointers (no allocation); TOML strings passed to
  `oasis_create` / `oasis_create_full` are parsed during the call and can be freed afterwards
- The framebuffer pointer from `oasis_get_buffer()` is valid until the next `oasis_tick()` or `oasis_destroy()`
