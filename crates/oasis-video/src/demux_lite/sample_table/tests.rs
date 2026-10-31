use super::*;

fn stts(runs: &[(u32, u32)]) -> Vec<SttsEntry> {
    runs.iter()
        .map(|&(sample_count, sample_delta)| SttsEntry {
            sample_count,
            sample_delta,
        })
        .collect()
}

fn finalized(mut table: SampleTable) -> SampleTable {
    table.finalize();
    table
}

fn track(table: SampleTable) -> TrackInfo {
    TrackInfo {
        kind: TrackKind::Video,
        table: finalized(table),
        avcc: None,
        aac_config: None,
    }
}

#[test]
fn sample_pts_from_stts() {
    let table = finalized(SampleTable {
        stts: stts(&[(3, 1000), (2, 2000)]),
        timescale: 30000,
        ..Default::default()
    });
    assert_eq!(sample_pts(&table, 0), 0);
    assert_eq!(sample_pts(&table, 1), 1000);
    assert_eq!(sample_pts(&table, 2), 2000);
    assert_eq!(sample_pts(&table, 3), 3000); // first in 2nd run
    assert_eq!(sample_pts(&table, 4), 3000 + 2000);
}

#[test]
fn sample_pts_with_ctts() {
    let table = finalized(SampleTable {
        stts: stts(&[(3, 1000)]),
        ctts: vec![
            CttsEntry {
                sample_count: 1,
                sample_offset: 500,
            },
            CttsEntry {
                sample_count: 2,
                sample_offset: -200,
            },
        ],
        timescale: 1000,
        ..Default::default()
    });
    assert_eq!(sample_pts(&table, 0), 500); // DTS=0 + CTS=500
    assert_eq!(sample_pts(&table, 1), 800); // DTS=1000 + CTS=-200
    assert_eq!(sample_pts(&table, 2), 1800);
    assert_eq!(sample_pts(&table, 3), 3000); // past ctts: no offset
}

#[test]
fn keyframe_lookup_stss() {
    let table = finalized(SampleTable {
        stss: vec![1, 5, 10], // 1-based
        stsz: vec![100; 15].into(),
        stts: stts(&[(15, 1000)]),
        timescale: 1000,
        ..Default::default()
    });
    assert!(is_keyframe(&table, 0)); // sample 1 (0-based=0)
    assert!(!is_keyframe(&table, 1));
    assert!(is_keyframe(&table, 4)); // sample 5 (0-based=4)
    assert!(is_keyframe(&table, 9)); // sample 10 (0-based=9)
    assert!(!is_keyframe(&table, usize::MAX));
}

#[test]
fn unsorted_stss_is_sorted_by_finalize() {
    // Regression: a hostile/odd muxer's unsorted stss made the binary
    // search in `is_keyframe` miss real sync samples.
    let table = finalized(SampleTable {
        stss: vec![10, 1, 5, 5],
        ..Default::default()
    });
    assert_eq!(table.stss, vec![1, 5, 10]);
    assert!(is_keyframe(&table, 9));
}

#[test]
fn empty_stts_table_pts_is_zero() {
    let table = finalized(SampleTable::default());
    assert_eq!(sample_pts(&table, 0), 0);
    assert_eq!(sample_pts(&table, 100), 0);
}

#[test]
fn empty_stsz_table_sample_offset_returns_none() {
    let table = finalized(SampleTable::default());
    assert!(sample_file_offset(&table, 0).is_none());
}

#[test]
fn empty_stsc_table_sample_to_chunk_returns_none() {
    let table = finalized(SampleTable {
        stsz: vec![100; 5].into(),
        stco: vec![0],
        ..Default::default()
    });
    // stsc is empty, so sample_to_chunk returns None.
    assert!(sample_file_offset(&table, 0).is_none());
}

#[test]
fn empty_stco_table_sample_offset_returns_none() {
    let table = finalized(SampleTable {
        stsz: vec![100; 5].into(),
        stsc: vec![StscEntry {
            first_chunk: 1,
            samples_per_chunk: 5,
        }],
        // stco is empty.
        ..Default::default()
    });
    assert!(sample_file_offset(&table, 0).is_none());
}

