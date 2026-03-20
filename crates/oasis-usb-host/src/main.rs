//! USB host for PSP thin client.
//!
//! Connects to the PSP USB device driver, runs echo tests, measures
//! throughput, and (later) streams frames + receives input.
//!
//! Usage: sudo cargo run -p oasis-usb-host
//! (sudo required for USB access, or set up udev rules)

#[allow(dead_code)]
mod device;
#[allow(dead_code)]
mod protocol;

use device::PspDevice;
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

    // Don't clear_halt before reading — it may cancel PSP's pending transfers

    // Read the "PSP READY" message
    print!("Waiting for PSP READY... ");
    match psp.read_ready() {
        Ok(msg) => println!("{msg}"),
        Err(e) => {
            eprintln!("failed: {e}");
            std::process::exit(1);
        },
    }

    // Give PSP time to process send_complete and queue its first recv
    println!("Waiting for PSP echo mode...");
    std::thread::sleep(Duration::from_secs(2));

    println!();
    test_echo(&psp);
    println!();
    test_multi_packet(&psp);
    println!();
    test_throughput(&psp);
    println!();

    println!("All tests complete.");
}

fn test_echo(psp: &PspDevice) {
    println!("--- Echo Test ---");
    let data = b"Hello from Rust USB host!";
    match psp.echo(data) {
        Ok(true) => println!("  PASS: {} bytes echoed", data.len()),
        Ok(false) => println!("  FAIL: data mismatch"),
        Err(e) => println!("  FAIL: {e}"),
    }
}

fn test_multi_packet(psp: &PspDevice) {
    println!("--- Multi-Packet Test ---");
    let mut pass = 0;
    let total = 10;
    for i in 0..total {
        let size = 10 + i * 50; // 10, 60, 110, ..., 460
        let data: Vec<u8> = (0..size).map(|j| (j & 0xFF) as u8).collect();
        match psp.echo(&data) {
            Ok(true) => pass += 1,
            Ok(false) => println!("  [{i}] FAIL: mismatch ({size} bytes)"),
            Err(e) => {
                println!("  [{i}] FAIL: {e} ({size} bytes)");
                psp.clear_halt();
            },
        }
    }
    println!("  {pass}/{total} passed");
}

fn test_throughput(psp: &PspDevice) {
    println!("--- Throughput Test ---");

    // Use 500 bytes (not 512) to avoid ZLP issue
    let payload: Vec<u8> = (0..500).map(|i| (i & 0xFF) as u8).collect();
    let duration = Duration::from_secs(5);
    let start = Instant::now();
    let mut count: u64 = 0;
    let mut errors: u64 = 0;

    while start.elapsed() < duration {
        match psp.echo(&payload) {
            Ok(true) => count += 1,
            Ok(false) => errors += 1,
            Err(_) => {
                errors += 1;
                psp.clear_halt();
                if errors > 10 {
                    println!("  Too many errors, stopping");
                    break;
                }
            },
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let total_bytes = count * payload.len() as u64 * 2; // send + receive
    let throughput_kb = total_bytes as f64 / elapsed / 1024.0;
    let throughput_mb = throughput_kb / 1024.0;

    println!("  {count} round-trips in {elapsed:.1}s");
    println!("  {throughput_kb:.0} KB/s ({throughput_mb:.1} MB/s), {errors} errors");

    // Latency estimate
    if count > 0 {
        let avg_us = (elapsed * 1_000_000.0) / count as f64;
        println!("  Avg round-trip: {avg_us:.0} µs");
    }
}
