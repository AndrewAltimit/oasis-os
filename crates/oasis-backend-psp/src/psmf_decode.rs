//! PSMF ringbuffer H.264 decoder — alternative to the NAL direct path.
//!
//! Uses the standard sceMpeg ringbuffer API (sceMpegGetAvcAu + AvcDecode)
//! with data wrapped in MPEG-PS packs via the system's AvMpegBase module.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::psmf;
use crate::video::vlog_force;

// -----------------------------------------------------------------------
// Global callback state (needed because the kernel invokes the callback
// with a fixed parameter, and we can't pass &mut self directly)
// -----------------------------------------------------------------------

/// Pointer to the pending pack data for the callback to read.
static mut CB_PACK_DATA: *const u8 = core::ptr::null();
/// Number of pending packs available.
static CB_PACK_COUNT: AtomicU32 = AtomicU32::new(0);
/// Number of packs already consumed by the callback.
static CB_PACK_CONSUMED: AtomicU32 = AtomicU32::new(0);

/// Ringbuffer callback invoked by the kernel during sceMpegRingbufferPut.
///
/// Copies pending MPEG-PS packs into the ringbuffer data area.
unsafe extern "C" fn ringbuffer_callback(
    p_data: *mut c_void,
    num_packets: i32,
    _p_param: *mut c_void,
) -> i32 {
    let total = CB_PACK_COUNT.load(Ordering::Acquire);
    let consumed = CB_PACK_CONSUMED.load(Ordering::Relaxed);
    let available = total.saturating_sub(consumed);
    let to_copy = (num_packets as u32).min(available);

    if to_copy == 0 || CB_PACK_DATA.is_null() {
        return 0;
    }

    let src = CB_PACK_DATA.add(consumed as usize * psmf::PACK_SIZE);
    let dst = p_data as *mut u8;
    core::ptr::copy_nonoverlapping(src, dst, to_copy as usize * psmf::PACK_SIZE);

    // Flush D-cache so kernel/ME can read the copied data.
    psp::sys::sceKernelDcacheWritebackInvalidateRange(
        dst as *const c_void,
        to_copy * psmf::PACK_SIZE as u32,
    );

    CB_PACK_CONSUMED.store(consumed + to_copy, Ordering::Release);
    to_copy as i32
}

// -----------------------------------------------------------------------
// PSMF Decoder
// -----------------------------------------------------------------------

/// State for the PSMF ringbuffer decoder.
pub struct PsmfDecoder {
    mpeg_storage: *mut *mut c_void,
    _mpeg_data: Vec<u8>,
    ddr_block: psp::sys::SceUid,
    #[allow(dead_code)]
    ddr_aligned: u32,
    ringbuffer: Box<psp::sys::SceMpegRingbuffer>,
    _rb_data: Vec<u8>,
    au: psp::sys::SceMpegAu,
    output_buf: Vec<u8>,
    #[allow(dead_code)]
    es_buf: *mut c_void,
    stream: psp::sys::SceMpegStream,
    pub width: u32,
    pub height: u32,
    frame_width: u32,
    scr: u64,
    header_sent: bool,
    /// Flat buffer of pending packs (contiguous for callback).
    pack_buf: Vec<u8>,
    pic_num: i32,
}

impl PsmfDecoder {
    /// Create a new PSMF ringbuffer decoder for the given video dimensions.
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        // Standard mode (not mode 4/5 which are NAL-path specific).
        let mpeg_mode = 0;
        let frame_width = if width > 480 { 768u32 } else { 512 };

        vlog_force("[PSMF] creating decoder...");

        // Init MPEG subsystem.
        let ret = unsafe { psp::sys::sceMpegInit() };
        if ret < 0 && ret != 0x80618003u32 as i32
            && ret != 0x80618005u32 as i32
        {
            return Err(format!("sceMpegInit: {ret:#x}"));
        }

        // Query buffer size for mode 0.
        let mem_size = unsafe { psp::sys::sceMpegQueryMemSize(mpeg_mode) };
        if mem_size <= 0 {
            return Err(format!("QueryMemSize: {mem_size:#x}"));
        }
        vlog_force(&format!("[PSMF] memSize={mem_size}"));

        // Allocate mpeg data buffer (64-byte aligned).
        let mut mpeg_data = vec![0u8; mem_size as usize + 64];
        let mpeg_data_aligned = {
            let p = mpeg_data.as_mut_ptr();
            unsafe { p.add(p.align_offset(64)) }
        };

        // DDR top workspace (2MB, 4MB-aligned).
        let ddr_block = unsafe {
            psp::sys::sceKernelAllocPartitionMemory(
                psp::sys::SceSysMemPartitionId::SceKernelPrimaryUserPartition,
                b"PsmfDdr\0".as_ptr(),
                psp::sys::SceSysMemBlockTypes::Low,
                0x20_0000 + 0x40_0000,
                core::ptr::null_mut(),
            )
        };
        if ddr_block < psp::sys::SceUid(0) {
            return Err(format!("DDR alloc: {:#x}", ddr_block.0));
        }
        let ddr_raw =
            unsafe { psp::sys::sceKernelGetBlockHeadAddr(ddr_block) };
        let ddr_aligned = ((ddr_raw as u32) + 0x3F_FFFF) & !0x3F_FFFF;

