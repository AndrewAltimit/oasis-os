# FFI Integration Guide

This guide explains how to embed OASIS_OS in a C/C++ application (including Unreal Engine 5) using the C-ABI shared library.

## Building the Library

```bash
cargo build --release -p oasis-ffi
```

This produces a shared library:
- Linux: `target/release/liboasis_ffi.so`
- macOS: `target/release/liboasis_ffi.dylib`
- Windows: `target/release/oasis_ffi.dll`

## C Header Reference

Below is the equivalent C header for the exported API. The actual exports are defined in `crates/oasis-ffi/src/lib.rs`.

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

/* Create a new instance.
 *
 * width/height: Virtual screen dimensions (typically 480x272).
 * skin_toml:    Optional TOML skin definition (NULL for default).
 * layout_toml:  Optional TOML layout definition (NULL for default).
 * features_toml: Optional TOML features definition (NULL for default).
 *
 * Returns an opaque handle, or NULL on failure.
 */
OasisInstance* oasis_create(
    uint32_t width,
    uint32_t height,
    const char* skin_toml,
    const char* layout_toml,
    const char* features_toml
);

/* Destroy an instance and free all memory.
 * Safe to call with NULL.
 */
void oasis_destroy(OasisInstance* handle);

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

typedef struct {
    uint32_t event_type;
    int32_t x, y;         /* Cursor/pointer coordinates */
    uint32_t key;         /* Button or trigger code */
    uint32_t character;   /* Unicode codepoint (TEXT_INPUT only) */
} OasisInputEvent;

/* Deliver an input event to the instance. */
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

/* Free a string returned by oasis_send_command(). */
void oasis_free_string(char* ptr);

/* ----------------------------------------------------------------
 * Virtual File System
 * ---------------------------------------------------------------- */

/* Reset the VFS to a clean state. Also resets cwd to "/". */
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

/* Callback event types */
#define OASIS_CB_FILE_ACCESS   1
#define OASIS_CB_COMMAND_EXEC  2
#define OASIS_CB_APP_LAUNCH    3
#define OASIS_CB_LOGIN         4
#define OASIS_CB_NETWORK_SEND  5
#define OASIS_CB_PLUGIN_LOAD   6

typedef void (*OasisCallback)(uint32_t event, const char* detail);

/* Register a callback for OS events. */
void oasis_register_callback(
    OasisInstance* handle,
    uint32_t event,
    OasisCallback cb
);

/* ----------------------------------------------------------------
 * Audio
 * ---------------------------------------------------------------- */

typedef void (*OasisAudioCallback)(uint32_t event, uint64_t track_id, uint32_t value);

void oasis_set_audio_callback(OasisInstance* handle, OasisAudioCallback cb);

uint64_t oasis_audio_load(OasisInstance* handle, const uint8_t* data, uint32_t data_len);
bool oasis_audio_play(OasisInstance* handle, uint64_t track_id);
bool oasis_audio_pause(OasisInstance* handle);
bool oasis_audio_resume(OasisInstance* handle);
bool oasis_audio_stop(OasisInstance* handle);
bool oasis_audio_set_volume(OasisInstance* handle, uint8_t volume);
uint8_t oasis_audio_get_volume(OasisInstance* handle);
bool oasis_audio_is_playing(OasisInstance* handle);

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

The native resolution is 480x272. You can use a larger resolution, but the UI is designed for this size.

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
void on_file_access(uint32_t event, const char* detail) {
    printf("File accessed: %s\n", detail);
}

oasis_register_callback(os, OASIS_CB_FILE_ACCESS, on_file_access);
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

The `GameAssetVfs` backend in `oasis-backend-ue5` supports overlay writes on top of read-only UE5 game assets, enabling VFS files to be backed by packaged content.

## Thread Safety

Each `OasisInstance` must be accessed from a single thread. The FFI functions are not thread-safe. If you need multi-threaded access, synchronize externally.

## Memory Management

- `oasis_create()` allocates; `oasis_destroy()` frees
- `oasis_send_command()` allocates; `oasis_free_string()` frees
- All other functions use borrowed pointers (no allocation)
- The framebuffer pointer from `oasis_get_buffer()` is valid until the next `oasis_tick()` or `oasis_destroy()`
