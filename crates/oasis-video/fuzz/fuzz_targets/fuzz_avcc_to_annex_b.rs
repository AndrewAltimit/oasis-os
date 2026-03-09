#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    // Use first byte to select nal_length_size (1-4).
    let nls = ((data[0] % 4) + 1) as usize;
    let is_keyframe = data[1] & 1 != 0;
    let payload = &data[2..];

    let avcc = oasis_video::demux_lite::AvccConfig {
        nal_length_size: nls,
        sps: vec![0x67, 0x42, 0x00, 0x0a], // minimal SPS
        pps: vec![0x68, 0xce, 0x38, 0x80], // minimal PPS
    };

    let _ = oasis_video::demux_lite::avcc_to_annex_b(payload, &avcc, is_keyframe);
});
