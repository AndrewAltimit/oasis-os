#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    // Attempt to parse arbitrary bytes as an MP4 file.
    // This exercises read_box_header, parse_moov, parse_trak, parse_stbl,
    // parse_mp4a_sample_entry, parse_avcc, and all sub-parsers.
    let mut cursor = Cursor::new(data);
    let _ = oasis_video::demux_lite::Mp4Lite::open(&mut cursor);

    // Also test parse_moov_tracks for streaming path.
    let _ = oasis_video::demux_lite::parse_moov_tracks(data);
});
