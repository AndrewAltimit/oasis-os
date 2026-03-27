//! PSMF ringbuffer H.264 decoder — alternative to the NAL direct path.
//!
//! Uses the standard sceMpeg ringbuffer API (sceMpegGetAvcAu + AvcDecode)
//! with data wrapped in MPEG-PS packs. This path uses the system's
//! AvMpegBase module (not mpeg_vsh370.prx) which may not have the mode 5
//! deadlock after ~90 frames.
//!
//! ## Data flow
//!
//! 1. H.264 NAL units (Annex B) → PSMF wrapper → 2048-byte MPEG-PS packs
//! 2. Packs fed via sceMpegRingbufferPut callback
//! 3. Kernel demuxes MPEG-PS → extracts H.264 AUs
//! 4. sceMpegGetAvcAu → sceMpegAvcDecode → decoded frame

use core::ffi::c_void;

use crate::psmf;
use crate::video::vlog_force;

/// State for the PSMF ringbuffer decoder.
pub struct PsmfDecoder {
    /// sceMpeg instance handle.
    mpeg_storage: *mut *mut c_void,
    /// Heap-allocated mpeg data buffer (64-byte aligned).
    _mpeg_data: Vec<u8>,
    /// DDR top block for ME workspace.
    ddr_block: psp::sys::SceUid,
    /// 4MB-aligned DDR workspace address.
    ddr_aligned: u32,
    /// Ringbuffer struct.
    ringbuffer: Box<psp::sys::SceMpegRingbuffer>,
    /// Ringbuffer data memory.
    _rb_data: Vec<u8>,
    /// AU struct for video.
    au: psp::sys::SceMpegAu,
    /// Output pixel buffer.
    output_buf: Vec<u8>,
    /// Video ES buffer handle.
    es_buf: *mut c_void,
    /// Video width/height.
    pub width: u32,
    pub height: u32,
    /// Frame width (stride for output).
    frame_width: u32,
    /// SCR counter for pack timestamps.
    scr: u64,
    /// Whether PSMF header + first pack have been sent.
    header_sent: bool,
    /// Pending packs ready to be fed via ringbuffer callback.
    pending_packs: Vec<[u8; psmf::PACK_SIZE]>,
    /// Index into pending_packs for callback consumption.
    pending_idx: usize,
    /// PTS counter (90kHz).
    pts_counter: u64,
}

/// Ringbuffer callback — copies pending packs into the ringbuffer.
///
/// # Safety
/// Called by the kernel during sceMpegRingbufferPut. The `pParam`
/// is a pointer to PsmfDecoder.
unsafe extern "C" fn ringbuffer_callback(
    p_data: *mut c_void,
    num_packets: i32,
    p_param: *mut c_void,
) -> i32 {
    let decoder = &mut *(p_param as *mut PsmfDecoder);
    let dst = p_data as *mut u8;
    let mut written = 0i32;

    for i in 0..num_packets {
        if decoder.pending_idx >= decoder.pending_packs.len() {
            break;
        }
        let pack = &decoder.pending_packs[decoder.pending_idx];
        let offset = i as usize * psmf::PACK_SIZE;
        core::ptr::copy_nonoverlapping(
            pack.as_ptr(),
            dst.add(offset),
            psmf::PACK_SIZE,
        );
        decoder.pending_idx += 1;
        written += 1;
    }

    // Flush D-cache so ME/kernel can read the data.
    if written > 0 {
        psp::sys::sceKernelDcacheWritebackInvalidateRange(
            dst as *const c_void,
            (written as u32) * psmf::PACK_SIZE as u32,
        );
    }

    written
}

impl PsmfDecoder {
    /// Create a new PSMF ringbuffer decoder.
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let mpeg_mode = 0; // Standard mode (not 4 or 5)
        let frame_width = if width > 480 { 768u32 } else { 512 };

        // Init MPEG subsystem.
        let ret = unsafe { psp::sys::sceMpegInit() };
        if ret < 0 && ret != 0x80618003u32 as i32 && ret != 0x80618005u32 as i32 {
            return Err(format!("sceMpegInit: {ret:#x}"));
        }