#[test]
fn sample_offsets_across_stsc_runs() {
    // Chunks 1-2 hold 2 samples each, chunk 3 holds 3.
    let table = finalized(SampleTable {
        stsz: vec![10, 11, 12, 13, 14, 15, 16].into(),
        stsc: vec![
            StscEntry {
                first_chunk: 1,
                samples_per_chunk: 2,
            },
            StscEntry {
                first_chunk: 3,
                samples_per_chunk: 3,
            },
        ],
        stco: vec![1000, 2000, 3000],
        ..Default::default()
    });
    assert_eq!(sample_file_offset(&table, 0), Some((1000, 10)));
    assert_eq!(sample_file_offset(&table, 1), Some((1010, 11)));
    assert_eq!(sample_file_offset(&table, 2), Some((2000, 12)));
    assert_eq!(sample_file_offset(&table, 3), Some((2012, 13)));
    assert_eq!(sample_file_offset(&table, 4), Some((3000, 14)));
    assert_eq!(sample_file_offset(&table, 6), Some((3000 + 14 + 15, 16)));
    assert_eq!(sample_file_offset(&table, 7), None);
}

#[test]
fn hostile_stsc_first_chunk_does_not_spin() {
    // Regression: a second stsc entry starting at chunk 0xFFFF_FFFF made
    // the old per-chunk walk iterate ~4 billion times per lookup.
    let table = finalized(SampleTable {
        stsz: SampleSizes::Uniform {
            size: 1,
            count: MAX_TABLE_ENTRIES,
        },
        stsc: vec![
            StscEntry {
                first_chunk: 1,
                samples_per_chunk: 0,
            },
            StscEntry {
                first_chunk: u32::MAX,
                samples_per_chunk: 1,
            },
        ],
        stco: vec![0; 4],
        ..Default::default()
    });
    assert_eq!(sample_file_offset(&table, 5_000_000), None);
}

#[test]
fn chunk_offset_overflow_is_none() {
    // Regression: co64 offset near u64::MAX + preceding sample sizes
    // overflowed (panic in debug builds).
    let table = finalized(SampleTable {
        stsz: vec![u32::MAX; 2].into(),
        stsc: vec![StscEntry {
            first_chunk: 1,
            samples_per_chunk: 2,
        }],
        stco: vec![u64::MAX - 10],
        ..Default::default()
    });
    assert_eq!(
        sample_file_offset(&table, 0),
        Some((u64::MAX - 10, u32::MAX))
    );
    assert_eq!(sample_file_offset(&table, 1), None);
}

#[test]
fn uniform_sizes_behave_like_per_sample() {
    let uniform = SampleSizes::Uniform { size: 7, count: 4 };
    let per: SampleSizes = vec![7; 4].into();
    for s in [&uniform, &per] {
        assert_eq!(s.len(), 4);
        assert_eq!(s.get(3), Some(7));
        assert_eq!(s.get(4), None);
        assert_eq!(s.max(), 7);
        assert_eq!(s.sum(1, 3), 14);
        assert_eq!(s.sum(3, 100), 7);
    }
    assert_eq!(SampleSizes::Uniform { size: 7, count: 0 }.max(), 0);
}

#[test]
fn indexed_size_sums_match_the_linear_walk() {
    let sizes: Vec<u32> = (0..1000u32).map(|i| i * 7 % 113).collect();
    let reference: SampleSizes = sizes.clone().into();
    let table = finalized(SampleTable {
        stsz: sizes.into(),
        ..Default::default()
    });
    for (start, end) in [
        (0, 0),
        (0, 64),
        (63, 65),
        (64, 128),
        (5, 999),
        (0, 1000),
        (900, 5000),
    ] {
        assert_eq!(
            table.size_sum(start, end),
            reference.sum(start, end),
            "{start}..{end}"
        );
    }
}

