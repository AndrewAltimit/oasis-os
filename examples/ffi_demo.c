/**
 * FFI Demo -- Embed OASIS_OS in a C application.
 *
 * Exercises the C-ABI exported by crates/oasis-ffi: instance creation with a
 * full skin (oasis_create_full), VFS seeding, ticking, framebuffer readback,
 * terminal commands, input (gamepad-style and raw keys), OS event callbacks,
 * and the audio callback. See docs/ffi-integration.md for the full reference.
 *
 * Build the shared library first (from the repo root):
 *   cargo build --profile release-ffi -p oasis-ffi
 *
 * Then compile and run this demo from the repo root (it reads skins/classic):
 *   Linux:  gcc -o ffi_demo examples/ffi_demo.c \
 *               -L target/release-ffi -loasis_ffi -Wl,-rpath,target/release-ffi
 *   macOS:  clang -o ffi_demo examples/ffi_demo.c \
 *               -L target/release-ffi -loasis_ffi -Wl,-rpath,@loader_path/target/release-ffi
 *   ./ffi_demo
 *
 * The video functions are only exported when oasis-ffi is built with
 * `--features video-decode` (or `video-decode-ffmpeg`); compile this demo
 * with -DOASIS_WITH_VIDEO to declare and call them.
 */

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------------
 * Declarations matching the oasis-ffi exports (crates/oasis-ffi/src).
 * ------------------------------------------------------------------------ */

typedef struct OasisInstance OasisInstance;

typedef struct {
    uint32_t event_type;
    int32_t x, y;       /* pointer coords; x = OASIS_MOD_* bits for KEY; y = wheel delta */
    uint32_t key;       /* button/trigger code, or OASIS_KEY_* for KEY */
    uint32_t character; /* codepoint for TEXT_INPUT and OASIS_KEY_CHAR */
} OasisInputEvent;

typedef void (*OasisCallback)(uint32_t event, const char* detail);
typedef void (*OasisAudioCallback)(uint32_t event, uint64_t track_id, uint32_t value);

/* Lifecycle */
extern OasisInstance* oasis_create(uint32_t width, uint32_t height, const char* skin_toml,
                                   const char* layout_toml, const char* features_toml);
extern OasisInstance* oasis_create_full(uint32_t width, uint32_t height,
                                        const char* manifest_toml, const char* layout_toml,
                                        const char* features_toml, const char* theme_toml,
                                        const char* strings_toml);
extern void oasis_destroy(OasisInstance* handle);
extern void oasis_tick(OasisInstance* handle, float delta_seconds);

/* Framebuffer */
extern const uint8_t* oasis_get_buffer(OasisInstance* handle, uint32_t* out_w, uint32_t* out_h);
extern bool oasis_get_dirty(OasisInstance* handle);

/* Input */
extern void oasis_send_input(OasisInstance* handle, const OasisInputEvent* ev);

/* Terminal commands */
extern char* oasis_send_command(OasisInstance* handle, const char* cmd);
extern void oasis_free_string(char* ptr);

/* VFS */
extern void oasis_set_vfs_root(OasisInstance* handle, const char* path);
extern void oasis_add_vfs_file(OasisInstance* handle, const char* path, const uint8_t* data,
                               uint32_t data_len);

/* Callbacks */
extern void oasis_register_callback(OasisInstance* handle, uint32_t event, OasisCallback cb);

/* Audio (the host performs actual output; OASIS reports state changes) */
extern void oasis_set_audio_callback(OasisInstance* handle, OasisAudioCallback cb);
extern uint64_t oasis_audio_load(OasisInstance* handle, const uint8_t* data, uint32_t data_len);
extern bool oasis_audio_play(OasisInstance* handle, uint64_t track_id);
extern bool oasis_audio_pause(OasisInstance* handle);
extern bool oasis_audio_resume(OasisInstance* handle);
extern bool oasis_audio_stop(OasisInstance* handle);
extern bool oasis_audio_set_volume(OasisInstance* handle, uint8_t volume);
extern uint8_t oasis_audio_get_volume(OasisInstance* handle);
extern bool oasis_audio_is_playing(OasisInstance* handle);

#ifdef OASIS_WITH_VIDEO
/* Video (only with oasis-ffi feature video-decode / video-decode-ffmpeg) */
extern int32_t oasis_video_play(OasisInstance* handle, const char* path);
extern void oasis_video_stop(OasisInstance* handle);
extern int32_t oasis_video_is_playing(OasisInstance* handle);
extern int32_t oasis_video_next_frame(OasisInstance* handle, uint8_t* buf, uint32_t buf_size,
                                      uint32_t* out_w, uint32_t* out_h);
extern int32_t oasis_video_get_audio(OasisInstance* handle, float* buf, uint32_t max_samples);
#endif

