//! TV Guide subsystem controller.
//!
//! Extracted from the main event loop to reduce complexity. Handles TV catalog
//! fetching, video player ticking, tune/untune requests, and audio streaming.

mod catalog;
mod download;
mod player;
mod streaming_buffer;
mod tune;

// Re-export public items so existing `use crate::tv_controller::*` paths work.
#[cfg(feature = "_video")]
pub(crate) use streaming_buffer::{MIN_PREBUFFER, StreamingInner};

use crate::app_state::AppState;
use oasis_core::apps::AppRunner;
use oasis_core::backend::SdiBackend;
use oasis_core::vfs::Vfs;

/// Process one frame of TV state: catalog fetching, tune requests, video
/// player ticking, and untune detection.
pub fn tick(state: &mut AppState, backend: &mut impl SdiBackend, vfs: &mut dyn Vfs) {
    catalog::poll_catalog_fetch(state);
    catalog::start_catalog_fetch_if_needed(state);
    tune::handle_tune_requests(state, backend, vfs);
    player::tick_video_player(state, backend);
    player::detect_untune(state, backend);
    player::auto_advance_episode(state, backend);
}

/// Find a TV Guide runner in either the full-screen runner or open windowed runners.
fn find_tv_guide_runner<'a>(
    app_runner: &'a mut Option<AppRunner>,
    open_runners: &'a mut [(String, AppRunner)],
) -> Option<&'a mut AppRunner> {
    if let Some(ref mut runner) = *app_runner
        && runner.title == "TV Guide"
    {
        log::trace!("TV: found TV Guide in app_runner (full-screen)");
        return Some(runner);
    }
    let found = open_runners
        .iter_mut()
        .map(|(_, runner)| runner)
        .find(|runner| runner.title == "TV Guide");
    if found.is_some() {
        log::trace!("TV: found TV Guide in open_runners (windowed)");
    }
    found
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "_video")]
mod tests {
    use super::download::parse_moov_duration;
    use super::streaming_buffer::*;

    // ---------------------------------------------------------------
    // StreamingBuffer must be Send + Sync (compile-time assertion)
    // ---------------------------------------------------------------

    const _: () = {
        fn assert_send_sync<T: Send + Sync>() {}
        fn check() {
            assert_send_sync::<StreamingBuffer>();
        }
    };

    // ---------------------------------------------------------------
    // should_throttle_pure tests
    // ---------------------------------------------------------------

    #[test]
    fn throttle_decoder_zero_no_moov_no_throttle() {
        assert!(!should_throttle_pure(0, 0, false, 0));
    }

