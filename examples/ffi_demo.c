/**
 * FFI Demo -- Embed OASIS_OS in a C application.
 *
 * Build the shared library first:
 *   cargo build --release -p oasis-ffi
 *
 * Then compile and run this demo:
 *   gcc -o ffi_demo examples/ffi_demo.c \
 *       -L target/release -loasis_ffi -Wl,-rpath,target/release
 *   ./ffi_demo
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>

/* Forward declarations matching oasis-ffi exports. */
typedef struct OasisInstance OasisInstance;

typedef struct {
    uint32_t event_type;
    int32_t  x, y;
    uint32_t key;
    uint32_t character;
} OasisInputEvent;

extern OasisInstance* oasis_create(
    uint32_t width, uint32_t height,
    const char* skin_toml, const char* layout_toml,
    const char* features_toml
);
extern void oasis_destroy(OasisInstance* handle);
extern void oasis_tick(OasisInstance* handle, float delta_seconds);
extern const uint8_t* oasis_get_buffer(
    OasisInstance* handle, uint32_t* out_w, uint32_t* out_h
);
extern bool oasis_get_dirty(OasisInstance* handle);
extern void oasis_send_input(OasisInstance* handle, const OasisInputEvent* ev);
extern char* oasis_send_command(OasisInstance* handle, const char* cmd);
extern void oasis_free_string(char* ptr);
extern void oasis_add_vfs_file(
    OasisInstance* handle, const char* path,
    const uint8_t* data, uint32_t data_len
);

#define OASIS_EVENT_BUTTON_PRESS 2
#define OASIS_BUTTON_CONFIRM     4

int main(void) {
    printf("Creating OASIS_OS instance (480x272)...\n");

    /* Create instance with default skin. */
    OasisInstance* os = oasis_create(480, 272, NULL, NULL, NULL);
    if (!os) {
        fprintf(stderr, "ERROR: oasis_create() returned NULL\n");
        return 1;
    }

    /* Add a file to the virtual filesystem. */
    const char* hello = "Hello from the C FFI demo!";
    oasis_add_vfs_file(os, "/home/hello.txt",
        (const uint8_t*)hello, (uint32_t)strlen(hello));

    /* Tick a few frames to let the OS initialize. */
    for (int i = 0; i < 10; i++) {
        oasis_tick(os, 1.0f / 60.0f);
    }

    /* Read the framebuffer. */
    uint32_t w = 0, h = 0;
    const uint8_t* pixels = oasis_get_buffer(os, &w, &h);
    printf("Framebuffer: %ux%u (%u bytes)\n", w, h, w * h * 4);
    printf("First pixel RGBA: (%u, %u, %u, %u)\n",
        pixels[0], pixels[1], pixels[2], pixels[3]);

    /* Execute a terminal command. */
    char* output = oasis_send_command(os, "cat /home/hello.txt");
    if (output) {
        printf("Command output: %s\n", output);
        oasis_free_string(output);
    }

    /* List files. */
    output = oasis_send_command(os, "ls /home");
    if (output) {
        printf("File listing:\n%s\n", output);
        oasis_free_string(output);
    }

    /* Simulate a button press. */
    OasisInputEvent ev = {0};
    ev.event_type = OASIS_EVENT_BUTTON_PRESS;
    ev.key = OASIS_BUTTON_CONFIRM;
    oasis_send_input(os, &ev);
    oasis_tick(os, 1.0f / 60.0f);

    /* Clean up. */
    oasis_destroy(os);
    printf("Done.\n");
    return 0;
}