/* Constants (mirrors crates/oasis-ffi/src/types.rs) */
#define OASIS_EVENT_BUTTON_PRESS 2
#define OASIS_EVENT_KEY 17
#define OASIS_BUTTON_RIGHT 3
#define OASIS_KEY_CHAR 0
#define OASIS_MOD_CTRL 2
#define OASIS_CB_COMMAND_EXEC 2
#define OASIS_CB_FILE_ACCESS 1

/* ------------------------------------------------------------------------
 * Helpers
 * ------------------------------------------------------------------------ */

/* Read a whole text file into a NUL-terminated heap buffer (NULL if missing). */
static char* read_file(const char* path) {
    FILE* f = fopen(path, "rb");
    if (!f) {
        return NULL;
    }
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    char* buf = (char*)malloc((size_t)len + 1);
    if (buf && fread(buf, 1, (size_t)len, f) == (size_t)len) {
        buf[len] = '\0';
    } else {
        free(buf);
        buf = NULL;
    }
    fclose(f);
    return buf;
}

static void on_os_event(uint32_t event, const char* detail) {
    printf("[callback] event=%u detail=%s\n", event, detail ? detail : "(null)");
}

static void on_audio_event(uint32_t event, uint64_t track_id, uint32_t value) {
    printf("[audio] event=%u track=%llu value=%u\n", event, (unsigned long long)track_id,
           value);
}

static void run_command(OasisInstance* os, const char* cmd) {
    char* output = oasis_send_command(os, cmd);
    if (output) {
        printf("$ %s\n%s\n", cmd, output);
        oasis_free_string(output);
    }
}

int main(void) {
    /* Load the classic skin's full TOML set. Any NULL falls back to defaults;
     * the skin is only applied when manifest + layout + features are present. */
    char* manifest = read_file("skins/classic/skin.toml");
    char* layout = read_file("skins/classic/layout.toml");
    char* features = read_file("skins/classic/features.toml");
    char* theme = read_file("skins/classic/theme.toml");
    char* strings = read_file("skins/classic/strings.toml");

    printf("Creating OASIS_OS instance (480x272, skin %s)...\n",
           manifest ? "skins/classic" : "default");
    OasisInstance* os =
        oasis_create_full(480, 272, manifest, layout, features, theme, strings);
    /* The TOML strings are copied during creation and can be freed now. */
    free(manifest);
    free(layout);
    free(features);
    free(theme);
    free(strings);
    if (!os) {
        fprintf(stderr, "ERROR: oasis_create_full() returned NULL\n");
        return 1;
    }

    /* Callbacks: OS events and audio state changes. */
    oasis_register_callback(os, OASIS_CB_COMMAND_EXEC, on_os_event);
    oasis_register_callback(os, OASIS_CB_FILE_ACCESS, on_os_event);
    oasis_set_audio_callback(os, on_audio_event);

    /* Seed the virtual filesystem before the first tick. */
    const char* hello = "Hello from the C FFI demo!";
    oasis_add_vfs_file(os, "/home/hello.txt", (const uint8_t*)hello, (uint32_t)strlen(hello));

    /* Tick a few frames. */
    for (int i = 0; i < 10; i++) {
        oasis_tick(os, 1.0f / 60.0f);
    }

    /* Read the framebuffer (RGBA8, row-major) when it changed. */
    if (oasis_get_dirty(os)) {
        uint32_t w = 0, h = 0;
        const uint8_t* pixels = oasis_get_buffer(os, &w, &h);
        if (pixels) {
            printf("Framebuffer: %ux%u (%u bytes), first pixel RGBA (%u, %u, %u, %u)\n", w, h,
                   w * h * 4, pixels[0], pixels[1], pixels[2], pixels[3]);
        }
    }

    /* Terminal commands. */
    run_command(os, "cat /home/hello.txt");
    run_command(os, "ls /home");

    /* Gamepad-style input: move the dashboard selection right. */
    OasisInputEvent ev = {0};
    ev.event_type = OASIS_EVENT_BUTTON_PRESS;
    ev.key = OASIS_BUTTON_RIGHT;
    oasis_send_input(os, &ev);

    /* Raw key input: Ctrl+L (printable key -> OASIS_KEY_CHAR + codepoint). */
    OasisInputEvent key = {0};
    key.event_type = OASIS_EVENT_KEY;
    key.key = OASIS_KEY_CHAR;
    key.character = 'l';
    key.x = OASIS_MOD_CTRL;
    oasis_send_input(os, &key);
    oasis_tick(os, 1.0f / 60.0f);

    /* Audio: volume round-trip (fires the audio callback). */
    oasis_audio_set_volume(os, 42);
    printf("Audio volume: %u, playing: %s\n", oasis_audio_get_volume(os),
           oasis_audio_is_playing(os) ? "yes" : "no");

#ifdef OASIS_WITH_VIDEO
    printf("Video playing: %d\n", oasis_video_is_playing(os));
#endif

    oasis_destroy(os);
    printf("Done.\n");
    return 0;
}
