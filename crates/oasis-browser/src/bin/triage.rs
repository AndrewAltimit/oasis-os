//! `oasis-browser-triage` — a local-only crawler for bucket-sorting
//! real-world HTML files by how they exercise the engine.
//!
//! This is the "crawler script" from the real-world-compatibility
//! measurement epic. It is **not** part of CI — nothing here is
//! gated. The intended workflow is:
//!
//! 1. Curate a directory of real-world HTML snapshots (curl the
//!    top-500 Alexa sites, or `wget --mirror` a specific app you
//!    care about).
//! 2. Run `cargo run -p oasis-browser --bin oasis-browser-triage --
//!    --input dir/ --output report.md`.
//! 3. The tool walks the directory, runs each `.html` file through
//!    `parse → cascade → layout → paint`, and categorizes the
//!    outcome: `ok`, `panic`, `slow`, `empty-layout`, or
//!    `no-draw-calls`.
//! 4. The report lands as Markdown so the output can be pasted into
//!    an issue or a PR without further post-processing.
//!
//! Design decisions:
//!
//! - **Local files only.** We don't pull in an HTTP client here.
//!   The backlog explicitly says the tool should be "local triage",
//!   and network fetching adds TLS / proxy / throttling concerns
//!   that belong in the real browser, not in a diagnostic tool.
//!   If you want to crawl the web, `curl` or `wget` the pages
//!   first, then run the triage tool over the resulting directory.
//! - **Panic catching via `std::panic::catch_unwind`.** The full
//!   pipeline is `UnwindSafe` because we clone the HTML input and
//!   rebuild every intermediate structure per page. A panicking
//!   page is recorded, not fatal.
//! - **No hang protection.** We measure wall-clock time and flag
//!   anything over a soft threshold as "slow", but we don't kill
//!   runaway pages — that requires a worker thread and an abort
//!   protocol that would complicate this tool out of proportion
//!   to its value. If you need hang protection, run the tool
//!   under `timeout(1)`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oasis_browser::SimpleTextMeasurer;
use oasis_browser::internals::{
    CascadeContext, PaintViewport, Stylesheet, Tokenizer, TreeBuilder, build_layout_tree,
    default_stylesheet, paint_page, style_tree,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Bucket {
    Ok,
    Panic,
    PaintError,
    IoError,
    Slow,
    EmptyLayout,
    NoDrawCalls,
}

impl Bucket {
    fn label(&self) -> &'static str {
        match self {
            Bucket::Ok => "ok",
            Bucket::Panic => "panic",
            Bucket::PaintError => "paint-error",
            Bucket::IoError => "io-error",
            Bucket::Slow => "slow",
            Bucket::EmptyLayout => "empty-layout",
            Bucket::NoDrawCalls => "no-draw-calls",
        }
    }
}

struct Outcome {
    path: PathBuf,
    bucket: Bucket,
    duration: Duration,
    notes: String,
}

struct Args {
    input: PathBuf,
    output: Option<PathBuf>,
    slow_budget_ms: u64,
    viewport_w: f32,
    viewport_h: f32,
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut slow_budget_ms: u64 = 2000;
    let mut viewport_w: f32 = 800.0;
    let mut viewport_h: f32 = 600.0;

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut it = raw.into_iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--input" => {
                input = Some(PathBuf::from(it.next().ok_or("--input requires a value")?));
            },
            "--output" => {
                output = Some(PathBuf::from(it.next().ok_or("--output requires a value")?));
            },
            "--slow-budget-ms" => {
                slow_budget_ms = it
                    .next()
                    .ok_or("--slow-budget-ms requires a value")?
                    .parse()
                    .map_err(|e| format!("--slow-budget-ms: {e}"))?;
            },
            "--viewport" => {
                let v = it.next().ok_or("--viewport requires WxH")?;
                let (w, h) = v
                    .split_once('x')
                    .ok_or("--viewport expects WxH, e.g. 800x600")?;
                viewport_w = w.parse().map_err(|e| format!("--viewport W: {e}"))?;
                viewport_h = h.parse().map_err(|e| format!("--viewport H: {e}"))?;
            },
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            },
            other => return Err(format!("unknown flag: {other}")),
        }
    }

    Ok(Args {
        input: input.ok_or("--input is required")?,
        output,
        slow_budget_ms,
        viewport_w,
        viewport_h,
    })
}

fn print_help() {
    println!(
        "oasis-browser-triage — bucket-sort real-world HTML by engine behaviour

USAGE:
    oasis-browser-triage --input <FILE_OR_DIR> [--output report.md] \\
                         [--slow-budget-ms 2000] [--viewport 800x600]

DESCRIPTION:
    Walks the given directory (or reads the given file), runs each
    `.html` page through the full parse → cascade → layout → paint
    pipeline, and writes a Markdown report bucketing each page by
    outcome (ok, panic, slow, empty-layout, no-draw-calls).

    This tool is local-only: it does not fetch URLs. Populate an
    input directory with `curl`-saved HTML snapshots first.

OPTIONS:
    --input <path>         File or directory to scan (required).
    --output <path>        Report path (default: stdout).
    --slow-budget-ms <ms>  Mark pages exceeding this as `slow`.
    --viewport <WxH>       Viewport used for layout + paint.
"
    );
}

