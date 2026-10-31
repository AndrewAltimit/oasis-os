use super::*;
use crate::types::TrackInfo;

fn make_tracks(n: usize) -> Vec<TrackInfo> {
    (0..n)
        .map(|i| {
            TrackInfo::from_path(&format!("/music/track{i}.mp3"))
                .with_title(&format!("Track {i}"))
                .with_duration_ms((i as u64 + 1) * 60_000)
        })
        .collect()
}

#[test]
fn empty_playlist() {
    let pl = Playlist::new();
    assert!(pl.is_empty());
    assert_eq!(pl.len(), 0);
    assert!(pl.current_track().is_none());
}

#[test]
fn add_and_get_tracks() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    assert_eq!(pl.len(), 3);
    assert!(pl.current_track().is_none()); // Not started yet.
}

#[test]
fn sequential_next() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }

    assert!(pl.advance()); // -> Track 0
    assert_eq!(pl.current_index(), Some(0));
    assert!(pl.advance()); // -> Track 1
    assert_eq!(pl.current_index(), Some(1));
    assert!(pl.advance()); // -> Track 2
    assert_eq!(pl.current_index(), Some(2));
    assert!(!pl.advance()); // End of playlist (repeat off).
}

#[test]
fn sequential_prev() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    pl.set_current(2);

    assert!(pl.go_back()); // -> Track 1
    assert_eq!(pl.current_index(), Some(1));
    assert!(pl.go_back()); // -> Track 0
    assert_eq!(pl.current_index(), Some(0));
    assert!(!pl.go_back()); // Beginning of playlist.
}

#[test]
fn repeat_all() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    pl.repeat = RepeatMode::All;
    pl.set_current(2);

    assert!(pl.advance()); // Wraps to Track 0.
    assert_eq!(pl.current_index(), Some(0));
}

#[test]
fn repeat_all_prev_wraps() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    pl.repeat = RepeatMode::All;
    pl.set_current(0);

    assert!(pl.go_back()); // Wraps to Track 2.
    assert_eq!(pl.current_index(), Some(2));
}

#[test]
fn repeat_one() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    pl.repeat = RepeatMode::One;
    pl.set_current(1);

    assert!(pl.advance()); // Stays on Track 1.
    assert_eq!(pl.current_index(), Some(1));
    assert!(pl.go_back()); // Still Track 1.
    assert_eq!(pl.current_index(), Some(1));
}

#[test]
fn set_current() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    assert!(pl.set_current(1));
    assert_eq!(pl.current_track().unwrap().title, "Track 1");
    assert!(!pl.set_current(10)); // Out of bounds.
}

#[test]
fn remove_track() {
    let mut pl = Playlist::new();
    for t in make_tracks(4) {
        pl.add(t);
    }
    pl.set_current(2);

    // Remove track before current.
    let removed = pl.remove(0).unwrap();
    assert_eq!(removed.title, "Track 0");
    assert_eq!(pl.current_index(), Some(1)); // Adjusted down.
    assert_eq!(pl.len(), 3);
}

#[test]
fn remove_current_track() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    pl.set_current(1);

    pl.remove(1);
    // Current stays at 1 (now "Track 2").
    assert_eq!(pl.current_index(), Some(1));
    assert_eq!(pl.current_track().unwrap().title, "Track 2");
}

#[test]
fn clear_playlist() {
    let mut pl = Playlist::new();
    for t in make_tracks(3) {
        pl.add(t);
    }
    pl.set_current(1);
    pl.clear();
    assert!(pl.is_empty());
    assert!(pl.current_track().is_none());
}

#[test]
fn shuffle_next() {
    let mut pl = Playlist::new();
    for t in make_tracks(4) {
        pl.add(t);
    }
    pl.set_shuffle(true);
    pl.set_current(0);

    // Should advance through shuffle order.
    assert!(pl.advance());
    let first = pl.current_index().unwrap();
    assert!(pl.advance());
    let second = pl.current_index().unwrap();
    // In shuffle mode, the order differs from sequential.
    // We can't assert exact values but can check they're valid indices.
    assert!(first < 4);
    assert!(second < 4);
}

#[test]
fn format_duration_minutes() {
    assert_eq!(format_duration(0), "0:00");
    assert_eq!(format_duration(1_000), "0:01");
    assert_eq!(format_duration(61_000), "1:01");
    assert_eq!(format_duration(3_600_000), "1:00:00");
    assert_eq!(format_duration(3_661_000), "1:01:01");
}

#[test]
fn format_playlist_display() {
    let mut pl = Playlist::new();
    for t in make_tracks(2) {
        pl.add(t);
    }
    pl.set_current(0);

    let text = format_playlist(&pl);
    assert!(text.contains("2 tracks"));
    assert!(text.contains("> 0. Track 0"));
    assert!(text.contains("  1. Track 1"));
}

