//! Terminal scrollback benchmarks: trimming a burst of output into the
//! capped buffer, and syncing the scrollback into the windowed Terminal
//! runner after a single-line append.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use oasis_core::apps::AppRunner;
use oasis_core::dashboard::AppEntry;
use oasis_core::terminal_sdi::{MAX_OUTPUT_LINES, trim_scrollback};
use oasis_core::vfs::MemoryVfs;
use oasis_types::backend::Color;

/// A full scrollback followed by a 5000-line command output burst.
fn full_plus_burst() -> Vec<String> {
    let mut lines: Vec<String> = (0..MAX_OUTPUT_LINES).map(|i| format!("old {i}")).collect();
    lines.extend((0..5000).map(|i| format!("new output line {i}")));
    lines
}

fn bench_trim(c: &mut Criterion) {
    let mut group = c.benchmark_group("scrollback_trim_5000_into_2000");
    // The pre-fix algorithm, kept for comparison.
    group.bench_function("remove0_loop", |b| {
        b.iter_batched(
            full_plus_burst,
            |mut lines| {
                while lines.len() > MAX_OUTPUT_LINES {
                    lines.remove(0);
                }
                lines
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("drain", |b| {
        b.iter_batched(
            full_plus_burst,
            |mut lines| {
                trim_scrollback(&mut lines);
                lines
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

fn terminal_runner() -> AppRunner {
    let entry = AppEntry {
        title: "Terminal".to_string(),
        path: "/apps/terminal".to_string(),
        icon_png: Vec::new(),
        color: Color::rgb(0, 0, 0),
    };
    AppRunner::launch(&entry, &MemoryVfs::new())
}

fn bench_sync(c: &mut Criterion) {
    let mut group = c.benchmark_group("scrollback_sync_append_one");
    let mut output: Vec<String> = (0..MAX_OUTPUT_LINES).map(|i| format!("line {i}")).collect();

    // The pre-fix desktop path: clone the scrollback, push the prompt,
    // hand it to set_lines (which re-clones into the runner mirror).
    let mut runner = terminal_runner();
    let mut n = 0usize;
    group.bench_function("full_clone", |b| {
        b.iter(|| {
            output.push(format!("appended {n}"));
            n += 1;
            trim_scrollback(&mut output);
            let mut lines = output.clone();
            lines.push("> ".to_string());
            runner.set_lines(lines, 0);
        });
    });

    let mut runner = terminal_runner();
    runner.sync_terminal_lines(&output, "> ", 0);
    group.bench_function("incremental", |b| {
        b.iter(|| {
            output.push(format!("appended {n}"));
            n += 1;
            trim_scrollback(&mut output);
            runner.sync_terminal_lines(&output, "> ", 0);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_trim, bench_sync);
criterion_main!(benches);
