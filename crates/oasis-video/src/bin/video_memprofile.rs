//! Memory profiling binary for oasis-video decode pipeline.
//!
//! Decodes N frames from an MP4 and reports RSS (resident set size) to stdout
//! in a machine-readable format for CI assertion.
//!
//! Usage: video-memprofile <path.mp4> [--frames N] [--max-rss-kb N] [--loops N]

use std::env;
use std::fs;
use std::process;

use oasis_video::SoftwareVideoDecoder;

/// Read RSS from /proc/self/statm (Linux only). Returns KB.
fn rss_kb() -> u64 {
    let statm = match fs::read_to_string("/proc/self/statm") {
        Ok(s) => s,
        Err(_) => return 0,
    };
    // statm fields: size resident shared text lib data dt (in pages)
    let resident_pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let page_size = 4; // assume 4KB pages
    resident_pages * page_size
}

fn usage() -> ! {
    eprintln!("Usage: video-memprofile <file.mp4> [--frames N] [--max-rss-kb N] [--loops N]");
    eprintln!();
    eprintln!("Decodes up to N video frames (default: all) and reports peak RSS.");
    eprintln!("Machine-readable output on stdout: PEAK_RSS_KB=<val> FRAME_COUNT=<val>");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --frames N       Max frames per loop (default: all)");
    eprintln!("  --max-rss-kb N   Exit 1 if peak RSS exceeds N KB");
    eprintln!("  --loops N        Re-decode N times (seek to start), report RSS per loop");
    process::exit(1);
}

fn decode_loop(
    decoder: &mut SoftwareVideoDecoder,
    max_frames: u64,
    peak_rss: &mut u64,
    loop_idx: usize,
) -> u64 {
    let mut frame_count: u64 = 0;

    loop {
        if frame_count >= max_frames {
            break;
        }

        match decoder.next_video_frame() {
            Ok(Some(frame)) => {
                frame_count += 1;
                let (w, h) = (frame.width, frame.height);

                // Sample RSS every 10 frames to reduce overhead.
                if frame_count.is_multiple_of(10) || frame_count == 1 {
                    let current = rss_kb();
                    if current > *peak_rss {
                        *peak_rss = current;
                    }
                    if loop_idx == 0 {
                        eprintln!(
                            "  frame {frame_count}: {w}x{h} ts={:.3}s rss={current} KB",
                            frame.timestamp_secs
                        );
                    }
                }
            },
            Ok(None) => break,
            Err(e) => {
                eprintln!("decode error at frame {frame_count}: {e}");
                break;
            },
        }
    }

    frame_count
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
    }

    let mut mp4_path = None;
    let mut max_frames: u64 = u64::MAX;
    let mut max_rss_limit: Option<u64> = None;
    let mut loops: u64 = 1;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--frames" {
            i += 1;
            if i >= args.len() {
                eprintln!("error: --frames requires a value");
                process::exit(1);
            }
            max_frames = args[i].parse().unwrap_or_else(|_| {
                eprintln!("error: invalid frame count: {}", args[i]);
                process::exit(1);
            });
        } else if args[i] == "--max-rss-kb" {
            i += 1;
            if i >= args.len() {
                eprintln!("error: --max-rss-kb requires a value");
                process::exit(1);
            }
            max_rss_limit = Some(args[i].parse().unwrap_or_else(|_| {
                eprintln!("error: invalid max-rss-kb: {}", args[i]);
                process::exit(1);
            }));
        } else if args[i] == "--loops" {
            i += 1;
            if i >= args.len() {
                eprintln!("error: --loops requires a value");
                process::exit(1);
            }
            loops = args[i].parse().unwrap_or_else(|_| {
                eprintln!("error: invalid loop count: {}", args[i]);
                process::exit(1);
            });
        } else if args[i].starts_with('-') {
            eprintln!("error: unknown option: {}", args[i]);
            usage();
        } else {
            mp4_path = Some(args[i].clone());
        }
        i += 1;
    }

    let mp4_path = mp4_path.unwrap_or_else(|| usage());

    eprintln!("Loading: {mp4_path}");
    let data = fs::read(&mp4_path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {mp4_path}: {e}");
        process::exit(1);
    });
    eprintln!("File size: {} bytes", data.len());

    let mut decoder = SoftwareVideoDecoder::open(data).unwrap_or_else(|e| {
        eprintln!("error: cannot open MP4: {e}");
        process::exit(1);
    });

    let rss_before = rss_kb();
    eprintln!("RSS before decode: {rss_before} KB");

    let mut peak_rss: u64 = rss_before;
    let mut total_frames: u64 = 0;

    for loop_idx in 0..loops {
        if loop_idx > 0
            && let Err(e) = decoder.seek(0.0)
        {
            eprintln!("seek to start failed on loop {loop_idx}: {e}");
            break;
        }

        let frames = decode_loop(&mut decoder, max_frames, &mut peak_rss, loop_idx as usize);
        total_frames += frames;

        let loop_rss = rss_kb();
        if loop_rss > peak_rss {
            peak_rss = loop_rss;
        }

        // Machine-readable per-loop RSS for leak detection.
        println!("LOOP_RSS_KB={loop_rss}");
        if loops > 1 {
            eprintln!(
                "  loop {}: {frames} frames, RSS={loop_rss} KB",
                loop_idx + 1,
            );
        }
    }

    // Final RSS sample.
    let rss_after = rss_kb();
    if rss_after > peak_rss {
        peak_rss = rss_after;
    }

    eprintln!();
    eprintln!("--- Summary ---");
    eprintln!("Loops: {loops}");
    eprintln!("Total frames decoded: {total_frames}");
    eprintln!("RSS before: {rss_before} KB");
    eprintln!("RSS after:  {rss_after} KB");
    eprintln!("Peak RSS:   {peak_rss} KB");
    let (vw, vh) = decoder.video_size();
    eprintln!("Video size: {vw}x{vh}");

    // Machine-readable output for CI.
    println!("PEAK_RSS_KB={peak_rss}");
    println!("FRAME_COUNT={total_frames}");

    // Assert max RSS if threshold was given.
    if let Some(limit) = max_rss_limit {
        if peak_rss > limit {
            eprintln!("FAIL: peak RSS {peak_rss} KB exceeds limit {limit} KB");
            process::exit(1);
        }
        eprintln!("PASS: peak RSS {peak_rss} KB within limit {limit} KB");
    }
}
