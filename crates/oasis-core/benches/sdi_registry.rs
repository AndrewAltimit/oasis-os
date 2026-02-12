//! Benchmarks for SDI registry operations.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_core::sdi::registry::SdiRegistry;

fn bench_create(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_create");

    for n in [100, 1_000, 10_000] {
        let names: Vec<String> = (0..n).map(|i| format!("obj_{i}")).collect();
        let label = format!("{n}");

        group.bench_function(BenchmarkId::new("create", &label), |b| {
            b.iter(|| {
                let mut reg = SdiRegistry::new();
                for name in &names {
                    reg.create(name);
                }
                reg
            });
        });
    }

    group.finish();
}

fn bench_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_get");

    for n in [100, 1_000, 10_000] {
        let mut reg = SdiRegistry::new();
        let names: Vec<String> = (0..n).map(|i| format!("obj_{i}")).collect();
        for name in &names {
            reg.create(name);
        }
        let label = format!("{n}");

        group.bench_function(BenchmarkId::new("get", &label), |b| {
            b.iter(|| {
                for name in &names {
                    let _ = reg.get(name);
                }
            });
        });
    }

    group.finish();
}

fn bench_destroy(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_destroy");

    for n in [100, 1_000, 10_000] {
        let names: Vec<String> = (0..n).map(|i| format!("obj_{i}")).collect();
        let label = format!("{n}");

        group.bench_function(BenchmarkId::new("destroy", &label), |b| {
            b.iter_batched(
                || {
                    let mut reg = SdiRegistry::new();
                    for name in &names {
                        reg.create(name);
                    }
                    reg
                },
                |mut reg| {
                    for name in &names {
                        let _ = reg.destroy(name);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_move_to_top(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_move_to_top");

    for n in [100, 1_000] {
        let names: Vec<String> = (0..n).map(|i| format!("obj_{i}")).collect();
        let label = format!("{n}");

        group.bench_function(BenchmarkId::new("move_to_top", &label), |b| {
            b.iter_batched(
                || {
                    let mut reg = SdiRegistry::new();
                    for name in &names {
                        reg.create(name);
                    }
                    reg
                },
                |mut reg| {
                    for name in &names {
                        let _ = reg.move_to_top(name);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_create,
    bench_get,
    bench_destroy,
    bench_move_to_top
);
criterion_main!(benches);
