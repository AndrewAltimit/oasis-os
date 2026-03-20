//! USB host for PSP thin client.
//!
//! Connects to the PSP USB device driver, streams RGB565 frames to the
//! PSP display, and receives controller input state back.
//!
//! Usage: sudo cargo run -p oasis-usb-host
//! (sudo required for USB access, or set up udev rules)

#[allow(dead_code)]
mod device;
#[allow(dead_code)]
mod protocol;

use device::PspDevice;
use protocol::InputState;
use std::time::{Duration, Instant};

fn main() {
    println!("=== OASIS USB Host ===\n");

    let psp = match PspDevice::open() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!("Is the PSP running USBCLIENT and connected?");
            std::process::exit(1);
        },
    };

    // Read the "PSP READY" message
    print!("Waiting for PSP READY... ");
    match psp.read_ready() {
        Ok(msg) => println!("{msg}"),
        Err(e) => {
            eprintln!("failed: {e}");
            std::process::exit(1);
        },
    }

    // Give PSP time to queue first recv
    println!("Waiting for PSP thin-client mode...");
    std::thread::sleep(Duration::from_secs(2));

    println!();
    test_input(&psp);
    println!();
    test_frame(&psp);
    println!();
    test_streaming(&psp);
    println!();

    println!("All tests complete.");
}

/// Test GET_INPUT: send a single input poll and print the response.
fn test_input(psp: &PspDevice) {
    println!("--- Input Test ---");
    match psp.get_input() {
        Ok(input) => {
            println!("  PASS: {}", input.display());
        },
        Err(e) => println!("  FAIL: {e}"),
    }
}

/// Test single frame: send a solid red frame, verify display + input response.
fn test_frame(psp: &PspDevice) {
    println!("--- Frame Test ---");

    // Generate solid red frame (RGB565: R=31, G=0, B=0 -> 0xF800)
    let red_pixel: u16 = 0xF800;
    let frame = generate_solid_frame(red_pixel);

    match psp.send_frame(&frame, 0) {
        Ok(input) => {
            println!("  PASS: solid red frame sent ({} bytes)", frame.len());
            println!("  Input: {}", input.display());
        },
        Err(e) => {
            println!("  FAIL: {e}");
            psp.clear_halt();
        },
    }
}

/// Test streaming: send 100 frames of cycling colors, measure FPS, print input.
fn test_streaming(psp: &PspDevice) {
    println!("--- Streaming Test (100 frames) ---");

    let colors: [u16; 3] = [
        0xF800, // Red
        0x07E0, // Green
        0x001F, // Blue
    ];

    let start = Instant::now();
    let mut errors = 0u32;
    let mut last_input = InputState::default();

    let mut last_buttons = 0u32;
    for i in 0u8..100 {
        let color = colors[i as usize % 3];
        let frame = generate_solid_frame(color);

        match psp.send_frame(&frame, i) {
            Ok(input) => {
                last_input = input;
                // Print on button state changes only
                if input.buttons != last_buttons {
                    println!("  Frame {i}: {}", input.display());
                    last_buttons = input.buttons;
                }
            },
            Err(e) => {
                errors += 1;
                println!("  Frame {i}: ERROR {e}");
                psp.clear_halt();
                if errors > 5 {
                    println!("  Too many errors, stopping");
                    break;
                }
            },
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let frames_sent = 100 - errors;
    let fps = frames_sent as f64 / elapsed;
    let total_bytes = frames_sent as u64 * protocol::FRAME_SIZE_STRIDE as u64;
    let throughput_mb = total_bytes as f64 / elapsed / 1024.0 / 1024.0;

    println!();
    println!("  {frames_sent} frames in {elapsed:.2}s = {fps:.1} FPS");
    println!("  {throughput_mb:.1} MB/s frame data, {errors} errors");
    println!("  Last input: {}", last_input.display());
}

/// Generate a solid-color stride-padded RGB565 frame.
fn generate_solid_frame(color_rgb565: u16) -> Vec<u8> {
    let size = protocol::FRAME_SIZE_STRIDE;
    let mut frame = vec![0u8; size];
    let pixel = color_rgb565.to_le_bytes();

    // Fill stride-padded buffer: 512 pixels/row x 272 rows
    for row in 0..protocol::DISPLAY_HEIGHT as usize {
        let row_offset = row * protocol::FRAME_STRIDE as usize * 2;
        // Fill visible 480 pixels
        for col in 0..protocol::DISPLAY_WIDTH as usize {
            let offset = row_offset + col * 2;
            frame[offset] = pixel[0];
            frame[offset + 1] = pixel[1];
        }
        // Padding columns (480..512) remain zero (black)
    }

    frame
}
