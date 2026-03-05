//! Benchmarks for the oasis-video decode pipeline.

use criterion::{Criterion, criterion_group, criterion_main};

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/test_320x240_2s.mp4")
}

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(fixture_path()).expect("fixture file missing")
}

#[cfg(feature = "h264")]
fn bench_demux_open(c: &mut Criterion) {
    let data = fixture_bytes();
    c.bench_function("demux_open", |b| {
        b.iter(|| {
            let _ = oasis_video::SoftwareVideoDecoder::open(data.clone()).unwrap();
        });
    });
}

#[cfg(feature = "h264")]
fn bench_video_decode_throughput(c: &mut Criterion) {
    let data = fixture_bytes();
    c.bench_function("video_decode_throughput", |b| {
        b.iter(|| {
            let mut dec = oasis_video::SoftwareVideoDecoder::open(data.clone()).unwrap();
            let mut count = 0u32;
            while let Ok(Some(_frame)) = dec.next_video_frame() {
                count += 1;
            }
            count
        });
    });
}

#[cfg(feature = "h264")]
fn bench_audio_decode_throughput(c: &mut Criterion) {
    let data = fixture_bytes();
    c.bench_function("audio_decode_throughput", |b| {
        b.iter(|| {
            let mut dec = oasis_video::SoftwareVideoDecoder::open(data.clone()).unwrap();
            let mut count = 0u32;
            while let Ok(Some(_chunk)) = dec.next_audio_samples() {
                count += 1;
            }
            count
        });
    });
}

fn bench_yuv420_to_rgba(c: &mut Criterion) {
    let mut group = c.benchmark_group("yuv420_to_rgba");

    for (w, h) in [(320u32, 240u32), (640, 480)] {
        let label = format!("{w}x{h}");
        let stride_y = w as usize;
        let stride_uv = (w / 2) as usize;
        let y = vec![128u8; stride_y * h as usize];
        let u = vec![128u8; stride_uv * (h as usize / 2)];
        let v = vec![128u8; stride_uv * (h as usize / 2)];

        group.bench_function(&label, |b| {
            b.iter(|| oasis_video::yuv::yuv420_to_rgba(&y, &u, &v, w, h, stride_y, stride_uv));
        });
    }

    group.finish();
}

#[cfg(feature = "h264")]
fn bench_seek_latency(c: &mut Criterion) {
    let data = fixture_bytes();
    c.bench_function("seek_latency", |b| {
        b.iter_batched(
            || {
                let mut dec = oasis_video::SoftwareVideoDecoder::open(data.clone()).unwrap();
                // Prime the decoder with one frame.
                let _ = dec.next_video_frame();
                dec
            },
            |mut dec| {
                let _ = dec.seek(1.0);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

#[cfg(feature = "no-std-demux")]
fn bench_demux_lite_open(c: &mut Criterion) {
    let data = fixture_bytes();
    c.bench_function("demux_lite_open", |b| {
        b.iter(|| {
            let cursor = std::io::Cursor::new(data.clone());
            let _ = oasis_video::demux_lite::Mp4Lite::open(cursor).unwrap();
        });
    });
}

#[cfg(feature = "no-std-demux")]
fn bench_demux_lite_read_samples(c: &mut Criterion) {
    let data = fixture_bytes();
    c.bench_function("demux_lite_read_samples", |b| {
        b.iter(|| {
            let cursor = std::io::Cursor::new(data.clone());
            let mut mp4 = oasis_video::demux_lite::Mp4Lite::open(cursor).unwrap();
            let mut count = 0u32;
            while let Ok(Some(_)) = mp4.next_video_sample() {
                count += 1;
            }
            while let Ok(Some(_)) = mp4.next_audio_sample() {
                count += 1;
            }
            count
        });
    });
}

// Build the benchmark group based on enabled features.
#[cfg(feature = "h264")]
criterion_group!(
    h264_benches,
    bench_demux_open,
    bench_video_decode_throughput,
    bench_audio_decode_throughput,
    bench_seek_latency,
);

#[cfg(not(feature = "h264"))]
criterion_group!(h264_benches, bench_yuv420_to_rgba);

criterion_group!(common_benches, bench_yuv420_to_rgba);

#[cfg(feature = "no-std-demux")]
criterion_group!(
    lite_benches,
    bench_demux_lite_open,
    bench_demux_lite_read_samples,
);

#[cfg(all(feature = "h264", feature = "no-std-demux"))]
criterion_main!(h264_benches, common_benches, lite_benches);

#[cfg(all(feature = "h264", not(feature = "no-std-demux")))]
criterion_main!(h264_benches, common_benches);

#[cfg(all(not(feature = "h264"), feature = "no-std-demux"))]
criterion_main!(common_benches, lite_benches);

#[cfg(all(not(feature = "h264"), not(feature = "no-std-demux")))]
criterion_main!(common_benches);