        // Ringbuffer: 64 packets (128KB).
        let rb_packets = 64;
        let rb_size = unsafe {
            psp::sys::sceMpegRingbufferQueryMemSize(rb_packets)
        };
        let mut rb_data = vec![0u8; rb_size.max(1) as usize];
        let mut ringbuffer = Box::new(unsafe {
            core::mem::zeroed::<psp::sys::SceMpegRingbuffer>()
        });

        // Construct ringbuffer with our callback.
        let ret = unsafe {
            psp::sys::sceMpegRingbufferConstruct(
                &mut *ringbuffer,
                rb_packets,
                rb_data.as_mut_ptr() as *mut c_void,
                rb_size,
                Some(ringbuffer_callback),
                core::ptr::null_mut(), // param (unused, we use globals)
            )
        };
        if ret < 0 {
            unsafe {
                psp::sys::sceKernelFreePartitionMemory(ddr_block);
            }
            return Err(format!("RbConstruct: {ret:#x}"));
        }
        vlog_force("[PSMF] ringbuffer constructed");

        // Create sceMpeg instance.
        let mpeg_storage =
            Box::into_raw(Box::new(core::ptr::null_mut::<c_void>()));
        let mpeg: psp::sys::SceMpeg = unsafe {
            core::mem::transmute(mpeg_storage as *mut *mut c_void)
        };
        let ret = unsafe {
            psp::sys::sceMpegCreate(
                mpeg,
                mpeg_data_aligned as *mut c_void,
                mem_size,
                &mut *ringbuffer,
                frame_width as i32,
                mpeg_mode,
                ddr_aligned as i32,
            )
        };
        if ret < 0 {
            unsafe {
                let _ = Box::from_raw(mpeg_storage);
                psp::sys::sceKernelFreePartitionMemory(ddr_block);
            }
            return Err(format!("sceMpegCreate: {ret:#x}"));
        }
        vlog_force("[PSMF] mpeg created");

        // Register video stream (type 0 = video, channel 0).
        let stream =
            unsafe { psp::sys::sceMpegRegistStream(mpeg, 0, 0) };
        vlog_force("[PSMF] stream registered");

        // Allocate ES buffer.
        let es_buf = unsafe { psp::sys::sceMpegMallocAvcEsBuf(mpeg) };
        vlog_force(&format!("[PSMF] esBuf={:#x}", es_buf as u32));

        // Init AU using the ES buffer.
        let mut au = unsafe {
            let mut a =
                core::mem::MaybeUninit::<psp::sys::SceMpegAu>::uninit();
            core::ptr::write_bytes(
                a.as_mut_ptr() as *mut u8,
                0xFF,
                core::mem::size_of::<psp::sys::SceMpegAu>(),
            );
            a.assume_init()
        };
        let ret =
            unsafe { psp::sys::sceMpegInitAu(mpeg, es_buf, &mut au) };
        vlog_force(&format!("[PSMF] initAu={ret:#x}"));

        // Set decode mode to ABGR 8888.
        let mut mode = psp::sys::SceMpegAvcMode {
            unk0: -1,
            pixel_format: psp::sys::DisplayPixelFormat::Psm8888,
        };
        unsafe { psp::sys::sceMpegAvcDecodeMode(mpeg, &mut mode) };

        // Output buffer.
        let out_h = ((height + 15) / 16) * 16;
        let output_buf =
            vec![0u8; frame_width as usize * out_h as usize * 4];

        vlog_force("[PSMF] decoder created OK");

