//! Deterministic schedule engine for TV guide channels.
//!
//! Given a channel catalog and a Unix timestamp, computes exactly which
//! episode is playing. The schedule is deterministic: same catalog + same
//! timestamp = same result, regardless of when the app was started.

use super::catalog::{ChannelCatalog, VideoEpisode};

/// A scheduled time slot — what's playing and when.
#[derive(Debug, Clone)]
pub struct ScheduleSlot {
    /// The episode playing in this slot.
    pub episode: VideoEpisode,
    /// Unix timestamp when this episode started.
    pub start_time: u64,
    /// Seconds elapsed since the episode started.
    pub elapsed_secs: u64,
    /// Seconds remaining until the episode ends.
    pub remaining_secs: u64,
}

/// Compute what's playing on a channel at a given Unix timestamp.
///
/// Returns `None` if the catalog has no episodes or zero total duration.
pub fn schedule_at(catalog: &ChannelCatalog, unix_time: u64) -> Option<ScheduleSlot> {
    if catalog.episodes.is_empty() || catalog.total_duration_secs <= 0.0 {
        return None;
    }

    let playlist = deterministic_shuffle(&catalog.episodes, channel_seed(catalog.channel_number));
    let cycle_duration = catalog.total_duration_secs as u64;
    if cycle_duration == 0 {
        return None;
    }

    let position_in_cycle = unix_time % cycle_duration;
    let mut elapsed = 0u64;

    for episode in &playlist {
        let ep_duration = episode.duration_secs as u64;
        if ep_duration == 0 {
            continue;
        }
        if elapsed + ep_duration > position_in_cycle {
            let ep_elapsed = position_in_cycle - elapsed;
            return Some(ScheduleSlot {
                episode: episode.clone(),
                start_time: unix_time - ep_elapsed,
                elapsed_secs: ep_elapsed,
                remaining_secs: ep_duration - ep_elapsed,
            });
        }
        elapsed += ep_duration;
    }

    // Fallback: should not reach here if durations are consistent,
    // but return last episode if rounding causes a gap.
    playlist.last().map(|ep| ScheduleSlot {
        episode: ep.clone(),
        start_time: unix_time,
        elapsed_secs: 0,
        remaining_secs: ep.duration_secs as u64,
    })
}

/// Generate schedule for a channel over a time range.
///
/// Returns schedule slots covering `[start_time, end_time)`. Each slot
/// represents one episode that falls (wholly or partially) in the window.
pub fn schedule_range(
    catalog: &ChannelCatalog,
    start_time: u64,
    end_time: u64,
) -> Vec<ScheduleSlot> {
    if catalog.episodes.is_empty() || catalog.total_duration_secs <= 0.0 || start_time >= end_time {
        return Vec::new();
    }

    let mut slots = Vec::new();
    let mut t = start_time;

    while t < end_time {
        let Some(slot) = schedule_at(catalog, t) else {
            break;
        };
        let next_t = slot.start_time + slot.episode.duration_secs as u64;
        slots.push(slot);
        // Advance to the next episode's start.
        t = next_t;
    }

    slots
}

/// Round a Unix timestamp down to the nearest 30-minute boundary.
pub fn align_to_slot(unix_time: u64) -> u64 {
    (unix_time / 1800) * 1800
}

/// Format a Unix timestamp as "H:MM PM" for display.
pub fn format_time(unix_time: u64) -> String {
    let seconds_in_day = unix_time % 86400;
    let hours_24 = (seconds_in_day / 3600) as u32;
    let minutes = ((seconds_in_day % 3600) / 60) as u32;
    let (hours_12, ampm) = match hours_24 {
        0 => (12, "AM"),
        1..=11 => (hours_24, "AM"),
        12 => (12, "PM"),
        _ => (hours_24 - 12, "PM"),
    };
    format!("{hours_12}:{minutes:02} {ampm}")
}

