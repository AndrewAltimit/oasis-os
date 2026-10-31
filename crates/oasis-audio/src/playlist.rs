//! Playlist management.
//!
//! An ordered list of tracks with current-index tracking, repeat mode,
//! and shuffle support. The playlist is backend-agnostic -- it manages
//! only metadata and ordering.
//!
//! # Shuffle
//!
//! Shuffle keeps a permutation of the track indices (`shuffle_order`) and a
//! cursor into it.  Turning shuffle on (or wrapping around under
//! [`RepeatMode::All`]) draws a fresh random order with the playing track
//! first, so `next` visits every other track exactly once before anything
//! repeats.  Adding or removing tracks edits the order in place instead of
//! reshuffling, so the tracks already played stay played.

use super::types::{RepeatMode, TrackInfo};

/// An ordered playlist of audio tracks.
#[derive(Debug)]
pub struct Playlist {
    /// Tracks in play order.
    tracks: Vec<TrackInfo>,
    /// Index of the current track (None if empty/not started).
    current: Option<usize>,
    /// Repeat mode.
    pub repeat: RepeatMode,
    /// Whether shuffle is enabled.
    shuffle: bool,
    /// Shuffle order: a permutation of `0..tracks.len()` while shuffling.
    shuffle_order: Vec<usize>,
    /// Position of the current track in `shuffle_order`.
    shuffle_pos: usize,
    /// xorshift64 state for shuffling (never 0).
    rng: u64,
}

impl Playlist {
    /// Create an empty playlist.
    pub fn new() -> Self {
        // Seed from the clock: every session gets its own shuffle order.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self::with_seed(nanos)
    }

    /// Create an empty playlist whose shuffle orders come from `seed`
    /// (reproducible orders for tests).
    pub fn with_seed(seed: u64) -> Self {
        Self {
            tracks: Vec::new(),
            current: None,
            repeat: RepeatMode::Off,
            shuffle: false,
            shuffle_order: Vec::new(),
            shuffle_pos: 0,
            // splitmix64 finalizer: spreads small seeds, and `| 1` keeps
            // xorshift off its all-zero fixed point.
            rng: {
                let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                (z ^ (z >> 31)) | 1
            },
        }
    }

    /// Add a track to the end of the playlist.
    ///
    /// While shuffling, the track joins the shuffle order at a random
    /// position among the tracks still to come.
    pub fn add(&mut self, info: TrackInfo) {
        self.tracks.push(info);
        if self.shuffle {
            let idx = self.tracks.len() - 1;
            // Insert after the cursor (or anywhere, before playback starts).
            let first = if self.current.is_some() {
                self.shuffle_pos + 1
            } else {
                0
            };
            let first = first.min(self.shuffle_order.len());
            let slots = (self.shuffle_order.len() - first + 1) as u64;
            let at = first + (self.next_random() % slots) as usize;
            self.shuffle_order.insert(at, idx);
        }
    }

    /// Remove a track by index. Adjusts current index if needed.
    ///
    /// Removing the current track moves `current` to the track that would
    /// have played next (the following track, or the next one in shuffle
    /// order), or to the new last track when there is none.
    pub fn remove(&mut self, index: usize) -> Option<TrackInfo> {
        if index >= self.tracks.len() {
            return None;
        }
        let removed = self.tracks.remove(index);

        if self.shuffle {
            self.remove_from_shuffle(index);
            return Some(removed);
        }

        // Adjust current index.
        if let Some(cur) = self.current {
            if index < cur {
                self.current = Some(cur - 1);
            } else if index == cur {
                // Current track was removed.
                if self.tracks.is_empty() {
                    self.current = None;
                } else if cur >= self.tracks.len() {
                    self.current = Some(self.tracks.len() - 1);
                }
            }
        }
        Some(removed)
    }

    /// Drop track `index` (already removed from `tracks`) from the shuffle
    /// order, renumber the rest, and keep `current` / the cursor on the
    /// same track (or its shuffle successor if it was the one removed).
    fn remove_from_shuffle(&mut self, index: usize) {
        let pos = self.shuffle_order.iter().position(|&i| i == index);
        if let Some(pos) = pos {
            self.shuffle_order.remove(pos);
        }
        for i in &mut self.shuffle_order {
            if *i > index {
                *i -= 1;
            }
        }
        let Some(cur) = self.current else {
            return;
        };
        if self.shuffle_order.is_empty() {
            self.current = None;
            self.shuffle_pos = 0;
            return;
        }
        match pos {
            Some(p) if p < self.shuffle_pos => self.shuffle_pos -= 1,
            Some(p) if p == self.shuffle_pos => {
                // The current track went: its successor takes its place.
                self.shuffle_pos = self.shuffle_pos.min(self.shuffle_order.len() - 1);
                self.current = Some(self.shuffle_order[self.shuffle_pos]);
                return;
            },
            _ => {},
        }
        self.current = Some(if cur > index { cur - 1 } else { cur });
    }