        Ok(Self {
            mpeg_storage,
            _mpeg_data: mpeg_data,
            ddr_block,
            ddr_aligned,
            ringbuffer,
            _rb_data: rb_data,
            au,
            output_buf,
            es_buf,
            stream,
            width,
            height,
            frame_width,
            scr: 0,
            header_sent: false,
            pack_buf: Vec::new(),
            pic_num: 0,
        })
    }

    fn mpeg(&self) -> psp::sys::SceMpeg {
        unsafe {
            core::mem::transmute(self.mpeg_storage as *mut *mut c_void)
        }
    }

    /// Send the PSMF header and first pack to the ringbuffer.
    fn send_header(&mut self) -> Result<(), String> {
        let hdr = psmf::generate_psmf_header(
            self.width as u16,
            self.height as u16,
            0x04000000,
        );

        // Validate header with sceMpegQueryStreamOffset.
        let mut offset: i32 = 0;
        let ret = unsafe {
            psp::sys::sceMpegQueryStreamOffset(
                self.mpeg(),
                &hdr as *const u8 as *mut c_void,
                &mut offset,
            )
        };
        vlog_force(&format!(
            "[PSMF] QueryStreamOffset={ret:#x} offset={offset:#x}"
        ));
        if ret < 0 {
            return Err(format!("QueryStreamOffset: {ret:#x}"));
        }

        // Feed header as the first ringbuffer packet.
        let first_pack = psmf::generate_first_pack(self.scr);
        self.scr += 27_000_000 / 30;

        // Pack both into the flat buffer.
        self.pack_buf.clear();
        self.pack_buf.extend_from_slice(&hdr);
        self.pack_buf.extend_from_slice(&first_pack);

        self.feed_packs(2)?;
        self.header_sent = true;
        vlog_force("[PSMF] header sent OK");
        Ok(())
    }

    /// Feed `count` packs from pack_buf to the ringbuffer.
    fn feed_packs(&mut self, count: u32) -> Result<(), String> {
        // Set global callback state.
        unsafe {
            CB_PACK_DATA = self.pack_buf.as_ptr();
        }
        CB_PACK_COUNT.store(count, Ordering::Release);
        CB_PACK_CONSUMED.store(0, Ordering::Release);

        // Flush the pack data from D-cache so it's visible to the callback.
        unsafe {
            psp::sys::sceKernelDcacheWritebackInvalidateRange(
                self.pack_buf.as_ptr() as *const c_void,
                self.pack_buf.len() as u32,
            );
        }

        let avail = unsafe {
            psp::sys::sceMpegRingbufferAvailableSize(
                &mut *self.ringbuffer,
            )
        };
        if avail <= 0 {
            return Err("ringbuffer full".into());
        }

        let to_put = (count as i32).min(avail);
        let ret = unsafe {
            psp::sys::sceMpegRingbufferPut(
                &mut *self.ringbuffer,
                to_put,
                avail,
            )
        };
        vlog_force(&format!(
            "[PSMF] put {to_put} packs, ret={ret}"
        ));
        if ret < 0 {
            return Err(format!("RingbufferPut: {ret:#x}"));
        }
        Ok(())
    }

    /// Feed one H.264 access unit (Annex B) and attempt decode.
    ///
    /// Returns `true` if a frame was decoded, `false` otherwise.
    pub fn feed_and_decode(&mut self, annex_b: &[u8], pts_secs: f64)
        -> bool
    {
        let pts_90khz = (pts_secs * 90000.0) as u64;

        // Send PSMF header on first call.
        if !self.header_sent {
            if let Err(e) = self.send_header() {
                vlog_force(&format!("[PSMF] header error: {e}"));
                return false;
            }
        }

        // Wrap the AU in MPEG-PS packs.
        let mut packs: Vec<[u8; psmf::PACK_SIZE]> = Vec::new();
        let pack_count = psmf::wrap_video_au(
            annex_b,
            pts_90khz,
            &mut self.scr,
            &mut packs,
        );

        // Flatten into contiguous buffer for callback.
        self.pack_buf.clear();
        for pack in &packs {
            self.pack_buf.extend_from_slice(pack);
        }

        if let Err(e) = self.feed_packs(pack_count as u32) {
            vlog_force(&format!("[PSMF] feed error: {e}"));
            return false;
        }

        // Try to get a decoded frame.
        self.try_decode()
    }

    /// Try to extract and decode one video AU from the ringbuffer.
    fn try_decode(&mut self) -> bool {
        let mpeg = self.mpeg();

        // Get next video AU.
        let ret = unsafe {
            psp::sys::sceMpegGetAvcAu(
                mpeg,
                self.stream,
                &mut self.au,
                core::ptr::null_mut(),
            )
        };
        if ret < 0 {
            return false; // No AU available yet.
        }

        // Decode.
        let mut output_ptr =
            self.output_buf.as_mut_ptr() as *mut c_void;
        let buf_arg =
            &mut output_ptr as *mut *mut c_void as *mut c_void;
        let ret = unsafe {
            psp::sys::sceMpegAvcDecode(
                mpeg,
                &mut self.au,
                self.frame_width as i32,
                buf_arg,
                &mut self.pic_num,
            )
        };
        if ret < 0 || self.pic_num <= 0 {
            return false;
        }

        true
    }

    /// Get a reference to the decoded frame's pixel data.
    ///
    /// The output buffer contains ABGR 8888 pixels at `frame_width` stride.
    /// Call after `feed_and_decode` returns `true`.
    pub fn pixels(&self) -> &[u8] {
        let size = (self.width * self.height * 4) as usize;
        &self.output_buf[..size.min(self.output_buf.len())]
    }
}

impl Drop for PsmfDecoder {
    fn drop(&mut self) {
        let mpeg = self.mpeg();
        unsafe {
            if !self.es_buf.is_null() {
                psp::sys::sceMpegFreeAvcEsBuf(mpeg, self.es_buf);
            }
            psp::sys::sceMpegDelete(mpeg);
            let _ = Box::from_raw(self.mpeg_storage);
            if self.ddr_block >= psp::sys::SceUid(0) {
                psp::sys::sceKernelFreePartitionMemory(self.ddr_block);
            }
            psp::sys::sceMpegFinish();
        }
    }
}
