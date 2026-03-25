//! Minimal bridge PRX for sceMpegVsh_library function resolution.
//!
//! This PRX imports from sceMpegVsh_library (weak stubs). When loaded
//! AFTER mpeg_vsh370.prx, the kernel resolves these stubs with the
//! correct syscall numbers. The EBOOT then reads the resolved stubs
//! from this module's memory to patch its own sceMpeg stubs.
//!
//! Build: cd crates/oasis-mpeg-bridge && RUST_PSP_BUILD_STD=1 cargo +nightly psp --release
//! Output: target/mipsel-sony-psp-std/release/oasis-mpeg-bridge.prx

#![no_std]
#![no_main]

use core::ffi::c_void;
use psp::sys::{
    SceMpeg, SceMpegStream, SceMpegRingbuffer, SceMpegRingbufferCb,
    SceMpegAu, SceMpegAvcMode,
};

psp::module!("MpegBridge", 1, 0);

// Import all sceMpegVsh_library functions. These get resolved at module
// load time when sceMpegVsh_library is already registered (by mpeg_vsh370).
psp_extern! {
    #![name = "sceMpegVsh_library"]
    #![flags = 0x0009]
    #![version = (0x00, 0x00)]

    #[psp(0x682A619B)]
    pub fn bridge_sceMpegInit() -> i32;

    #[psp(0x874624D6)]
    pub fn bridge_sceMpegFinish();

    #[psp(0xC132E22F)]
    pub fn bridge_sceMpegQueryMemSize(unk: i32) -> i32;

    #[psp(0xD8C5F121)]
    pub fn bridge_sceMpegCreate(
        handle: SceMpeg, data: *mut c_void, size: i32,
        ringbuffer: *mut SceMpegRingbuffer, frame_width: i32,
        unk1: i32, unk2: i32,
    ) -> i32;

    #[psp(0x606A4649)]
    pub fn bridge_sceMpegDelete(handle: SceMpeg);

    #[psp(0x42560F23)]
    pub fn bridge_sceMpegRegistStream(handle: SceMpeg, stream_id: i32, unk: i32) -> SceMpegStream;

    #[psp(0x591A4AA2)]
    pub fn bridge_sceMpegUnRegistStream(handle: SceMpeg, stream: SceMpegStream);

    #[psp(0xA780CF7E)]
    pub fn bridge_sceMpegMallocAvcEsBuf(handle: SceMpeg) -> *mut c_void;

    #[psp(0xCEB870B1)]
    pub fn bridge_sceMpegFreeAvcEsBuf(handle: SceMpeg, buf: *mut c_void);

    #[psp(0x167AFD9E)]
    pub fn bridge_sceMpegInitAu(handle: SceMpeg, es_buffer: *mut c_void, au: *mut SceMpegAu) -> i32;

    #[psp(0xFE246728)]
    pub fn bridge_sceMpegGetAvcAu(handle: SceMpeg, stream: SceMpegStream, au: *mut SceMpegAu, unk: *mut i32) -> i32;

    #[psp(0xA11C7026)]
    pub fn bridge_sceMpegAvcDecodeMode(handle: SceMpeg, mode: *mut SceMpegAvcMode) -> i32;

    #[psp(0x0E3C2E9D)]
    pub fn bridge_sceMpegAvcDecode(handle: SceMpeg, au: *mut SceMpegAu, iframe_width: i32, buffer: *mut c_void, init: *mut i32) -> i32;

    #[psp(0x11F95CF1)]
    pub fn bridge_sceMpegGetAvcNalAu(handle: SceMpeg, nal: *mut c_void, au: *mut SceMpegAu) -> i32;

    #[psp(0xCF3547A2)]
    pub fn bridge_sceMpegAvcDecodeDetail2(handle: SceMpeg, detail: *mut *mut c_void) -> i32;

    #[psp(0xD7A29F46)]
    pub fn bridge_sceMpegRingbufferQueryMemSize(packets: i32) -> i32;

    #[psp(0x37295ED8)]
    pub fn bridge_sceMpegRingbufferConstruct(
        ringbuffer: *mut SceMpegRingbuffer, packets: i32, data: *mut c_void,
        size: i32, callback: SceMpegRingbufferCb, cb_param: *mut c_void,
    ) -> i32;

    #[psp(0x13407F13)]
    pub fn bridge_sceMpegRingbufferDestruct(ringbuffer: *mut SceMpegRingbuffer);

    #[psp(0xB5F6DC87)]
    pub fn bridge_sceMpegRingbufferAvailableSize(ringbuffer: *mut SceMpegRingbuffer) -> i32;

    #[psp(0xB240A59E)]
    pub fn bridge_sceMpegRingbufferPut(ringbuffer: *mut SceMpegRingbuffer, num_packets: i32, available: i32) -> i32;

    #[psp(0x21FF80E4)]
    pub fn bridge_sceMpegQueryStreamOffset(handle: SceMpeg, buffer: *mut c_void, offset: *mut i32) -> i32;

    #[psp(0x611E9E11)]
    pub fn bridge_sceMpegQueryStreamSize(buffer: *mut c_void, size: *mut i32) -> i32;
}

fn psp_main() {
    // Nothing to do — module_start just returns 0.
    // The import stubs above are the entire purpose of this PRX.
}