        // Query buffer size.
        let mem_size = unsafe { psp::sys::sceMpegQueryMemSize(mpeg_mode) };
        if mem_size <= 0 {
            return Err(format!("sceMpegQueryMemSize: {mem_size:#x}"));
        }

        // Allocate mpeg data buffer.
        let mut mpeg_data = vec![0u8; mem_size as usize + 64];
        let mpeg_data_aligned = {
            let p = mpeg_data.as_mut_ptr();
            unsafe { p.add(p.align_offset(64)) }
        };

        // DDR top workspace.
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
        let ddr_raw = unsafe { psp::sys::sceKernelGetBlockHeadAddr(ddr_block) };
        let ddr_aligned = ((ddr_raw as u32) + 0x3F_FFFF) & !0x3F_FFFF;

        // Ringbuffer: 32 packets of 2048 bytes = 64KB.
        let rb_packets = 32;
        let rb_size =
            unsafe { psp::sys::sceMpegRingbufferQueryMemSize(rb_packets) };
        let mut rb_data = vec![0u8; if rb_size > 0 { rb_size as usize } else { 65536 }];

        let mut ringbuffer = Box::new(
            unsafe { core::mem::zeroed::<psp::sys::SceMpegRingbuffer>() }
        );

        // We'll set the callback after we have the decoder pointer.
        // For now, construct with a placeholder.
        if rb_size > 0 {
            let ret = unsafe {
                psp::sys::sceMpegRingbufferConstruct(
                    &mut *ringbuffer,
                    rb_packets,
                    rb_data.as_mut_ptr() as *mut c_void,
                    rb_size,
                    None, // callback set later
                    core::ptr::null_mut(),
                )
            };
            if ret < 0 {
                unsafe { psp::sys::sceKernelFreePartitionMemory(ddr_block) };
                return Err(format!("RingbufferConstruct: {ret:#x}"));
            }
        }

        // Create sceMpeg instance (mode 0 for ringbuffer path).
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

        // Register video stream.
        let _stream = unsafe { psp::sys::sceMpegRegistStream(mpeg, 0, 0) };

        // Init AU.
        let mut au = unsafe {
            let mut a = core::mem::MaybeUninit::<psp::sys::SceMpegAu>::uninit();
            core::ptr::write_bytes(
                a.as_mut_ptr() as *mut u8,
                0xFF,
                core::mem::size_of::<psp::sys::SceMpegAu>(),
            );
            a.assume_init()
        };

        // Allocate ES buffer for video AU data.
        let es_buf = unsafe { psp::sys::sceMpegMallocAvcEsBuf(mpeg) };

        let au_buffer = unsafe { psp::sys::sceMpegMallocAvcEsBuf(mpeg) };
        let ret = unsafe {
            psp::sys::sceMpegInitAu(mpeg, au_buffer, &mut au)
        };
        if ret < 0 {
            vlog_force("[PSMF] sceMpegInitAu failed");
        }

        // Set decode mode.
        let mut mode = psp::sys::SceMpegAvcMode {
            unk0: -1,
            pixel_format: psp::sys::DisplayPixelFormat::Psm8888,
        };
        unsafe { psp::sys::sceMpegAvcDecodeMode(mpeg, &mut mode) };