/// Walk `root`; return every file with an `.html` extension.
fn collect_html(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        if root.extension().is_some_and(|e| e == "html") {
            out.push(root.to_path_buf());
        }
        return out;
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "oasis-browser-triage: cannot read directory {}: {e}",
                root.display()
            );
            return out;
        },
    };
    let mut stack: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            match fs::read_dir(&p) {
                Ok(entries) => {
                    stack.extend(entries.filter_map(|e| e.ok().map(|e| e.path())));
                },
                Err(e) => {
                    eprintln!(
                        "oasis-browser-triage: cannot read directory {}: {e}",
                        p.display()
                    );
                },
            }
        } else if p.extension().is_some_and(|e| e == "html") {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Run the full pipeline on one HTML file and return an `Outcome`.
fn triage_one(path: &Path, w: f32, h: f32, slow_budget: Duration) -> Outcome {
    let html = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return Outcome {
                path: path.to_path_buf(),
                bucket: Bucket::IoError,
                duration: Duration::ZERO,
                notes: format!("read error: {e}"),
            };
        },
    };

    let start = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let tokens = Tokenizer::new(&html).tokenize();
        let doc = TreeBuilder::build(tokens);
        let ua = default_stylesheet();
        let sheets: Vec<&Stylesheet> = vec![&ua];
        let ctx = CascadeContext::default();
        let styles = style_tree(&doc, &sheets, &[], &ctx);
        let layout = build_layout_tree(
            &doc,
            &styles,
            &SimpleTextMeasurer,
            w,
            h,
            None,
            &HashMap::new(),
        );
        let layout_box_count = count_boxes(&layout);
        let mut backend = oasis_test_backend_stub::NullBackend::new(w as u32, h as u32);
        let vp = PaintViewport {
            scroll_y: 0.0,
            scroll_x: 0.0,
            x: 0,
            y: 0,
            width: w,
            height: h,
            visible_height: h,
            focused_node: None,
        };
        let paint_result = paint_page(&layout, &mut backend, vp, &HashMap::new());
        (layout_box_count, backend.call_count, paint_result.is_ok())
    }));
    let duration = start.elapsed();

    match result {
        Ok((box_count, draw_calls, paint_ok)) => {
            let mut bucket = if !paint_ok {
                Bucket::PaintError
            } else if box_count <= 1 {
                Bucket::EmptyLayout
            } else if draw_calls == 0 {
                Bucket::NoDrawCalls
            } else {
                Bucket::Ok
            };
            if matches!(bucket, Bucket::Ok) && duration > slow_budget {
                bucket = Bucket::Slow;
            }
            let notes = format!("boxes={box_count} draws={draw_calls}");
            Outcome {
                path: path.to_path_buf(),
                bucket,
                duration,
                notes,
            }
        },
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "<opaque panic payload>".to_string()
            };
            Outcome {
                path: path.to_path_buf(),
                bucket: Bucket::Panic,
                duration,
                notes: msg,
            }
        },
    }
}

fn count_boxes(root: &oasis_browser::internals::LayoutBox) -> usize {
    let mut n = 1usize;
    for c in &root.children {
        n += count_boxes(c);
    }
    n
}