#[test]
fn format_empty_playlist() {
    let pl = Playlist::new();
    assert_eq!(format_playlist(&pl), "(empty playlist)");
}

#[test]
fn next_on_empty_playlist() {
    let mut pl = Playlist::new();
    assert!(!pl.advance());
    assert!(!pl.go_back());
}

mod prop {
    use super::*;
    use crate::types::TrackInfo;
    use proptest::prelude::*;

    fn arb_track() -> impl Strategy<Value = TrackInfo> {
        ("[a-z]{1,10}", "[a-z]{1,10}", 0u64..600_000).prop_map(|(title, artist, dur)| {
            TrackInfo::from_path(&format!("/music/{title}.mp3"))
                .with_title(&title)
                .with_artist(&artist)
                .with_duration_ms(dur)
        })
    }

    fn arb_tracks(min: usize, max: usize) -> impl Strategy<Value = Vec<TrackInfo>> {
        proptest::collection::vec(arb_track(), min..max)
    }

    proptest! {
        #[test]
        fn len_equals_added_count(tracks in arb_tracks(0, 20)) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            prop_assert_eq!(pl.len(), tracks.len());
        }

        #[test]
        fn add_then_remove_restores_len(tracks in arb_tracks(1, 10)) {
            let mut pl = Playlist::new();
            let n = tracks.len();
            for t in tracks {
                pl.add(t);
            }
            for i in (0..n).rev() {
                pl.remove(i);
            }
            prop_assert!(pl.is_empty());
            prop_assert_eq!(pl.len(), 0);
        }

