//! Benchmarks for SDI registry operations.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use oasis_sdi::registry::SdiRegistry;

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

/// Registry with `n` objects, half of them hidden (mirrors a real scene:
/// inactive modes keep their object pools around, invisible).
fn half_hidden_registry(n: usize) -> (SdiRegistry, Vec<String>) {
    let mut reg = SdiRegistry::new();
    let names: Vec<String> = (0..n).map(|i| format!("obj_{i}")).collect();
    for (i, name) in names.iter().enumerate() {
        let obj = reg.create(name);
        obj.w = 10;
        obj.h = 10;
        obj.text = Some(format!("label {i}"));
        obj.visible = i % 2 == 0;
    }
    (reg, names)
}

fn bench_scene_signature(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_scene_signature");
    for n in [100, 1_000] {
        let (reg, _) = half_hidden_registry(n);
        group.bench_function(BenchmarkId::new("signature", n), |b| {
            b.iter(|| reg.scene_signature());
        });
    }
    group.finish();
}

/// The per-frame "hide pass" idiom: rewrite `visible = false` on objects
/// that are already hidden, then run the idle-frame dirty check. On an
/// unchanged scene this should cost lookups only — no dirty frame and no
/// signature hash.
fn bench_idle_hide_pass(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdi_idle_hide_pass");
    for n in [200, 1_000] {
        let (mut reg, names) = half_hidden_registry(n);
        let hidden: Vec<&String> = names.iter().skip(1).step_by(2).collect();
        let _ = reg.take_scene_dirty();
        group.bench_function(BenchmarkId::new("get_mut", n), |b| {
            b.iter(|| {
                for name in &hidden {
                    if let Ok(obj) = reg.get_mut(name) {
                        obj.visible = false;
                    }
                }
                if reg.take_scene_dirty() {
                    criterion::black_box(reg.scene_signature());
                }
            });
        });
        group.bench_function(BenchmarkId::new("set_visible", n), |b| {
            b.iter(|| {
                for name in &hidden {
                    reg.set_visible(name, false);
                }
                if reg.take_scene_dirty() {
                    criterion::black_box(reg.scene_signature());
                }
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_create,
    bench_get,
    bench_destroy,
    bench_move_to_top,
    bench_scene_signature,
    bench_idle_hide_pass
);
criterion_main!(benches);