/// Write a Markdown report that buckets each outcome by category.
fn render_report(outcomes: &[Outcome], args: &Args) -> String {
    let mut by_bucket: BTreeMap<String, Vec<&Outcome>> = BTreeMap::new();
    for o in outcomes {
        by_bucket
            .entry(o.bucket.label().to_string())
            .or_default()
            .push(o);
    }

    let total = outcomes.len();
    let ok_count = by_bucket.get("ok").map(|v| v.len()).unwrap_or(0);
    let panic_count = by_bucket.get("panic").map(|v| v.len()).unwrap_or(0);
    let paint_err_count = by_bucket.get("paint-error").map(|v| v.len()).unwrap_or(0);
    let io_err_count = by_bucket.get("io-error").map(|v| v.len()).unwrap_or(0);
    let slow_count = by_bucket.get("slow").map(|v| v.len()).unwrap_or(0);
    let empty_count = by_bucket.get("empty-layout").map(|v| v.len()).unwrap_or(0);
    let no_draw_count = by_bucket.get("no-draw-calls").map(|v| v.len()).unwrap_or(0);

    let total_dur: Duration = outcomes.iter().map(|o| o.duration).sum();
    let mean_ms = if total > 0 {
        (total_dur.as_millis() as f64) / (total as f64)
    } else {
        0.0
    };

    let mut out = String::new();
    out.push_str("# oasis-browser triage report\n\n");
    out.push_str(&format!(
        "- **Input:** `{}`\n- **Viewport:** {}x{}\n- **Slow budget:** {} ms\n\n",
        args.input.display(),
        args.viewport_w as u32,
        args.viewport_h as u32,
        args.slow_budget_ms
    ));
    out.push_str(&format!(
        "Processed **{total}** pages, **{mean_ms:.1} ms** average.\n\n"
    ));
    out.push_str("## Summary\n\n");
    out.push_str("| bucket | count | % |\n|---|---|---|\n");
    for (label, count) in [
        ("ok", ok_count),
        ("slow", slow_count),
        ("empty-layout", empty_count),
        ("no-draw-calls", no_draw_count),
        ("paint-error", paint_err_count),
        ("io-error", io_err_count),
        ("panic", panic_count),
    ] {
        let pct = if total > 0 {
            (count as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        out.push_str(&format!("| {label} | {count} | {pct:.1}% |\n"));
    }
    out.push('\n');

    for label in [
        "panic",
        "paint-error",
        "io-error",
        "slow",
        "empty-layout",
        "no-draw-calls",
        "ok",
    ] {
        let Some(entries) = by_bucket.get(label) else {
            continue;
        };
        out.push_str(&format!("## {label}\n\n"));
        for o in entries {
            out.push_str(&format!(
                "- `{}` — {} ms — {}\n",
                o.path.display(),
                o.duration.as_millis(),
                o.notes
            ));
        }
        out.push('\n');
    }

    out
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oasis-browser-triage: {e}\n");
            print_help();
            std::process::exit(2);
        },
    };

    let files = collect_html(&args.input);
    if files.is_empty() {
        eprintln!(
            "oasis-browser-triage: no .html files found under {}",
            args.input.display()
        );
        std::process::exit(1);
    }

    eprintln!("oasis-browser-triage: processing {} pages...", files.len());
    let slow_budget = Duration::from_millis(args.slow_budget_ms);
    let mut outcomes = Vec::with_capacity(files.len());
    for (i, path) in files.iter().enumerate() {
        eprint!("\r  [{:4}/{}] {}", i + 1, files.len(), path.display());
        let outcome = triage_one(path, args.viewport_w, args.viewport_h, slow_budget);
        outcomes.push(outcome);
    }
    eprintln!("\n");

    let report = render_report(&outcomes, &args);
    match &args.output {
        Some(path) => {
            if let Err(e) = fs::write(path, &report) {
                eprintln!(
                    "oasis-browser-triage: failed to write {}: {e}",
                    path.display()
                );
                std::process::exit(1);
            }
            eprintln!("wrote report to {}", path.display());
        },
        None => print!("{report}"),
    }
}

/// A tiny SdiCore stand-in that counts draw calls without recording
/// them, so we don't pull `oasis-test-backend` into the binary's
/// runtime dependency graph.
mod oasis_test_backend_stub {
    use oasis_types::backend::{
        Color, SdiAlpha, SdiBatch, SdiClipTransform, SdiCore, SdiGradients, SdiRenderTarget,
        SdiShapes, SdiText, SdiTextures, SdiVector, TextureId, bitmap_measure_text,
    };
    use oasis_types::error::Result;

    pub struct NullBackend {
        pub call_count: usize,
        viewport_w: u32,
        viewport_h: u32,
    }

    impl NullBackend {
        pub fn new(viewport_w: u32, viewport_h: u32) -> Self {
            Self {
                call_count: 0,
                viewport_w,
                viewport_h,
            }
        }
    }

    impl SdiCore for NullBackend {
        fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn clear(&mut self, _c: Color) -> Result<()> {
            self.call_count += 1;
            Ok(())
        }
        fn blit(&mut self, _t: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            self.call_count += 1;
            Ok(())
        }
        fn fill_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32, _c: Color) -> Result<()> {
            self.call_count += 1;
            Ok(())
        }
        fn draw_text(&mut self, _t: &str, _x: i32, _y: i32, _fs: u16, _c: Color) -> Result<()> {
            self.call_count += 1;
            Ok(())
        }
        fn swap_buffers(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> Result<TextureId> {
            Ok(TextureId(1))
        }
        fn destroy_texture(&mut self, _t: TextureId) -> Result<()> {
            Ok(())
        }
        fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> Result<()> {
            Ok(())
        }
        fn measure_text(&self, text: &str, font_size: u16) -> u32 {
            bitmap_measure_text(text, font_size)
        }
        fn read_pixels(&self, _x: i32, _y: i32, w: u32, h: u32) -> Result<Vec<u8>> {
            Ok(vec![0; (w as usize) * (h as usize) * 4])
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl SdiShapes for NullBackend {}
    impl SdiGradients for NullBackend {}
    impl SdiAlpha for NullBackend {
        fn viewport_size(&self) -> (u32, u32) {
            (self.viewport_w, self.viewport_h)
        }
    }
    impl SdiText for NullBackend {}
    impl SdiTextures for NullBackend {}
    impl SdiClipTransform for NullBackend {}
    impl SdiVector for NullBackend {}
    impl SdiBatch for NullBackend {}
    impl SdiRenderTarget for NullBackend {}
}