#[test]
fn offset_lookup_in_a_huge_chunk_is_not_linear() {
    // One chunk of 2M samples: the last sample's offset used to sum
    // all 2M preceding sizes on every lookup.
    let n = 2_000_000usize;
    let table = finalized(SampleTable {
        stsc: vec![StscEntry {
            first_chunk: 1,
            samples_per_chunk: n as u32,
        }],
        stsz: vec![3u32; n].into(),
        stco: vec![1000],
        ..Default::default()
    });
    let start = std::time::Instant::now();
    for i in (n - 1000)..n {
        assert_eq!(
            sample_file_offset(&table, i),
            Some((1000 + 3 * i as u64, 3))
        );
    }
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn no_stss_means_all_keyframes() {
    let table = finalized(SampleTable {
        stsz: vec![100; 10].into(),
        ..Default::default()
    });
    for i in 0..10 {
        assert!(
            is_keyframe(&table, i),
            "sample {i} should be keyframe with empty stss"
        );
    }
}

#[test]
fn sample_pts_beyond_stts_range() {
    // Ask for sample_idx beyond the range covered by stts.
    let table = finalized(SampleTable {
        stts: stts(&[(2, 1000)]),
        timescale: 1000,
        ..Default::default()
    });
    // Sample 5 is beyond the table: the DTS stays at its total, 2000.
    assert_eq!(sample_pts(&table, 5), 2000);
}

#[test]
fn ctts_negative_offset_clamps_to_zero() {
    let table = finalized(SampleTable {
        stts: stts(&[(1, 100)]),
        ctts: vec![CttsEntry {
            sample_count: 1,
            sample_offset: -500, // larger than DTS=0
        }],
        timescale: 1000,
        ..Default::default()
    });
    // DTS=0 + CTS=-500 = -500, clamped to 0.
    assert_eq!(sample_pts(&table, 0), 0);
}

#[test]
fn dts_delta_extrapolates_past_table() {
    let table = finalized(SampleTable {
        stts: stts(&[(2, 100), (0, 5), (3, 10)]),
        ..Default::default()
    });
    assert_eq!(sample_dts_delta(&table, 1), (100, 100));
    assert_eq!(sample_dts_delta(&table, 2), (200, 10));
    assert_eq!(sample_dts_delta(&table, 4), (220, 10));
    // Past the 5 samples (230 ticks): extrapolate by the last delta.
    assert_eq!(sample_dts_delta(&table, 6), (240, 10));
}

#[test]
fn containing_dts_skips_zero_delta_runs() {
    let table = finalized(SampleTable {
        stts: stts(&[(2, 100), (3, 0), (2, 50)]),
        ..Default::default()
    });
    assert_eq!(sample_containing_dts(&table, 0), 0);
    assert_eq!(sample_containing_dts(&table, 199), 1);
    // The zero-delta run occupies no time: 200 is the third run's first.
    assert_eq!(sample_containing_dts(&table, 200), 5);
    assert_eq!(sample_containing_dts(&table, 260), 6);
    assert_eq!(sample_containing_dts(&table, 10_000), 6);
}

#[test]
fn find_keyframe_before_no_stss() {
    let track = track(SampleTable {
        stts: stts(&[(10, 1000)]),
        stsz: vec![100; 10].into(),
        timescale: 1000,
        ..Default::default()
    });
    // With no stss, delegates to find_sample_at.
    assert_eq!(find_keyframe_before(&track, 5000), 5);
}

#[test]
fn find_sample_at_rounds_up_to_next_sample() {
    let track = track(SampleTable {
        stts: stts(&[(3, 1000), (0, 7), (2, 500)]),
        ..Default::default()
    });
    assert_eq!(find_sample_at(&track, 0), 0);
    assert_eq!(find_sample_at(&track, 1), 1);
    assert_eq!(find_sample_at(&track, 2000), 2);
    assert_eq!(find_sample_at(&track, 2001), 3);
    assert_eq!(find_sample_at(&track, 3400), 4);
    assert_eq!(find_sample_at(&track, 99_999), 4);
}

#[test]
fn find_sample_at_huge_run_is_constant_time() {
    // Regression: the old per-sample walk took ~4 billion steps here.
    let track = track(SampleTable {
        stts: stts(&[(u32::MAX, 1)]),
        ..Default::default()
    });
    assert_eq!(find_sample_at(&track, 4_000_000_000), 4_000_000_000);
}

#[test]
fn find_sample_at_empty_stts() {
    let track = track(SampleTable::default());
    assert_eq!(find_sample_at(&track, 5000), 0);
}

// ---------------------------------------------------------------
// Property-based tests (proptest)
// ---------------------------------------------------------------

mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Reference implementations: the original per-sample walks.
    fn naive_find_sample_at(table: &SampleTable, target: u64) -> usize {
        let mut dts = 0u64;
        let mut sample = 0usize;
        for e in &table.stts {
            for _ in 0..e.sample_count {
                if dts >= target {
                    return sample;
                }
                dts += u64::from(e.sample_delta);
                sample += 1;
            }
        }
        sample.saturating_sub(1)
    }

    fn naive_pts(table: &SampleTable, idx: usize) -> u64 {
        let mut dts = 0u64;
        let mut sample = 0usize;
        for e in &table.stts {
            let c = e.sample_count as usize;
            if sample + c > idx {
                dts += (idx - sample) as u64 * u64::from(e.sample_delta);
                break;
            }
            dts += c as u64 * u64::from(e.sample_delta);
            sample += c;
        }
        let mut s = 0usize;
        let mut off = 0i64;
        for e in &table.ctts {
            if s + e.sample_count as usize > idx {
                off = i64::from(e.sample_offset);
                break;
            }
            s += e.sample_count as usize;
        }
        (dts as i64 + off).max(0) as u64
    }

    fn naive_offset(table: &SampleTable, idx: usize) -> Option<(u64, u32)> {
        let size = table.stsz.get(idx)?;
        let mut accum = 0usize;
        for (i, e) in table.stsc.iter().enumerate() {
            let first = (e.first_chunk as usize).saturating_sub(1);
            let next = table.stsc.get(i + 1).map_or(table.stco.len(), |n| {
                (n.first_chunk as usize).saturating_sub(1)
            });
            for chunk in first..next.min(table.stco.len()) {
                let spc = e.samples_per_chunk as usize;
                if idx < accum + spc {
                    let base = *table.stco.get(chunk)?;
                    return Some((base + table.stsz.sum(accum, idx), size));
                }
                accum += spc;
            }
        }
        None
    }

    fn arb_table() -> impl Strategy<Value = SampleTable> {
        (
            proptest::collection::vec((0u32..20, 0u32..1000), 0..8),
            proptest::collection::vec((0u32..20, -500i32..500), 0..8),
            proptest::collection::vec((1u32..6, 0u32..5), 0..5),
            proptest::collection::vec(0u64..1_000_000, 0..10),
            proptest::collection::vec(0u32..5000, 0..60),
        )
            .prop_map(|(t, c, mut sc, stco, sizes)| {
                sc.sort_by_key(|&(first, _)| first);
                finalized(SampleTable {
                    stts: stts(&t),
                    ctts: c
                        .into_iter()
                        .map(|(sample_count, sample_offset)| CttsEntry {
                            sample_count,
                            sample_offset,
                        })
                        .collect(),
                    stsc: sc
                        .into_iter()
                        .map(|(first_chunk, samples_per_chunk)| StscEntry {
                            first_chunk,
                            samples_per_chunk,
                        })
                        .collect(),
                    stco,
                    stsz: sizes.into(),
                    timescale: 1000,
                    ..Default::default()
                })
            })
    }

    proptest! {
        #[test]
        fn indexed_lookups_match_naive_walks(
            table in arb_table(),
            idx in 0usize..200,
            target in 0u64..50_000,
        ) {
            prop_assert_eq!(sample_pts(&table, idx), naive_pts(&table, idx));
            prop_assert_eq!(
                sample_file_offset(&table, idx),
                naive_offset(&table, idx)
            );
            let t = TrackInfo {
                kind: TrackKind::Audio,
                table: table.clone(),
                avcc: None,
                aac_config: None,
            };
            prop_assert_eq!(
                find_sample_at(&t, target),
                naive_find_sample_at(&table, target)
            );
        }

        #[test]
        fn sample_pts_no_panic(
            counts in proptest::collection::vec(any::<u32>(), 1..20),
            deltas in proptest::collection::vec(any::<u32>(), 1..20),
            sample_idx in any::<usize>(),
        ) {
            let runs: Vec<_> = counts.into_iter().zip(deltas).collect();
            let table = finalized(SampleTable {
                stts: stts(&runs),
                timescale: 90000,
                ..Default::default()
            });
            let _ = sample_pts(&table, sample_idx);
            let _ = sample_dts_delta(&table, sample_idx);
            let _ = sample_containing_dts(&table, sample_idx as u64);
        }

        #[test]
        fn find_sample_at_no_panic(
            counts in proptest::collection::vec(any::<u32>(), 1..10),
            deltas in proptest::collection::vec(any::<u32>(), 1..10),
            target_ts in any::<u64>(),
        ) {
            let runs: Vec<_> = counts.into_iter().zip(deltas).collect();
            let t = track(SampleTable {
                stts: stts(&runs),
                timescale: 90000,
                ..Default::default()
            });
            let _ = find_sample_at(&t, target_ts);
        }
    }
}