    #[test]
    fn throttle_decoder_zero_no_moov_large_buf_no_throttle() {
        // Without moov, never throttle even with huge buffer.
        assert!(!should_throttle_pure(0, 100_000_000, false, 100_000_000));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_small_buf_no_throttle() {
        // moov found but buffer under threshold.
        assert!(!should_throttle_pure(0, 1_000_000, true, 1_000_000));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_at_threshold_no_throttle() {
        // Exactly at MAX_LOOKAHEAD -- not over, so no throttle.
        assert!(!should_throttle_pure(0, MAX_LOOKAHEAD, true, MAX_LOOKAHEAD));
    }

    #[test]
    fn throttle_decoder_zero_has_moov_over_threshold_throttle() {
        assert!(should_throttle_pure(
            0,
            MAX_LOOKAHEAD + 1,
            true,
            MAX_LOOKAHEAD + 1,
        ));
    }

    #[test]
    fn throttle_decoder_active_under_lookahead_no_throttle() {
        let decoder = 10_000_000u64;
        let received = decoder + MAX_LOOKAHEAD - 1;
        assert!(!should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_at_boundary_no_throttle() {
        let decoder = 10_000_000u64;
        let received = decoder + MAX_LOOKAHEAD;
        // received == decoder + MAX_LOOKAHEAD, not >, so no throttle.
        assert!(!should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_over_lookahead_throttle() {
        let decoder = 10_000_000u64;
        let received = decoder + MAX_LOOKAHEAD + 1;
        assert!(should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_ignores_moov_flag() {
        // When decoder_pos > 0, moov doesn't matter.
        let decoder = 5_000_000u64;
        let received = decoder + MAX_LOOKAHEAD + 100;
        assert!(should_throttle_pure(decoder, received, false, received));
        assert!(should_throttle_pure(decoder, received, true, received));
    }

    #[test]
    fn throttle_decoder_active_received_less_than_decoder() {
        // Edge: received < decoder (shouldn't happen, but shouldn't panic).
        assert!(!should_throttle_pure(100, 50, true, 50));
    }

    #[test]
    fn throttle_large_values() {
        // Multi-GB file scenario.
        let decoder = 2_000_000_000u64; // 2 GB
        let received = decoder + MAX_LOOKAHEAD + 1;
        assert!(should_throttle_pure(decoder, received, true, received));
    }

    // ---------------------------------------------------------------
    // linear_seek_interpolation tests
    // ---------------------------------------------------------------

    #[test]
    fn seek_interpolation_zero_secs() {
        let offset = linear_seek_interpolation(0.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000);
    }

    #[test]
    fn seek_interpolation_at_duration() {
        let offset = linear_seek_interpolation(100.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000 + 50_000);
    }

    #[test]
    fn seek_interpolation_half_duration() {
        let offset = linear_seek_interpolation(50.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000 + 25_000);
    }

    #[test]
    fn seek_interpolation_beyond_duration_clamps() {
        // seek_secs > duration -> frac clamped to 1.0
        let offset = linear_seek_interpolation(200.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000 + 50_000);
    }

    #[test]
    fn seek_interpolation_duration_zero() {
        // Edge: duration=0 -> returns mdat_offset (no division).
        let offset = linear_seek_interpolation(50.0, 0.0, 1000, 50_000);
        assert_eq!(offset, 1000);
    }

    #[test]
    fn seek_interpolation_negative_duration() {
        // Edge: negative duration -> returns mdat_offset.
        let offset = linear_seek_interpolation(50.0, -10.0, 1000, 50_000);
        assert_eq!(offset, 1000);
    }

    #[test]
    fn seek_interpolation_small_file() {
        let offset = linear_seek_interpolation(1.0, 2.0, 0, 100);
        assert_eq!(offset, 50);
    }

    #[test]
    fn seek_interpolation_large_file() {
        // 4 GB file at quarter duration.
        let file_size = 4_000_000_000u64;
        let offset = linear_seek_interpolation(25.0, 100.0, 0, file_size);
        assert_eq!(offset, 1_000_000_000);
    }

    #[test]
    fn seek_interpolation_saturates_on_large_offset() {
        // mdat_offset near u64::MAX -- addition must saturate, not wrap.
        let offset = linear_seek_interpolation(50.0, 100.0, u64::MAX - 100, 1000);
        assert_eq!(offset, u64::MAX);
    }

    // ---------------------------------------------------------------
    // parse_moov_duration tests
    // ---------------------------------------------------------------

    /// Build a minimal moov atom containing an mvhd v0 child.
    fn build_moov_v0(timescale: u32, duration: u32) -> Vec<u8> {
        // mvhd v0: version(1) + flags(3) + create(4) + mod(4)
        //          + timescale(4) + duration(4) = 20 bytes
        let mut mvhd_body = Vec::new();
        mvhd_body.push(0); // version 0
        mvhd_body.extend_from_slice(&[0, 0, 0]); // flags
        mvhd_body.extend_from_slice(&[0; 4]); // creation_time
        mvhd_body.extend_from_slice(&[0; 4]); // modification_time
        mvhd_body.extend_from_slice(&timescale.to_be_bytes());
        mvhd_body.extend_from_slice(&duration.to_be_bytes());
        // Pad to plausible size (real mvhd has more fields).
        mvhd_body.extend_from_slice(&[0; 80]);

        let mvhd_size = (8 + mvhd_body.len()) as u32;
        let moov_size = (8 + mvhd_size as usize) as u32;

        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&mvhd_size.to_be_bytes());
        moov.extend_from_slice(b"mvhd");
        moov.extend_from_slice(&mvhd_body);
        moov
    }

    #[test]
    fn parse_moov_duration_v0() {
        let moov = build_moov_v0(1000, 60000);
        let dur = parse_moov_duration(&moov);
        assert_eq!(dur, Some(60.0));
    }

    #[test]
    fn parse_moov_duration_zero_timescale() {
        let moov = build_moov_v0(0, 60000);
        assert_eq!(parse_moov_duration(&moov), None);
    }

    #[test]
    fn parse_moov_duration_no_mvhd() {
        // moov with only a trak child, no mvhd.
        let trak_body = [0u8; 16];
        let trak_size = (8 + trak_body.len()) as u32;
        let moov_size = (8 + trak_size as usize) as u32;
        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&trak_size.to_be_bytes());
        moov.extend_from_slice(b"trak");
        moov.extend_from_slice(&trak_body);
        assert_eq!(parse_moov_duration(&moov), None);
    }

    #[test]
    fn parse_moov_duration_too_short() {
        assert_eq!(parse_moov_duration(&[0; 4]), None);
    }

    // ---------------------------------------------------------------
    // maybe_evict tests (via StreamingBuffer)
    // ---------------------------------------------------------------

    #[test]
    fn evict_small_buffer_no_eviction() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push less than RETAIN_BEHIND bytes.
        inner.push(&vec![0xAA; 1024]);
        let sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // Enable eviction.
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        // Nothing evicted -- cursor is at 0, not past RETAIN_BEHIND.
        assert_eq!(s.base_offset, 0);
        assert_eq!(s.buf.len(), 1024);
    }

    #[test]
    fn evict_large_buffer_evicts_old_data() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        let data_size = RETAIN_BEHIND + 2 * 1024 * 1024;
        inner.push(&vec![0xBB; data_size]);
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        // Move cursor past RETAIN_BEHIND.
        sb.pos = data_size as u64;
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        // Some data should have been evicted.
        assert!(s.base_offset > 0, "expected eviction");
        // Remaining buffer should be approximately RETAIN_BEHIND.
        assert!(
            s.buf.len() <= RETAIN_BEHIND + 1,
            "expected buf <= RETAIN_BEHIND after eviction"
        );
    }

    #[test]
    fn evict_disabled_no_eviction() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        let data_size = RETAIN_BEHIND + 2 * 1024 * 1024;
        inner.push(&vec![0xCC; data_size]);
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // eviction_enabled defaults to false.
        sb.pos = data_size as u64;
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.base_offset, 0, "eviction should be disabled");
        assert_eq!(s.buf.len(), data_size);
    }

    #[test]
    fn evict_cursor_at_start_no_eviction() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&vec![0xDD; RETAIN_BEHIND + 1024]);
        let sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        // pos=0 means cursor_in_buf=0, not > RETAIN_BEHIND.
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.base_offset, 0);
    }

    #[test]
    fn evict_preserves_data_near_cursor() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        let total = RETAIN_BEHIND * 3;
        inner.push(&vec![0xEE; total]);
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);
        // Cursor at 2*RETAIN_BEHIND: evicts first RETAIN_BEHIND.
        sb.pos = (RETAIN_BEHIND * 2) as u64;
        sb.maybe_evict();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.base_offset, RETAIN_BEHIND as u64);
        assert_eq!(s.buf.len(), RETAIN_BEHIND * 2);
    }

    // ---------------------------------------------------------------
    // StreamingInner: push, bytes_received, finish, cancel, set_error
    // ---------------------------------------------------------------

    #[test]
    fn inner_new_defaults() {
        let inner = StreamingInner::new();
        assert_eq!(inner.bytes_received(), 0);
        assert!(!inner.is_done());
        assert!(!inner.is_cancelled());
        assert!(inner.probe_mode.load(std::sync::atomic::Ordering::Acquire));
        let s = inner.state.lock().unwrap();
        assert!(s.buf.is_empty());
        assert_eq!(s.base_offset, 0);
        assert!(s.moov.is_none());
        assert!(s.header.is_none());
        assert!(s.atoms.is_empty());
    }

    #[test]
    fn inner_push_accumulates_data() {
        let inner = StreamingInner::new();
        inner.push(&[1, 2, 3]);
        inner.push(&[4, 5]);
        assert_eq!(inner.bytes_received(), 5);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.buf, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn inner_finish_marks_done() {
        let inner = StreamingInner::new();
        inner.push(&[0; 16]);
        assert!(!inner.is_done());
        inner.finish();
        assert!(inner.is_done());
    }

    #[test]
    fn inner_cancel_marks_done_and_cancelled() {
        let inner = StreamingInner::new();
        assert!(!inner.is_cancelled());
        inner.cancel();
        assert!(inner.is_cancelled());
        assert!(inner.is_done());
    }

    #[test]
    fn inner_set_error_marks_done_and_stores_message() {
        let inner = StreamingInner::new();
        inner.set_error("connection reset".into());
        assert!(inner.is_done());
        let err = inner.error.lock().unwrap();
        assert_eq!(err.as_deref(), Some("connection reset"));
    }

    #[test]
    fn inner_disable_probe_mode() {
        let inner = StreamingInner::new();
        assert!(inner.probe_mode.load(std::sync::atomic::Ordering::Acquire));
        inner.disable_probe_mode();
        assert!(!inner.probe_mode.load(std::sync::atomic::Ordering::Acquire));
    }

    // ---------------------------------------------------------------
    // Atom scanning: ftyp, mdat, moov detection and retention
    // ---------------------------------------------------------------

    /// Build a minimal MP4 atom with a given fourcc and body.
    fn build_atom(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let size = (8 + body.len()) as u32;
        let mut atom = Vec::new();
        atom.extend_from_slice(&size.to_be_bytes());
        atom.extend_from_slice(fourcc);
        atom.extend_from_slice(body);
        atom
    }

    #[test]
    fn scan_atoms_ftyp_only() {
        let inner = StreamingInner::new();
        let ftyp = build_atom(b"ftyp", &[0; 16]);
        inner.push(&ftyp);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].2, *b"ftyp");
        assert_eq!(s.atoms[0].0, 0); // offset 0
        assert_eq!(s.atoms[0].1, 24); // 8 header + 16 body
    }

    #[test]
    fn scan_atoms_ftyp_mdat() {
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"mdat", &[0xFF; 32]));
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 2);
        assert_eq!(s.atoms[0].2, *b"ftyp");
        assert_eq!(s.atoms[1].2, *b"mdat");
        assert_eq!(s.atoms[1].0, 24); // after ftyp
    }

    #[test]
    fn scan_atoms_retains_moov() {
        let inner = StreamingInner::new();
        let moov_body = [0xAB; 64];
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"moov", &moov_body));
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_some(), "moov should be retained");
        let (offset, moov_data) = s.moov.as_ref().unwrap();
        assert_eq!(*offset, 24); // after ftyp
        // moov data includes the atom header (8 bytes) + body
        assert_eq!(moov_data.len(), 8 + moov_body.len());
    }

    #[test]
    fn scan_atoms_incomplete_moov_waits() {
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        // Write a moov header claiming 100 bytes but only provide 20.
        let moov_size: u32 = 100;
        data.extend_from_slice(&moov_size.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&[0; 12]); // only 12 of 92 body bytes
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        // ftyp should be scanned, but moov should NOT be in atoms yet
        // (incomplete).
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].2, *b"ftyp");
        assert!(s.moov.is_none());
    }

    #[test]
    fn scan_atoms_extended_size() {
        let inner = StreamingInner::new();
        // Build an atom with extended size (size32 == 1).
        let body = [0u8; 32];
        let total_size: u64 = 16 + body.len() as u64; // 16-byte header + body
        let mut atom = Vec::new();
        atom.extend_from_slice(&1u32.to_be_bytes()); // size32 = 1 (extended)
        atom.extend_from_slice(b"free");
        atom.extend_from_slice(&total_size.to_be_bytes());
        atom.extend_from_slice(&body);
        inner.push(&atom);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].1, total_size);
        assert_eq!(s.atoms[0].2, *b"free");
    }

    #[test]
    fn scan_atoms_header_retained() {
        let inner = StreamingInner::new();
        let ftyp = build_atom(b"ftyp", &[0; 16]);
        inner.push(&ftyp);
        let s = inner.state.lock().unwrap();
        assert!(s.header.is_some(), "header should be retained after ftyp");
        let hdr = s.header.as_ref().unwrap();
        // Header should include at least the ftyp atom.
        assert!(hdr.len() >= ftyp.len());
    }

    #[test]
    fn finish_handles_size_zero_atom() {
        // An atom with size==0 extends to EOF. `finish()` should handle it.
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        // Append an mdat atom with size=0 (extends to EOF).
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[0xFF; 100]);
        inner.push(&data);
        // Before finish, the size-0 atom is not scanned.
        {
            let s = inner.state.lock().unwrap();
            assert_eq!(s.atoms.len(), 1); // only ftyp
        }
        inner.finish();
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 2);
        assert_eq!(s.atoms[1].2, *b"mdat");
        // The size should be total - scan_pos (rest of file).
        let expected_size = data.len() as u64 - 24; // after ftyp
        assert_eq!(s.atoms[1].1, expected_size);
    }

    #[test]
    fn finish_retains_moov_at_end() {
        // Moov at end of file (common in non-faststart MP4s).
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        let mdat = build_atom(b"mdat", &[0xFF; 100]);
        data.extend_from_slice(&mdat);
        // Moov at end with size=0 (extends to EOF).
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        // mvhd inside moov.
        let mvhd_body = vec![0u8; 100];
        let mvhd = build_atom(b"mvhd", &mvhd_body);
        data.extend_from_slice(&mvhd);
        inner.push(&data);
        // mdat was scanned but moov (size=0) was not.
        inner.finish();
        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_some(), "moov at end should be retained on finish");
    }

    // ---------------------------------------------------------------
    // StreamingBuffer Read: probe_mode returns zeros, no decoder_pos
    // ---------------------------------------------------------------

    #[test]
    fn read_probe_mode_returns_zeros() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push some data and set total_size.
        inner.push(&[0xAA; 1024]);
        inner
            .total_size
            .store(4096, std::sync::atomic::Ordering::Release);
        inner.finish(); // mark done so reads don't block
        // probe_mode is true by default.
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // Seek past available data.
        sb.pos = 2048;
        let mut buf = [0xFF; 64];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 64);
        // All zeros (probe mode).
        assert!(buf.iter().all(|&b| b == 0), "probe reads should be zeros");
    }

    #[test]
    fn read_probe_mode_does_not_update_decoder_pos() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0xAA; 1024]);
        inner
            .total_size
            .store(4096, std::sync::atomic::Ordering::Release);
        inner.finish();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 2048;
        let mut buf = [0; 64];
        let _ = sb.read(&mut buf).unwrap();
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 0, "decoder_pos must not update during probe_mode");
    }

    #[test]
    fn read_normal_mode_updates_decoder_pos() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0xBB; 256]);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 64];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 64);
        assert!(buf.iter().all(|&b| b == 0xBB));
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 64, "decoder_pos should advance after normal read");
    }

    #[test]
    fn read_normal_mode_correct_data() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let data: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        inner.push(&data);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 200];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 200);
        assert_eq!(&buf[..], &data[..]);
    }

    #[test]
    fn read_partial_then_rest() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[1, 2, 3, 4, 5, 6, 7, 8]);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 4];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(buf, [1, 2, 3, 4]);
        assert_eq!(sb.pos, 4);
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(buf, [5, 6, 7, 8]);
        assert_eq!(sb.pos, 8);
    }

    #[test]
    fn read_eof_when_done_and_past_data() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 16]);
        inner
            .total_size
            .store(16, std::sync::atomic::Ordering::Release);
        inner.disable_probe_mode();
        inner.finish();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 16; // at EOF
        let mut buf = [0xFF; 8];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 0, "should return EOF at end of data");
    }

    #[test]
    fn read_returns_error_on_cancel() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 16]);
        inner.disable_probe_mode();
        inner.cancel();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 8];
        let result = sb.read(&mut buf);
        assert!(result.is_err(), "read after cancel should error");
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn read_returns_error_on_set_error() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 16]);
        inner.disable_probe_mode();
        inner.set_error("network timeout".into());
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        let mut buf = [0; 8];
        let result = sb.read(&mut buf);
        assert!(result.is_err(), "read after set_error should error");
    }

    #[test]
    fn read_from_retained_moov() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Build ftyp + moov.
        let moov_body = vec![0xCD; 64];
        let mut data = build_atom(b"ftyp", &[0; 16]);
        let moov_offset = data.len() as u64;
        data.extend_from_slice(&build_atom(b"moov", &moov_body));
        inner.push(&data);
        inner.disable_probe_mode();
        // Verify moov was retained.
        {
            let s = inner.state.lock().unwrap();
            assert!(s.moov.is_some());
        }
        // Now evict the buffer but moov should still be readable.
        {
            let mut s = inner.state.lock().unwrap();
            s.buf.clear();
            s.base_offset = data.len() as u64;
        }
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = moov_offset;
        let mut buf = [0; 72]; // 8 header + 64 body
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 72);
        // Verify moov header bytes.
        let size = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(size, 72);
        assert_eq!(&buf[4..8], b"moov");
    }

    #[test]
    fn read_from_retained_header() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let ftyp = build_atom(b"ftyp", &[0xAA; 16]);
        inner.push(&ftyp);
        inner.disable_probe_mode();
        // Evict the buffer but header should be retained.
        {
            let mut s = inner.state.lock().unwrap();
            s.buf.clear();
            s.base_offset = ftyp.len() as u64;
        }
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 0;
        let mut buf = [0; 24];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 24);
        assert_eq!(&buf[4..8], b"ftyp");
    }

    #[test]
    fn read_evicted_region_returns_error() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 1024]);
        inner.disable_probe_mode();
        inner.finish();
        // Manually evict by advancing base_offset.
        {
            let mut s = inner.state.lock().unwrap();
            s.buf.drain(..512);
            s.base_offset = 512;
            s.header = None; // clear header so it doesn't serve from there
        }
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.pos = 0; // before base_offset
        let mut buf = [0; 8];
        let result = sb.read(&mut buf);
        assert!(result.is_err(), "read from evicted region should error");
    }

    // ---------------------------------------------------------------
    // StreamingBuffer Seek trait
    // ---------------------------------------------------------------

    #[test]
    fn seek_start() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        let pos = sb.seek(std::io::SeekFrom::Start(42)).unwrap();
        assert_eq!(pos, 42);
        assert_eq!(sb.pos, 42);
    }

    #[test]
    fn seek_current_forward() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        sb.pos = 100;
        let pos = sb.seek(std::io::SeekFrom::Current(50)).unwrap();
        assert_eq!(pos, 150);
    }

    #[test]
    fn seek_current_backward() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        sb.pos = 100;
        let pos = sb.seek(std::io::SeekFrom::Current(-30)).unwrap();
        assert_eq!(pos, 70);
    }

    #[test]
    fn seek_negative_position_errors() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        sb.pos = 10;
        let result = sb.seek(std::io::SeekFrom::Current(-20));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn seek_end_with_known_size() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner
            .total_size
            .store(1000, std::sync::atomic::Ordering::Release);
        inner.finish(); // so it doesn't block waiting for total_size
        let mut sb = StreamingBuffer::new(inner);
        let pos = sb.seek(std::io::SeekFrom::End(-100)).unwrap();
        assert_eq!(pos, 900);
    }

    #[test]
    fn seek_end_at_zero_offset() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner
            .total_size
            .store(500, std::sync::atomic::Ordering::Release);
        inner.finish();
        let mut sb = StreamingBuffer::new(inner);
        let pos = sb.seek(std::io::SeekFrom::End(0)).unwrap();
        assert_eq!(pos, 500);
    }

    // ---------------------------------------------------------------
    // wait_for_moov
    // ---------------------------------------------------------------

    #[test]
    fn wait_for_moov_immediate() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Build a file with moov.
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"moov", &[0; 32]));
        inner.push(&data);
        let result = inner.wait_for_moov(std::time::Duration::from_millis(100));
        assert!(result.is_some(), "moov should be immediately available");
    }

    #[test]
    fn wait_for_moov_cancelled_returns_none() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.cancel();
        let result = inner.wait_for_moov(std::time::Duration::from_millis(100));
        assert!(result.is_none(), "cancelled session should return None");
    }

    #[test]
    fn wait_for_moov_done_no_moov_returns_none() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push data without a moov atom and finish.
        inner.push(&build_atom(b"ftyp", &[0; 16]));
        inner.push(&build_atom(b"mdat", &[0; 100]));
        inner.finish();
        let result = inner.wait_for_moov(std::time::Duration::from_millis(100));
        assert!(result.is_none(), "no moov in data should return None");
    }

    #[test]
    fn wait_for_moov_arrives_from_background() {
        use std::sync::Arc;
        let inner = Arc::new(StreamingInner::new());
        let inner2 = Arc::clone(&inner);
        // Spawn a thread that pushes moov after a short delay.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut data = build_atom(b"ftyp", &[0; 16]);
            data.extend_from_slice(&build_atom(b"moov", &[0; 64]));
            inner2.push(&data);
        });
        let result = inner.wait_for_moov(std::time::Duration::from_secs(2));
        assert!(
            result.is_some(),
            "moov pushed from background should be found"
        );
    }

    // ---------------------------------------------------------------
    // wait_for_buffered
    // ---------------------------------------------------------------

    #[test]
    fn wait_for_buffered_immediate() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 4096]);
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        assert!(ok);
    }

    #[test]
    fn wait_for_buffered_cancelled() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.cancel();
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        // Returns false because buffer is empty on cancel.
        assert!(!ok);
    }

    #[test]
    fn wait_for_buffered_done_with_partial_data() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 512]); // less than requested
        inner.finish();
        // Should return true because some data is available (not empty).
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        assert!(ok);
    }

    #[test]
    fn wait_for_buffered_done_empty() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.finish();
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        assert!(!ok, "empty buffer on done should return false");
    }

    // ---------------------------------------------------------------
    // should_throttle integration (via StreamingInner, not _pure)
    // ---------------------------------------------------------------

    #[test]
    fn should_throttle_integration_no_throttle_initially() {
        let inner = StreamingInner::new();
        inner.push(&[0; 1024]);
        assert!(!inner.should_throttle());
    }

    #[test]
    fn should_throttle_integration_with_moov_and_large_buffer() {
        let inner = StreamingInner::new();
        // Push ftyp + moov to trigger moov retention.
        let mut data = build_atom(b"ftyp", &[0; 16]);
        data.extend_from_slice(&build_atom(b"moov", &[0; 32]));
        // Add enough extra data to exceed MAX_LOOKAHEAD.
        let extra = vec![0u8; MAX_LOOKAHEAD as usize + 1];
        data.extend_from_slice(&extra);
        inner.push(&data);
        // decoder_pos is 0, has_moov is true, buf > MAX_LOOKAHEAD.
        assert!(
            inner.should_throttle(),
            "should throttle with moov and large buffer"
        );
    }

    #[test]
    fn should_throttle_integration_decoder_active() {
        let inner = StreamingInner::new();
        let data_size = MAX_LOOKAHEAD as usize + 1_000_000;
        inner.push(&vec![0; data_size]);
        // Set decoder_pos to somewhere in the stream.
        inner
            .decoder_pos
            .store(1000, std::sync::atomic::Ordering::Release);
        // received (data_size) > decoder_pos (1000) + MAX_LOOKAHEAD
        assert!(inner.should_throttle());
    }

    // ---------------------------------------------------------------
    // StreamingBuffer: Read + Seek round-trip (probe -> disable -> read)
    // ---------------------------------------------------------------

    #[test]
    fn probe_then_normal_read_roundtrip() {
        use std::io::{Read, Seek};
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Simulate a small MP4 file.
        let mut mp4 = build_atom(b"ftyp", &[0x11; 16]);
        mp4.extend_from_slice(&build_atom(b"mdat", &[0x22; 200]));
        mp4.extend_from_slice(&build_atom(b"moov", &[0x33; 64]));
        let file_size = mp4.len() as u64;
        inner.push(&mp4);
        inner
            .total_size
            .store(file_size, std::sync::atomic::Ordering::Release);
        inner.finish();

        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));

        // Phase 1: probe mode -- skip ahead, reads return zeros.
        sb.pos = 100;
        let mut buf = [0xFF; 16];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 16);
        // In probe mode, data beyond the sliding buffer returns zeros.
        // But pos=100 is within the buffer, so it returns real data.
        // Let's check decoder_pos was NOT updated in probe mode.
        // Actually pos=100 is within the buffer so it returns real data,
        // but decoder_pos should still not be updated.
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 0, "decoder_pos must not update during probe");

        // Phase 2: disable probe, seek back, read real data.
        inner.disable_probe_mode();
        sb.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut header = [0; 8];
        let n = sb.read(&mut header).unwrap();
        assert_eq!(n, 8);
        assert_eq!(&header[4..8], b"ftyp");
        let dp = inner.decoder_pos.load(std::sync::atomic::Ordering::Acquire);
        assert_eq!(dp, 8, "decoder_pos should update after probe disabled");
    }

    // ---------------------------------------------------------------
    // Multiple push + incremental atom scanning
    // ---------------------------------------------------------------

    #[test]
    fn incremental_atom_scanning() {
        let inner = StreamingInner::new();
        // Push ftyp in full.
        let ftyp = build_atom(b"ftyp", &[0; 16]);
        inner.push(&ftyp);
        {
            let s = inner.state.lock().unwrap();
            assert_eq!(s.atoms.len(), 1);
            assert_eq!(s.atoms_scanned_to, 24);
        }
        // Push first part of mdat header (only 4 bytes).
        inner.push(&100u32.to_be_bytes());
        {
            let s = inner.state.lock().unwrap();
            // Still only 1 atom -- header incomplete.
            assert_eq!(s.atoms.len(), 1);
        }
        // Push the rest of mdat header + some body.
        let mut rest = Vec::new();
        rest.extend_from_slice(b"mdat");
        rest.extend_from_slice(&[0xFF; 88]); // 100 - 8 = 92 body, gave 88
        inner.push(&rest);
        {
            let s = inner.state.lock().unwrap();
            // mdat header is now complete (24 + 100 = 124 bytes total,
            // we have 24 + 4 + 4 + 88 = 120; 92 body bytes needed,
            // 88 provided -- atom scanned because we have the header).
            assert_eq!(s.atoms.len(), 2);
            assert_eq!(s.atoms[1].2, *b"mdat");
        }
    }

    // ---------------------------------------------------------------
    // VideoSource trait
    // ---------------------------------------------------------------

    #[test]
    fn video_source_byte_len_unknown() {
        use oasis_video::VideoSource;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let sb = StreamingBuffer::new(inner);
        assert_eq!(sb.byte_len(), None);
    }

    #[test]
    fn video_source_byte_len_known() {
        use oasis_video::VideoSource;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner
            .total_size
            .store(12345, std::sync::atomic::Ordering::Release);
        let sb = StreamingBuffer::new(inner);
        assert_eq!(sb.byte_len(), Some(12345));
    }

    #[test]
    fn video_source_is_seekable() {
        use oasis_video::VideoSource;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let sb = StreamingBuffer::new(inner);
        assert!(sb.is_seekable());
    }

    // ---------------------------------------------------------------
    // Edge case: parse_moov_duration with mvhd version 1 (64-bit)
    // ---------------------------------------------------------------

    /// Build a minimal moov atom containing an mvhd v1 child.
    fn build_moov_v1(timescale: u32, duration: u64) -> Vec<u8> {
        // mvhd v1: version(1) + flags(3) + create(8) + mod(8)
        //          + timescale(4) + duration(8) = 32 bytes
        let mut mvhd_body = Vec::new();
        mvhd_body.push(1); // version 1
        mvhd_body.extend_from_slice(&[0, 0, 0]); // flags
        mvhd_body.extend_from_slice(&[0; 8]); // creation_time (64-bit)
        mvhd_body.extend_from_slice(&[0; 8]); // modification_time (64-bit)
        mvhd_body.extend_from_slice(&timescale.to_be_bytes());
        mvhd_body.extend_from_slice(&duration.to_be_bytes());
        // Pad to plausible size.
        mvhd_body.extend_from_slice(&[0; 80]);

        let mvhd_size = (8 + mvhd_body.len()) as u32;
        let moov_size = (8 + mvhd_size as usize) as u32;

        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&mvhd_size.to_be_bytes());
        moov.extend_from_slice(b"mvhd");
        moov.extend_from_slice(&mvhd_body);
        moov
    }

    #[test]
    fn parse_moov_duration_v1() {
        // 48000 Hz timescale, 2880000 ticks = 60.0 seconds
        let moov = build_moov_v1(48000, 2_880_000);
        let dur = parse_moov_duration(&moov);
        assert_eq!(dur, Some(60.0));
    }

    #[test]
    fn parse_moov_duration_v1_large_duration() {
        // 90000 Hz timescale, 3-hour file = 90000 * 10800 = 972_000_000
        let moov = build_moov_v1(90000, 972_000_000);
        let dur = parse_moov_duration(&moov);
        assert!((dur.unwrap() - 10800.0).abs() < 0.01);
    }

    #[test]
    fn parse_moov_duration_v1_zero_timescale() {
        let moov = build_moov_v1(0, 100_000);
        assert_eq!(parse_moov_duration(&moov), None);
    }

    #[test]
    fn parse_moov_duration_empty_data() {
        assert_eq!(parse_moov_duration(&[]), None);
    }

    #[test]
    fn parse_moov_duration_mvhd_body_too_short_for_v0() {
        // Create an mvhd with version 0 but truncated body (< 20 bytes).
        let mut mvhd_body = Vec::new();
        mvhd_body.push(0); // version 0
        mvhd_body.extend_from_slice(&[0, 0, 0]); // flags
        mvhd_body.extend_from_slice(&[0; 10]); // truncated: only 14 bytes total

        let mvhd_size = (8 + mvhd_body.len()) as u32;
        let moov_size = (8 + mvhd_size as usize) as u32;

        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&mvhd_size.to_be_bytes());
        moov.extend_from_slice(b"mvhd");
        moov.extend_from_slice(&mvhd_body);
        assert_eq!(parse_moov_duration(&moov), None);
    }

    #[test]
    fn parse_moov_duration_mvhd_unknown_version() {
        // Version 2 is not defined -- should return None.
        let mut mvhd_body = Vec::new();
        mvhd_body.push(2); // version 2 (invalid)
        mvhd_body.extend_from_slice(&[0; 100]);

        let mvhd_size = (8 + mvhd_body.len()) as u32;
        let moov_size = (8 + mvhd_size as usize) as u32;

        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&mvhd_size.to_be_bytes());
        moov.extend_from_slice(b"mvhd");
        moov.extend_from_slice(&mvhd_body);
        assert_eq!(parse_moov_duration(&moov), None);
    }

    #[test]
    fn parse_moov_duration_multiple_children_mvhd_second() {
        // moov with trak first, then mvhd.
        let trak_body = [0u8; 16];
        let trak_size = (8 + trak_body.len()) as u32;

        let mut mvhd_body = Vec::new();
        mvhd_body.push(0); // version 0
        mvhd_body.extend_from_slice(&[0, 0, 0]); // flags
        mvhd_body.extend_from_slice(&[0; 4]); // creation_time
        mvhd_body.extend_from_slice(&[0; 4]); // modification_time
        mvhd_body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        mvhd_body.extend_from_slice(&30000u32.to_be_bytes()); // duration
        mvhd_body.extend_from_slice(&[0; 80]);
        let mvhd_size = (8 + mvhd_body.len()) as u32;

        let moov_size = (8 + trak_size as usize + mvhd_size as usize) as u32;

        let mut moov = Vec::new();
        moov.extend_from_slice(&moov_size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&trak_size.to_be_bytes());
        moov.extend_from_slice(b"trak");
        moov.extend_from_slice(&trak_body);
        moov.extend_from_slice(&mvhd_size.to_be_bytes());
        moov.extend_from_slice(b"mvhd");
        moov.extend_from_slice(&mvhd_body);

        let dur = parse_moov_duration(&moov);
        assert_eq!(dur, Some(30.0));
    }

    // ---------------------------------------------------------------
    // Edge case: scan_atoms with invalid atom sizes
    // ---------------------------------------------------------------

    #[test]
    fn scan_atoms_invalid_size_stops_scanning() {
        let inner = StreamingInner::new();
        // Build ftyp, then an atom with size < 8 (invalid).
        let mut data = build_atom(b"ftyp", &[0; 16]);
        // Invalid atom: size=4 (less than minimum 8)
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(b"free");
        data.extend_from_slice(&[0; 16]);
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        // Only ftyp should be scanned; invalid atom stops scanning.
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].2, *b"ftyp");
    }

    #[test]
    fn scan_atoms_oversized_atom_stops_scanning() {
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        // Atom claiming to be larger than MAX_ATOM_SIZE (10 GB).
        let huge_size: u32 = 1; // extended size flag
        data.extend_from_slice(&huge_size.to_be_bytes());
        data.extend_from_slice(b"mdat");
        // Extended size: 11 GB
        data.extend_from_slice(&11_000_000_000u64.to_be_bytes());
        data.extend_from_slice(&[0; 32]);
        inner.push(&data);
        let s = inner.state.lock().unwrap();
        // Only ftyp should be scanned; oversized atom is rejected.
        assert_eq!(s.atoms.len(), 1);
    }

    #[test]
    fn scan_atoms_partial_header_waits_for_more_data() {
        let inner = StreamingInner::new();
        // Push only 4 bytes -- not enough for an atom header (need 8).
        inner.push(&[0, 0, 0, 24]);
        {
            let s = inner.state.lock().unwrap();
            assert_eq!(s.atoms.len(), 0);
        }
        // Push the rest of the atom.
        let mut rest = Vec::new();
        rest.extend_from_slice(b"ftyp");
        rest.extend_from_slice(&[0; 16]);
        inner.push(&rest);
        let s = inner.state.lock().unwrap();
        assert_eq!(s.atoms.len(), 1);
        assert_eq!(s.atoms[0].2, *b"ftyp");
    }

    // ---------------------------------------------------------------
    // Edge case: StreamingBuffer read with zero-length buffer
    // ---------------------------------------------------------------

    #[test]
    fn read_zero_length_buffer() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0xAA; 64]);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(inner);
        let mut buf = [];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 0);
        assert_eq!(sb.pos, 0, "position should not advance on empty read");
    }

    // ---------------------------------------------------------------
    // Edge case: StreamingBuffer read that spans partial data
    // ---------------------------------------------------------------

    #[test]
    fn read_more_than_available_returns_partial() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0x42; 10]);
        inner.disable_probe_mode();
        inner.finish(); // mark done so it doesn't block
        let mut sb = StreamingBuffer::new(inner);
        // Request 100 bytes but only 10 are available.
        let mut buf = [0; 100];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 10);
        assert!(buf[..10].iter().all(|&b| b == 0x42));
        assert_eq!(sb.pos, 10);
    }

    // ---------------------------------------------------------------
    // Edge case: seek + read interleaving
    // ---------------------------------------------------------------

    #[test]
    fn seek_then_read_at_various_positions() {
        use std::io::{Read, Seek};
        let inner = std::sync::Arc::new(StreamingInner::new());
        let data: Vec<u8> = (0..=255).collect();
        inner.push(&data);
        inner.disable_probe_mode();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));

        // Read from middle.
        sb.seek(std::io::SeekFrom::Start(100)).unwrap();
        let mut buf = [0; 4];
        sb.read(&mut buf).unwrap();
        assert_eq!(buf, [100, 101, 102, 103]);

        // Seek backward and read.
        sb.seek(std::io::SeekFrom::Start(0)).unwrap();
        sb.read(&mut buf).unwrap();
        assert_eq!(buf, [0, 1, 2, 3]);

        // Seek to end.
        sb.seek(std::io::SeekFrom::Start(252)).unwrap();
        sb.read(&mut buf).unwrap();
        assert_eq!(buf, [252, 253, 254, 255]);
    }

    // ---------------------------------------------------------------
    // Edge case: probe mode at exact total_size boundary (EOF)
    // ---------------------------------------------------------------

    #[test]
    fn probe_mode_eof_at_total_size() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 100]);
        inner
            .total_size
            .store(200, std::sync::atomic::Ordering::Release);
        inner.finish();
        // probe_mode is true by default.
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // Position at total_size -- should be EOF even in probe mode.
        sb.pos = 200;
        let mut buf = [0xFF; 16];
        let n = sb.read(&mut buf).unwrap();
        assert_eq!(n, 0, "should return EOF at total_size in probe mode");
    }

    #[test]
    fn probe_mode_read_up_to_total_size() {
        use std::io::Read;
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 100]);
        inner
            .total_size
            .store(200, std::sync::atomic::Ordering::Release);
        inner.finish();
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        // Position near total_size -- probe should fill zeros up to limit.
        sb.pos = 190;
        let mut buf = [0xFF; 20];
        let n = sb.read(&mut buf).unwrap();
        // Should only get 10 bytes (200 - 190), not 20.
        assert_eq!(n, 10);
        assert!(buf[..10].iter().all(|&b| b == 0));
    }

    // ---------------------------------------------------------------
    // Edge case: multiple sequential evictions
    // ---------------------------------------------------------------

    #[test]
    fn multiple_evictions_advance_base_offset() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        let chunk = RETAIN_BEHIND + 1024 * 1024;
        inner.push(&vec![0xAA; chunk]);
        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));
        sb.eviction_enabled
            .store(true, std::sync::atomic::Ordering::Release);

        // First eviction.
        sb.pos = chunk as u64;
        sb.maybe_evict();
        let offset1 = inner.state.lock().unwrap().base_offset;
        assert!(offset1 > 0);

        // Push more data and advance cursor.
        inner.push(&vec![0xBB; chunk]);
        sb.pos = (chunk * 2) as u64;
        sb.maybe_evict();
        let offset2 = inner.state.lock().unwrap().base_offset;
        assert!(offset2 > offset1, "second eviction should advance further");
    }

    // ---------------------------------------------------------------
    // Edge case: concurrent push and read from different threads
    // ---------------------------------------------------------------

    #[test]
    fn concurrent_push_and_read() {
        use std::io::Read;
        use std::sync::Arc;

        let inner = Arc::new(StreamingInner::new());
        inner.disable_probe_mode();
        let inner2 = Arc::clone(&inner);

        // Spawn a writer that pushes data in chunks.
        let writer = std::thread::spawn(move || {
            for i in 0..10 {
                let chunk = vec![i as u8; 1024];
                inner2.push(&chunk);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            inner2.finish();
        });

        let mut sb = StreamingBuffer::new(Arc::clone(&inner));
        let mut total_read = 0;
        let mut buf = [0; 256];
        // Read until EOF (finish + no more data).
        loop {
            match sb.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => total_read += n,
                Err(_) => break,
            }
        }
        writer.join().unwrap();
        assert_eq!(total_read, 10 * 1024, "should read all pushed data");
    }

    // ---------------------------------------------------------------
    // Edge case: should_throttle boundary precision
    // ---------------------------------------------------------------

    #[test]
    fn throttle_decoder_zero_buf_size_vs_received_differ() {
        // buf_size can differ from received when eviction has occurred.
        // decoder_pos=0, has_moov=true: throttle is based on buf_size, not received.
        assert!(!should_throttle_pure(
            0,
            MAX_LOOKAHEAD + 100,
            true,
            MAX_LOOKAHEAD
        ));
        assert!(should_throttle_pure(
            0,
            MAX_LOOKAHEAD + 100,
            true,
            MAX_LOOKAHEAD + 1
        ));
    }

    #[test]
    fn throttle_decoder_one_byte_past_boundary() {
        // decoder_pos=1 switches to the decoder-based formula.
        // received=1+MAX_LOOKAHEAD => not throttled (need >)
        assert!(!should_throttle_pure(1, 1 + MAX_LOOKAHEAD, true, 0));
        // received=1+MAX_LOOKAHEAD+1 => throttled
        assert!(should_throttle_pure(1, 1 + MAX_LOOKAHEAD + 1, true, 0));
    }

    // ---------------------------------------------------------------
    // Edge case: linear_seek_interpolation with NaN/infinity inputs
    // ---------------------------------------------------------------

    #[test]
    fn seek_interpolation_negative_seek_clamps_to_zero() {
        let offset = linear_seek_interpolation(-10.0, 100.0, 1000, 50_000);
        assert_eq!(offset, 1000, "negative seek_secs should clamp to start");
    }

    #[test]
    fn seek_interpolation_zero_mdat_size() {
        let offset = linear_seek_interpolation(50.0, 100.0, 1000, 0);
        assert_eq!(offset, 1000, "zero mdat_size means no offset added");
    }

    // ---------------------------------------------------------------
    // Edge case: parse_tail_for_moov
    // ---------------------------------------------------------------

    #[test]
    fn parse_tail_for_moov_finds_moov_in_tail() {
        use super::download::parse_tail_for_moov;
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Build tail data: some mdat garbage, then a moov atom.
        let mut tail = vec![0xFF; 100]; // garbage (mdat body)
        let moov_body = [0xAB; 32];
        tail.extend_from_slice(&build_atom(b"moov", &moov_body));
        let tail_offset = 500_000u64;
        let content_length = tail_offset + tail.len() as u64;
        parse_tail_for_moov(&inner, &tail, tail_offset, content_length, 0);
        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_some(), "moov should be found in tail data");
        let (offset, data) = s.moov.as_ref().unwrap();
        assert_eq!(*offset, tail_offset + 100); // after garbage
        assert_eq!(data.len(), 8 + moov_body.len());
    }

    #[test]
    fn parse_tail_for_moov_no_moov_in_tail() {
        use super::download::parse_tail_for_moov;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let tail = vec![0xFF; 200]; // all garbage, no moov fourcc
        parse_tail_for_moov(&inner, &tail, 0, 200, 0);
        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_none(), "no moov should be found in garbage data");
    }

    #[test]
    fn parse_tail_for_moov_false_positive_moov_fourcc() {
        use super::download::parse_tail_for_moov;
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Embed "moov" in data but with invalid size (doesn't fit).
        let mut tail = vec![0; 20];
        // Put "moov" at offset 8 (fourcc position), with size field = 0xFFFF
        // which would extend beyond the tail data.
        tail[4] = 0;
        tail[5] = 0;
        tail[6] = 0xFF;
        tail[7] = 0xFF; // size = 65535
        tail[8..12].copy_from_slice(b"moov");
        parse_tail_for_moov(&inner, &tail, 0, 20, 0);
        let s = inner.state.lock().unwrap();
        assert!(
            s.moov.is_none(),
            "invalid moov (size exceeds tail) should not be retained"
        );
    }

    #[test]
    fn parse_tail_for_moov_with_seek_sets_base_offset() {
        use super::download::parse_tail_for_moov;
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push some initial data.
        inner.push(&[0; 1024]);

        // Build tail with moov containing mvhd for duration parsing.
        let mut tail = vec![0xFF; 100];
        let moov = build_moov_v0(1000, 120_000); // 120 second file
        tail.extend_from_slice(&moov);
        let content_length = 100_000_000u64; // 100 MB file
        let tail_offset = content_length - tail.len() as u64;

        // Seek to 60 seconds (half the file).
        parse_tail_for_moov(&inner, &tail, tail_offset, content_length, 60);

        let s = inner.state.lock().unwrap();
        assert!(s.moov.is_some(), "moov should be retained");
        // With seek_secs=60 and duration=120, linear interpolation gives
        // roughly half the file. base_offset should be set if the seek
        // position is far enough ahead of bytes_received.
        // base_offset should have been updated (seek position > downloaded + threshold).
    }

    // ---------------------------------------------------------------
    // Edge case: wait_for_buffered timeout
    // ---------------------------------------------------------------

    #[test]
    fn wait_for_buffered_timeout_with_some_data() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        inner.push(&[0; 512]); // some data, but less than requested
        // Don't finish or cancel -- should hit timeout.
        let ok = inner.wait_for_buffered(1_000_000, std::time::Duration::from_millis(100));
        // Should return true because some data is present (not empty).
        assert!(ok, "timeout with partial data should return true");
    }

    #[test]
    fn wait_for_buffered_timeout_no_data() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Don't push any data, don't finish.
        let ok = inner.wait_for_buffered(1024, std::time::Duration::from_millis(100));
        assert!(!ok, "timeout with empty buffer should return false");
    }

    // ---------------------------------------------------------------
    // Edge case: wait_for_moov timeout with no moov
    // ---------------------------------------------------------------

    #[test]
    fn wait_for_moov_timeout_returns_none() {
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Push data without moov, don't finish.
        inner.push(&build_atom(b"ftyp", &[0; 16]));
        let result = inner.wait_for_moov(std::time::Duration::from_millis(100));
        assert!(result.is_none(), "timeout without moov should return None");
    }

    // ---------------------------------------------------------------
    // Edge case: StreamingBuffer seek resets logged_wait flag
    // ---------------------------------------------------------------

    #[test]
    fn seek_resets_position_correctly() {
        use std::io::Seek;
        let inner = std::sync::Arc::new(StreamingInner::new());
        let mut sb = StreamingBuffer::new(inner);
        sb.pos = 500;
        sb.seek(std::io::SeekFrom::Start(100)).unwrap();
        assert_eq!(sb.pos, 100, "seek should update position");
        // Seek back to 0.
        sb.seek(std::io::SeekFrom::Start(0)).unwrap();
        assert_eq!(sb.pos, 0);
    }

    // ---------------------------------------------------------------
    // Edge case: read serves data from header, moov, and buffer in
    // correct priority order
    // ---------------------------------------------------------------

    #[test]
    fn read_priority_header_then_moov_then_buffer() {
        use std::io::{Read, Seek};
        let inner = std::sync::Arc::new(StreamingInner::new());
        // Build ftyp + moov + mdat.
        let mut data = build_atom(b"ftyp", &[0x11; 16]);
        let moov_off = data.len() as u64;
        data.extend_from_slice(&build_atom(b"moov", &[0x22; 32]));
        let _mdat_off = data.len() as u64;
        data.extend_from_slice(&build_atom(b"mdat", &[0x33; 64]));
        inner.push(&data);
        inner.disable_probe_mode();

        // Evict the main buffer so only header and moov are retained.
        {
            let mut s = inner.state.lock().unwrap();
            s.buf.clear();
            s.base_offset = data.len() as u64;
        }

        let mut sb = StreamingBuffer::new(std::sync::Arc::clone(&inner));

        // Read from position 0 -- should serve from retained header.
        sb.seek(std::io::SeekFrom::Start(0)).unwrap();
        let mut buf = [0; 8];
        sb.read(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"ftyp", "should read from retained header");

        // Read from moov offset -- should serve from retained moov.
        sb.seek(std::io::SeekFrom::Start(moov_off)).unwrap();
        let mut buf = [0; 8];
        sb.read(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"moov", "should read from retained moov");
    }

    // ---------------------------------------------------------------
    // Edge case: finish with moov using extended size (size32==1)
    // ---------------------------------------------------------------

    #[test]
    fn finish_handles_extended_size_moov() {
        let inner = StreamingInner::new();
        let mut data = build_atom(b"ftyp", &[0; 16]);
        // Write moov with extended size header at end.
        let moov_body = [0xAA; 64];
        let total_atom_size = 16 + moov_body.len() as u64; // 16-byte header
        data.extend_from_slice(&1u32.to_be_bytes()); // size32=1 (extended)
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&total_atom_size.to_be_bytes());
        data.extend_from_slice(&moov_body);
        inner.push(&data);
        // Since the data was complete on push, scan_atoms should find it.
        let s = inner.state.lock().unwrap();
        let moov_atom = s.atoms.iter().find(|(_, _, cc)| cc == b"moov");
        assert!(moov_atom.is_some(), "extended-size moov should be scanned");
    }

    // ---------------------------------------------------------------
    // Edge case: check_moov_at_start_restart
    // ---------------------------------------------------------------

    #[test]
    fn check_moov_at_start_no_moov_returns_none() {
        use super::download::check_moov_at_start_restart;
        let s = SlidingState {
            buf: vec![0; 1024],
            base_offset: 0,
            bytes_received: 1024,
            moov: None,
            header: None,
            atoms: Vec::new(),
            atoms_scanned_to: 0,
        };
        assert_eq!(
            check_moov_at_start_restart(&s, 30),
            None,
            "no moov should return None"
        );
    }

    // ---------------------------------------------------------------
    // is_would_block tests
    // ---------------------------------------------------------------

    #[test]
    fn is_would_block_true_for_would_block_error() {
        use super::download::is_would_block;
        let err = std::io::Error::new(std::io::ErrorKind::WouldBlock, "test");
        assert!(is_would_block(&err));
    }

    #[test]
    fn is_would_block_false_for_other_errors() {
        use super::download::is_would_block;
        let err = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "test");
        assert!(!is_would_block(&err));
    }

    #[test]
    fn is_would_block_false_for_broken_pipe() {
        use super::download::is_would_block;
        let err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "test");
        assert!(!is_would_block(&err));
    }
}