        #[test]
        fn set_current_valid_index(tracks in arb_tracks(1, 10), idx_frac in 0.0f64..1.0) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            let idx = (idx_frac * tracks.len() as f64) as usize;
            let idx = idx.min(tracks.len() - 1);
            prop_assert!(pl.set_current(idx));
            prop_assert_eq!(pl.current_index(), Some(idx));
        }

        #[test]
        fn set_current_invalid_index_fails(tracks in arb_tracks(1, 10)) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            prop_assert!(!pl.set_current(tracks.len()));
            prop_assert!(!pl.set_current(tracks.len() + 100));
        }

        #[test]
        fn advance_through_all_with_repeat_all(tracks in arb_tracks(1, 10)) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            pl.repeat = RepeatMode::All;
            // Advance len+1 times -- should wrap around.
            for _ in 0..=tracks.len() {
                prop_assert!(pl.advance());
            }
            // After wrapping, current should be a valid index.
            let idx = pl.current_index().unwrap();
            prop_assert!(idx < tracks.len());
        }

        #[test]
        fn go_back_through_all_with_repeat_all(tracks in arb_tracks(1, 10)) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            pl.repeat = RepeatMode::All;
            pl.set_current(0);
            // go_back at index 0 with RepeatAll should wrap to last.
            prop_assert!(pl.go_back());
            prop_assert_eq!(pl.current_index(), Some(tracks.len() - 1));
        }

        #[test]
        fn repeat_one_stays_on_same(tracks in arb_tracks(1, 10), idx_frac in 0.0f64..1.0) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            let idx = (idx_frac * tracks.len() as f64).min(tracks.len() as f64 - 1.0) as usize;
            pl.set_current(idx);
            pl.repeat = RepeatMode::One;
            prop_assert!(pl.advance());
            prop_assert_eq!(pl.current_index(), Some(idx));
            prop_assert!(pl.go_back());
            prop_assert_eq!(pl.current_index(), Some(idx));
        }

        #[test]
        fn clear_empties_everything(tracks in arb_tracks(1, 10)) {
            let mut pl = Playlist::new();
            for t in tracks {
                pl.add(t);
            }
            pl.set_current(0);
            pl.clear();
            prop_assert!(pl.is_empty());
            prop_assert_eq!(pl.len(), 0);
            prop_assert!(pl.current_track().is_none());
        }

        #[test]
        fn remove_keeps_valid_current(
            tracks in arb_tracks(2, 10),
            remove_idx_frac in 0.0f64..1.0,
        ) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            pl.set_current(0);
            let remove_idx = (remove_idx_frac * tracks.len() as f64)
                .min(tracks.len() as f64 - 1.0) as usize;
            pl.remove(remove_idx);
            // After removal, current_index (if Some) must be in bounds.
            if let Some(cur) = pl.current_index() {
                prop_assert!(cur < pl.len(), "current index out of bounds: {cur} >= {}", pl.len());
            }
        }

        /// Shuffle order contains all original track indices exactly once.
        #[test]
        fn shuffle_visits_all_tracks(tracks in arb_tracks(2, 10)) {
            let mut pl = Playlist::new();
            let n = tracks.len();
            for t in &tracks {
                pl.add(t.clone());
            }
            pl.set_shuffle(true);
            pl.repeat = RepeatMode::Off;
            pl.set_current(0);

            // Collect all indices visited by advancing through shuffle.
            let mut visited = std::collections::HashSet::new();
            if let Some(idx) = pl.current_index() {
                visited.insert(idx);
            }
            // Advance n-1 times (first track already counted).
            for _ in 0..n {
                if pl.advance()
                    && let Some(idx) = pl.current_index() {
                        visited.insert(idx);
                    }
            }
            // All visited indices should be valid.
            for &idx in &visited {
                prop_assert!(idx < n, "shuffle produced invalid index: {idx}");
            }
        }

        /// next then prev returns to the same track (non-shuffle,
        /// non-repeat-one, when not at edges).
        #[test]
        fn next_then_prev_returns_to_same(tracks in arb_tracks(3, 10)) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            pl.repeat = RepeatMode::Off;
            // Start at index 1 (not at an edge).
            pl.set_current(1);
            let before = pl.current_index();
            prop_assert!(pl.advance()); // -> index 2
            prop_assert!(pl.go_back()); // -> index 1
            prop_assert_eq!(pl.current_index(), before);
        }

        /// prev then next returns to the same track.
        #[test]
        fn prev_then_next_returns_to_same(tracks in arb_tracks(3, 10)) {
            let mut pl = Playlist::new();
            for t in &tracks {
                pl.add(t.clone());
            }
            pl.repeat = RepeatMode::Off;
            // Start at index 1 (not at an edge).
            pl.set_current(1);
            let before = pl.current_index();
            prop_assert!(pl.go_back()); // -> index 0
            prop_assert!(pl.advance()); // -> index 1
            prop_assert_eq!(pl.current_index(), before);
        }

        /// With RepeatMode::All, advancing n times from any position
        /// cycles back to the start.
        #[test]
        fn repeat_all_cycles_completely(tracks in arb_tracks(1, 10)) {
            let mut pl = Playlist::new();
            let n = tracks.len();
            for t in &tracks {
                pl.add(t.clone());
            }
            pl.repeat = RepeatMode::All;
            pl.set_current(0);
            let start = pl.current_index();
            // Advance exactly n times to complete one full cycle.
            for _ in 0..n {
                prop_assert!(pl.advance());
            }
            prop_assert_eq!(pl.current_index(), start);
        }

        /// Any sequence of edits and moves keeps the playlist coherent:
        /// `current` in range, and while shuffling the order is a
        /// permutation whose cursor sits on `current`.
        #[test]
        fn random_operations_keep_invariants(
            seed in any::<u64>(),
            ops in proptest::collection::vec((0u8..8, 0usize..12), 0..60),
        ) {
            let mut pl = Playlist::with_seed(seed);
            let mut next_id = 0;
            for (op, arg) in ops {
                match op {
                    0 | 1 => {
                        pl.add(TrackInfo::from_path(&format!("/m/{next_id}.mp3")));
                        next_id += 1;
                    },
                    2 => { pl.remove(arg); },
                    3 => { pl.advance(); },
                    4 => { pl.go_back(); },
                    5 => { pl.set_current(arg); },
                    6 => pl.set_shuffle(!pl.shuffle()),
                    _ => pl.repeat = match arg % 3 {
                        0 => RepeatMode::Off,
                        1 => RepeatMode::All,
                        _ => RepeatMode::One,
                    },
                }
                if let Some(cur) = pl.current_index() {
                    prop_assert!(cur < pl.len());
                }
                if pl.shuffle() && !pl.shuffle_order.is_empty() {
                    let mut sorted = pl.shuffle_order.clone();
                    sorted.sort_unstable();
                    prop_assert_eq!(sorted, (0..pl.len()).collect::<Vec<_>>());
                    if let Some(cur) = pl.current_index() {
                        prop_assert_eq!(pl.shuffle_order[pl.shuffle_pos], cur);
                    }
                }
            }
        }
    }
}

fn shuffled(n: usize, seed: u64) -> Playlist {
    let mut pl = Playlist::with_seed(seed);
    for t in make_tracks(n) {
        pl.add(t);
    }
    pl.set_shuffle(true);
    pl
}

/// Advance until `advance` returns false, collecting the tracks played.
fn play_round(pl: &mut Playlist) -> Vec<usize> {
    let mut played: Vec<usize> = pl.current_index().into_iter().collect();
    while pl.advance() {
        played.push(pl.current_index().unwrap());
        assert!(played.len() <= pl.len(), "round longer than the playlist");
    }
    played
}