    /// Clear all tracks.
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current = None;
        self.shuffle_order.clear();
        self.shuffle_pos = 0;
    }

    /// Get the number of tracks.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Check if the playlist is empty.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Get the current track info.
    pub fn current_track(&self) -> Option<&TrackInfo> {
        self.current.and_then(|i| self.tracks.get(i))
    }

    /// Get the current track index.
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    /// Set the current track by index.
    ///
    /// While shuffling, a track not yet played this round is moved up to
    /// play now, so the tracks still to come in the round are all kept
    /// (jumping the cursor to its slot would skip the ones before it).
    pub fn set_current(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        if self.shuffle
            && let Some(pos) = self.shuffle_order.iter().position(|&i| i == index)
        {
            let next = if self.current.is_some() {
                self.shuffle_pos + 1
            } else {
                0
            };
            if pos >= next {
                let track = self.shuffle_order.remove(pos);
                self.shuffle_order.insert(next, track);
                self.shuffle_pos = next;
            } else {
                // Already played this round: continue from its slot.
                self.shuffle_pos = pos;
            }
        }
        self.current = Some(index);
        true
    }

    /// Index of the track with this VFS path.
    pub fn position_of(&self, path: &str) -> Option<usize> {
        self.tracks.iter().position(|t| t.path == path)
    }

    /// Whether shuffle is on.
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Turn shuffle on or off. Turning it on draws a fresh shuffle order
    /// that starts at the current track.
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        if on {
            self.rebuild_shuffle();
        } else {
            self.shuffle_order.clear();
            self.shuffle_pos = 0;
        }
    }

    /// Advance to the next track in the playlist.
    /// Returns `true` if there is a next track. Respects repeat mode and shuffle.
    pub fn advance(&mut self) -> bool {
        if self.tracks.is_empty() {
            return false;
        }

        if self.repeat == RepeatMode::One {
            // Stay on the same track.
            return self.current.is_some();
        }

        if self.shuffle {
            return self.next_shuffle();
        }

        match self.current {
            None => {
                self.current = Some(0);
                true
            },
            Some(i) => {
                if i + 1 < self.tracks.len() {
                    self.current = Some(i + 1);
                    true
                } else if self.repeat == RepeatMode::All {
                    self.current = Some(0);
                    true
                } else {
                    false
                }
            },
        }
    }

    /// Go back to the previous track in the playlist.
    /// Returns `true` if there is a previous track.
    pub fn go_back(&mut self) -> bool {
        if self.tracks.is_empty() {
            return false;
        }

        if self.repeat == RepeatMode::One {
            return self.current.is_some();
        }

        if self.shuffle {
            return self.prev_shuffle();
        }

        match self.current {
            None => {
                self.current = Some(0);
                true
            },
            Some(i) => {
                if i > 0 {
                    self.current = Some(i - 1);
                    true
                } else if self.repeat == RepeatMode::All {
                    self.current = Some(self.tracks.len() - 1);
                    true
                } else {
                    false
                }
            },
        }
    }

    /// Get an immutable reference to all tracks.
    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// Next value of the xorshift64 generator.
    fn next_random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// Draw a fresh random shuffle order (Fisher-Yates) with the current
    /// track, if any, first, and put the cursor on it.
    fn rebuild_shuffle(&mut self) {
        let n = self.tracks.len();
        let cur = self.current.filter(|&c| c < n);
        let mut order: Vec<usize> = (0..n).filter(|&i| Some(i) != cur).collect();
        for i in (1..order.len()).rev() {
            let j = (self.next_random() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        if let Some(c) = cur {
            order.insert(0, c);
        }
        self.shuffle_order = order;
        self.shuffle_pos = 0;
    }

    fn next_shuffle(&mut self) -> bool {
        if self.shuffle_order.len() != self.tracks.len() {
            self.rebuild_shuffle();
        }
        if self.current.is_none() {
            // Not started: play the first track of the order.
            self.shuffle_pos = 0;
            self.current = self.shuffle_order.first().copied();
            return self.current.is_some();
        }

        if self.shuffle_pos + 1 >= self.shuffle_order.len() {
            if self.repeat != RepeatMode::All {
                return false;
            }
            // Every track played: go round the same order again (so `next`
            // and `prev` stay inverses across the wrap).
            self.shuffle_pos = 0;
        } else {
            self.shuffle_pos += 1;
        }
        self.current = Some(self.shuffle_order[self.shuffle_pos]);
        true
    }

    fn prev_shuffle(&mut self) -> bool {
        if self.shuffle_order.len() != self.tracks.len() {
            self.rebuild_shuffle();
        }
        if self.current.is_none() {
            self.shuffle_pos = 0;
            self.current = self.shuffle_order.first().copied();
            return self.current.is_some();
        }
        if self.shuffle_pos > 0 {
            self.shuffle_pos -= 1;
            self.current = Some(self.shuffle_order[self.shuffle_pos]);
            true
        } else if self.repeat == RepeatMode::All {
            self.shuffle_pos = self.shuffle_order.len() - 1;
            self.current = Some(self.shuffle_order[self.shuffle_pos]);
            true
        } else {
            false
        }
    }
}

impl Default for Playlist {
    fn default() -> Self {
        Self::new()
    }
}

/// Format the playlist as a displayable string for terminal output.
pub fn format_playlist(playlist: &Playlist) -> String {
    if playlist.is_empty() {
        return "(empty playlist)".to_string();
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "Playlist ({} tracks, repeat: {}, shuffle: {})",
        playlist.len(),
        playlist.repeat,
        if playlist.shuffle { "on" } else { "off" }
    ));

    for (i, track) in playlist.tracks().iter().enumerate() {
        let marker = if Some(i) == playlist.current_index() {
            ">"
        } else {
            " "
        };
        let artist_part = if track.artist.is_empty() {
            String::new()
        } else {
            format!(" - {}", track.artist)
        };
        let dur = format_duration(track.duration_ms);
        lines.push(format!(
            " {marker} {i}. {}{artist_part} [{dur}]",
            track.title
        ));
    }
    lines.join("\n")
}

/// Format milliseconds as M:SS or H:MM:SS.
pub fn format_duration(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests;