/// Format a duration in seconds as "M:SS" or "H:MM:SS".
pub fn format_duration(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Create a deterministic seed from a channel number.
fn channel_seed(channel_number: u32) -> u64 {
    // Mix the channel number into a well-distributed seed.
    let mut seed = channel_number as u64;
    seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    seed
}

/// Fisher-Yates shuffle with a deterministic LCG PRNG.
///
/// Same seed always produces the same order.
fn deterministic_shuffle(episodes: &[VideoEpisode], seed: u64) -> Vec<VideoEpisode> {
    let mut result: Vec<VideoEpisode> = episodes.to_vec();
    let len = result.len();
    if len <= 1 {
        return result;
    }
    let mut rng = seed;
    for i in (1..len).rev() {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (rng >> 33) as usize % (i + 1);
        result.swap(i, j);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_catalog(channel: u32, episode_count: usize, duration: f64) -> ChannelCatalog {
        let mut catalog = ChannelCatalog::new(channel);
        let episodes: Vec<VideoEpisode> = (0..episode_count)
            .map(|i| VideoEpisode {
                item_id: format!("item-{i}"),
                filename: format!("ep{i}.mp4"),
                title: format!("Episode {i}"),
                duration_secs: duration,
                width: 640,
                height: 480,
                size_bytes: 1000,
            })
            .collect();
        catalog.add_episodes(episodes);
        catalog
    }

    #[test]
    fn schedule_at_returns_some() {
        let catalog = make_catalog(2, 5, 1800.0); // 5 episodes, 30 min each
        let slot = schedule_at(&catalog, 1_000_000).unwrap();
        assert!(slot.elapsed_secs < slot.episode.duration_secs as u64);
        assert_eq!(
            slot.elapsed_secs + slot.remaining_secs,
            slot.episode.duration_secs as u64,
        );
    }

    #[test]
    fn schedule_at_empty_catalog() {
        let catalog = ChannelCatalog::new(1);
        assert!(schedule_at(&catalog, 1_000_000).is_none());
    }

    #[test]
    fn schedule_at_deterministic() {
        let catalog = make_catalog(2, 10, 600.0);
        let slot1 = schedule_at(&catalog, 1_700_000_000).unwrap();
        let slot2 = schedule_at(&catalog, 1_700_000_000).unwrap();
        assert_eq!(slot1.episode.title, slot2.episode.title);
        assert_eq!(slot1.start_time, slot2.start_time);
        assert_eq!(slot1.elapsed_secs, slot2.elapsed_secs);
    }

    #[test]
    fn schedule_at_different_channels_differ() {
        let cat_a = make_catalog(2, 10, 600.0);
        let cat_b = make_catalog(5, 10, 600.0);
        let slot_a = schedule_at(&cat_a, 1_700_000_000).unwrap();
        let slot_b = schedule_at(&cat_b, 1_700_000_000).unwrap();
        // Different channel seeds should (almost certainly) produce different schedules.
        // With 10 episodes, it's extremely unlikely the first episode matches.
        // This is a statistical test — could theoretically fail but probability is ~1/10.
        let different = slot_a.episode.title != slot_b.episode.title
            || slot_a.elapsed_secs != slot_b.elapsed_secs;
        assert!(
            different,
            "different channels should produce different schedules"
        );
    }

    #[test]
    fn schedule_at_time_continuity() {
        let catalog = make_catalog(2, 5, 1800.0);
        // Check that advancing by 1 second gives consistent results.
        let slot1 = schedule_at(&catalog, 1_000_000).unwrap();
        let slot2 = schedule_at(&catalog, 1_000_001).unwrap();
        if slot1.episode.title == slot2.episode.title {
            // Same episode: elapsed should differ by 1.
            assert_eq!(slot2.elapsed_secs, slot1.elapsed_secs + 1);
        } else {
            // Episode boundary: slot1 should have 0 remaining.
            assert_eq!(slot1.remaining_secs, 0);
        }
    }

    #[test]
    fn schedule_range_basic() {
        let catalog = make_catalog(2, 5, 1800.0); // 5 x 30min = 2.5h total
        let start = 1_000_000;
        let end = start + 7200; // 2 hours
        let slots = schedule_range(&catalog, start, end);
        assert!(!slots.is_empty());
        // Should have multiple slots covering the 2-hour window.
        assert!(slots.len() >= 2);
    }

    #[test]
    fn schedule_range_empty() {
        let catalog = ChannelCatalog::new(1);
        let slots = schedule_range(&catalog, 1000, 2000);
        assert!(slots.is_empty());
    }

    #[test]
    fn schedule_range_invalid_bounds() {
        let catalog = make_catalog(1, 5, 600.0);
        let slots = schedule_range(&catalog, 2000, 1000);
        assert!(slots.is_empty());
    }

    #[test]
    fn align_to_slot_basic() {
        assert_eq!(align_to_slot(0), 0);
        assert_eq!(align_to_slot(1799), 0);
        assert_eq!(align_to_slot(1800), 1800);
        assert_eq!(align_to_slot(3599), 1800);
        assert_eq!(align_to_slot(3600), 3600);
    }

    #[test]
    fn format_time_midnight() {
        assert_eq!(format_time(0), "12:00 AM");
    }

    #[test]
    fn format_time_noon() {
        assert_eq!(format_time(43200), "12:00 PM");
    }

    #[test]
    fn format_time_afternoon() {
        // 3:30 PM = 15*3600 + 30*60 = 55800
        assert_eq!(format_time(55800), "3:30 PM");
    }

    #[test]
    fn format_time_morning() {
        // 9:15 AM = 9*3600 + 15*60 = 33300
        assert_eq!(format_time(33300), "9:15 AM");
    }

    #[test]
    fn format_duration_short() {
        assert_eq!(format_duration(65.0), "1:05");
        assert_eq!(format_duration(0.0), "0:00");
        assert_eq!(format_duration(3599.0), "59:59");
    }

    #[test]
    fn format_duration_long() {
        assert_eq!(format_duration(3600.0), "1:00:00");
        assert_eq!(format_duration(7384.0), "2:03:04");
    }

    #[test]
    fn deterministic_shuffle_same_seed() {
        let catalog = make_catalog(1, 20, 100.0);
        let a = deterministic_shuffle(&catalog.episodes, 42);
        let b = deterministic_shuffle(&catalog.episodes, 42);
        let titles_a: Vec<&str> = a.iter().map(|e| e.title.as_str()).collect();
        let titles_b: Vec<&str> = b.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles_a, titles_b);
    }

    #[test]
    fn deterministic_shuffle_different_seeds() {
        let catalog = make_catalog(1, 20, 100.0);
        let a = deterministic_shuffle(&catalog.episodes, 111);
        let b = deterministic_shuffle(&catalog.episodes, 222);
        let titles_a: Vec<&str> = a.iter().map(|e| e.title.as_str()).collect();
        let titles_b: Vec<&str> = b.iter().map(|e| e.title.as_str()).collect();
        assert_ne!(titles_a, titles_b);
    }

    #[test]
    fn deterministic_shuffle_empty() {
        let result = deterministic_shuffle(&[], 42);
        assert!(result.is_empty());
    }

    #[test]
    fn deterministic_shuffle_single() {
        let catalog = make_catalog(1, 1, 100.0);
        let result = deterministic_shuffle(&catalog.episodes, 42);
        assert_eq!(result.len(), 1);
    }
}