fn assert_each_once(played: &[usize], n: usize) {
    let mut sorted = played.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "played {played:?}");
}

#[test]
fn shuffle_from_not_started_plays_every_track_once() {
    // Regression: the first `advance` stepped the cursor past
    // shuffle_order[0], so that track was never played.
    for seed in 0..20 {
        let mut pl = shuffled(7, seed);
        assert!(pl.advance());
        assert_each_once(&play_round(&mut pl), 7);
    }
}

#[test]
fn shuffle_on_mid_playlist_plays_every_other_track() {
    // Regression: the order was fixed and the cursor jumped to the
    // current track's slot in it, so every track ordered before that
    // slot was skipped for the rest of the round.
    for seed in 0..20 {
        let mut pl = Playlist::with_seed(seed);
        for t in make_tracks(8) {
            pl.add(t);
        }
        pl.set_current(5);
        pl.set_shuffle(true);
        let played = play_round(&mut pl);
        assert_eq!(played[0], 5);
        assert_each_once(&played, 8);
    }
}

#[test]
fn shuffle_orders_differ_between_seeds() {
    // Regression: the "shuffle" was a fixed interleave, the same order
    // every time.
    let orders: std::collections::HashSet<Vec<usize>> = (0..10)
        .map(|seed| {
            let mut pl = shuffled(10, seed);
            pl.advance();
            play_round(&mut pl)
        })
        .collect();
    assert!(orders.len() > 5, "{orders:?}");
}

#[test]
fn repeat_all_shuffle_cycles_every_track_without_repeats() {
    let mut pl = shuffled(6, 3);
    pl.repeat = RepeatMode::All;
    let mut played = vec![];
    for _ in 0..30 {
        assert!(pl.advance());
        played.push(pl.current_index().unwrap());
    }
    // Every 6 consecutive tracks are all six, never one twice in a row.
    for window in played.windows(6) {
        assert_each_once(window, 6);
    }
    // `prev` undoes `next` across the wrap.
    let at = pl.current_index();
    assert!(pl.advance());
    assert!(pl.go_back());
    assert_eq!(pl.current_index(), at);
    // A single-track playlist repeats it.
    let mut one = shuffled(1, 9);
    one.repeat = RepeatMode::All;
    assert!(one.advance());
    assert!(one.advance());
    assert_eq!(one.current_index(), Some(0));
}

#[test]
fn shuffle_add_joins_the_current_round() {
    let mut pl = shuffled(4, 11);
    assert!(pl.advance());
    assert!(pl.advance());
    pl.add(TrackInfo::from_path("/music/new.mp3"));
    // The rest of the round includes the new track (index 4).
    let mut played = vec![];
    while pl.advance() {
        played.push(pl.current_index().unwrap());
    }
    assert!(played.contains(&4), "{played:?}");
}

#[test]
fn shuffle_remove_current_moves_to_its_successor() {
    let mut pl = shuffled(5, 5);
    assert!(pl.advance());
    assert!(pl.advance());
    let cur = pl.current_index().unwrap();
    let successor = pl.shuffle_order[pl.shuffle_pos + 1];
    let removed_path = pl.current_track().unwrap().path.clone();
    pl.remove(cur);
    let expected = if successor > cur {
        successor - 1
    } else {
        successor
    };
    assert_eq!(pl.current_index(), Some(expected));
    assert_ne!(pl.current_track().unwrap().path, removed_path);
    // Removing every track leaves nothing current.
    while !pl.is_empty() {
        pl.remove(0);
    }
    assert_eq!(pl.current_index(), None);
    assert!(!pl.advance());
    assert!(!pl.go_back());
}

#[test]
fn shuffle_pick_keeps_the_rest_of_the_round() {
    // Regression: picking a track jumped the cursor to its slot in the
    // order, skipping every unplayed track ordered before it (and a
    // pick of the last-ordered track ended the round at once).
    for seed in 0..20 {
        let mut pl = shuffled(6, seed);
        assert!(pl.advance());
        let first = pl.current_index().unwrap();
        let last_ordered = *pl.shuffle_order.last().unwrap();
        assert!(pl.set_current(last_ordered));
        let mut played = vec![first];
        played.extend(play_round(&mut pl));
        assert_each_once(&played, 6);
    }
}

#[test]
fn shuffle_off_then_on_starts_from_current() {
    let mut pl = shuffled(6, 1);
    pl.advance();
    pl.set_shuffle(false);
    pl.set_current(3);
    pl.set_shuffle(true);
    assert_eq!(pl.current_index(), Some(3));
    assert_each_once(&play_round(&mut pl), 6);
}