        // Output buffer.
        let out_h = ((height + 15) / 16) * 16;
        let output_buf = vec![0u8; frame_width as usize * out_h as usize * 4];

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
            width,
            height,
            frame_width,
            scr: 0,
            header_sent: false,
            pending_packs: Vec::new(),
            pending_idx: 0,
            pts_counter: 0,
        })
    }

    /// Feed one H.264 access unit (Annex B format) and attempt decode.
    ///
    /// Returns `Some((width, height, &[u8]))` with RGBA pixel data on
    /// success, `None` if no frame produced yet.
    pub fn feed_and_decode(
        &mut self,
        annex_b: &[u8],
        pts_secs: f64,
    ) -> Option<(u32, u32)> {
        let pts_90khz = (pts_secs * 90000.0) as u64;

        // Send PSMF header + first pack if not done yet.
        if !self.header_sent {
            let hdr = psmf::generate_psmf_header(
                self.width as u16,
                self.height as u16,
                0x04000000, // placeholder data size
            );
            // Feed header as one pack.
            self.pending_packs.clear();
            self.pending_packs.push(hdr);

            // First MPEG-PS pack with system header.
            let first_pack = psmf::generate_first_pack(self.scr);
            self.scr += 27_000_000 / 30;
            self.pending_packs.push(first_pack);

            self.feed_pending_to_ringbuffer();
            self.header_sent = true;
        }

        // Wrap the H.264 AU in MPEG-PS packs.
        self.pending_packs.clear();
        self.pending_idx = 0;
        let _pack_count = psmf::wrap_video_au(
            annex_b,
            pts_90khz,
            &mut self.scr,
            &mut self.pending_packs,
        );

        // Feed packs to ringbuffer.
        self.feed_pending_to_ringbuffer();

        // Try to get a decoded AU.
        self.try_decode()
    }

    /// Feed pending packs via sceMpegRingbufferPut.
    fn feed_pending_to_ringbuffer(&mut self) {
        if self.pending_packs.is_empty() {
            return;
        }

        self.pending_idx = 0;
        let num = self.pending_packs.len() as i32;

        // Set callback to our function (passing self as param).
        // Note: the ringbuffer callback is set during construct.
        // We need to update it to point to our data.
        // For now, manually copy data to ringbuffer memory.

        // Actually, sceMpegRingbufferPut invokes the callback.
        // We need to set up the callback properly.
        // The callback address and param are in the ringbuffer struct.

        // Direct approach: copy packs to the ringbuffer memory ourselves
        // and call sceMpegRingbufferPut with the count.
        unsafe {
            // Check available space.
            let avail = psp::sys::sceMpegRingbufferAvailableSize(
                &mut *self.ringbuffer,
            );
            if avail <= 0 {
                return;
            }

            let to_put = num.min(avail);

            // The ringbuffer callback approach requires the callback to
            // copy data. Since we constructed without a callback, we need
            // to manually write to the ringbuffer memory.
            // TODO: Proper callback integration.

            let ret = psp::sys::sceMpegRingbufferPut(
                &mut *self.ringbuffer,
                to_put,
                avail,
            );
            if ret < 0 {
                vlog_force("[PSMF] RingbufferPut failed");
            }
        }
    }

    /// Try to extract and decode one video AU from the ringbuffer.
    fn try_decode(&mut self) -> Option<(u32, u32)> {
        let mpeg: psp::sys::SceMpeg = unsafe {
            core::mem::transmute(self.mpeg_storage as *mut *mut c_void)
        };

        // Get next video AU.
        // SAFETY: SceMpegStream is a pointer type; 0 for the default stream.
        let stream: psp::sys::SceMpegStream = unsafe { core::mem::zeroed() };
        let ret = unsafe {
            psp::sys::sceMpegGetAvcAu(
                mpeg,
                stream,
                &mut self.au,
                core::ptr::null_mut(),
            )
        };
        if ret < 0 {
            // No AU available yet (normal during buffering).
            return None;
        }

        // Decode.
        let mut output_ptr = self.output_buf.as_mut_ptr() as *mut c_void;
        let buf_arg = &mut output_ptr as *mut *mut c_void as *mut c_void;
        let mut pic_num = 0i32;
        let ret = unsafe {
            psp::sys::sceMpegAvcDecode(
                mpeg,
                &mut self.au,
                self.frame_width as i32,
                buf_arg,
                &mut pic_num,
            )
        };
        if ret < 0 || pic_num <= 0 {
            return None;
        }

        Some((self.width, self.height))
    }
}

impl Drop for PsmfDecoder {
    fn drop(&mut self) {
        let mpeg: psp::sys::SceMpeg = unsafe {
            core::mem::transmute(self.mpeg_storage as *mut *mut c_void)
        };
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
