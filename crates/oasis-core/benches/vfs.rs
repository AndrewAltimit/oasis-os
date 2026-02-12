//! Benchmarks for VFS (MemoryVfs) operations.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_core::vfs::MemoryVfs;
use oasis_core::vfs::Vfs;

fn bench_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("vfs_write");

    let data_1k = vec![0xABu8; 1_024];
    let data_10k = vec![0xCDu8; 10_240];

    for (n_files, data, data_label) in [
        (100, &data_1k, "1KB"),
        (1_000, &data_1k, "1KB"),
        (100, &data_10k, "10KB"),
    ] {
        let label = format!("{n_files}x{data_label}");
        let paths: Vec<String> = (0..n_files)
            .map(|i| format!("/data/file_{i}.bin"))
            .collect();

        group.bench_function(BenchmarkId::new("write", &label), |b| {
            b.iter(|| {
                let mut vfs = MemoryVfs::new();
                vfs.mkdir("/data").unwrap();
                for path in &paths {
                    vfs.write(path, data).unwrap();
                }
            });
        });
    }

    group.finish();
}

fn bench_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("vfs_read");

    for n_files in [100, 1_000] {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/data").unwrap();
        let paths: Vec<String> = (0..n_files)
            .map(|i| format!("/data/file_{i}.bin"))
            .collect();
        let data = vec![0xABu8; 1_024];
        for path in &paths {
            vfs.write(path, &data).unwrap();
        }
        let label = format!("{n_files}");

        group.bench_function(BenchmarkId::new("read", &label), |b| {
            b.iter(|| {
                for path in &paths {
                    let _ = vfs.read(path);
                }
            });
        });
    }

    group.finish();
}

fn bench_readdir(c: &mut Criterion) {
    let mut group = c.benchmark_group("vfs_readdir");

    for n_entries in [100, 1_000] {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/dir").unwrap();
        for i in 0..n_entries {
            vfs.write(&format!("/dir/file_{i}.txt"), b"data").unwrap();
        }
        let label = format!("{n_entries}");

        group.bench_function(BenchmarkId::new("readdir", &label), |b| {
            b.iter(|| vfs.readdir("/dir"));
        });
    }

    group.finish();
}

fn bench_mkdir_deep(c: &mut Criterion) {
    let mut group = c.benchmark_group("vfs_mkdir");

    for depth in [10, 50, 100] {
        let path: String = (0..depth)
            .map(|i| format!("/d{i}"))
            .collect::<Vec<_>>()
            .join("");
        let label = format!("depth_{depth}");

        group.bench_function(BenchmarkId::new("mkdir_deep", &label), |b| {
            b.iter(|| {
                let mut vfs = MemoryVfs::new();
                vfs.mkdir(&path).unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_write,
    bench_read,
    bench_readdir,
    bench_mkdir_deep
);
criterion_main!(benches);
