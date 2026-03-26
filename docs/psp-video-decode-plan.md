# PSP H.264 Video Decode: sceMpeg + PSMF Implementation

## Status

**Phase 1 (complete):** Audio streaming works great. sceMpeg API fully initializes
on real PSP-3001 hardware (ARK-4 CFW, FW 6.61).

**Phase 2 (in progress):** PSMF container wrapping + sceMpegRingbufferPut. The
firmware's kernel-side MPEG-PS parser hangs on our generated MPEG-PS data.
Need a real PMF file as byte-level format reference.

## Architecture: What We Learned

### sceMpeg on Real Firmware vs Emulators

The user-mode `sceMpeg_library` (dumped from RAM, 32KB at 0x08C05C00) is a
**thin syscall wrapper**. All functions (RingbufferPut, GetAvcAu, AvcDecode)
are 5-15 instruction stubs that call into kernel space via `j 0x88XXXXX`.

**On PPSSPP/JPCSP:** sceMpegRingbufferPut directly calls the user callback,
runs `PostPutAction` to demux MPEG-PS, and returns synchronously.

**On real firmware:** sceMpegRingbufferPut enters the kernel, the kernel
invokes the callback, processes the data with its own MPEG-PS parser (runs
on the ME or a kernel thread), and returns. The kernel-side parser is what
hangs on our data.

### NID → Function Address Map (from mpeg.prx memory dump)

| NID | Address | Function |
|-----|---------|----------|
| 0xB240A59E | 0x08C0C080 | sceMpegRingbufferPut |
| 0x37295ED8 | 0x08C0BE88 | sceMpegRingbufferConstruct |
| 0xFE246728 | 0x08C0C0F0 | sceMpegGetAvcAu |
| 0x0E3C2E9D | 0x08C0BF10 | sceMpegAvcDecode |
| 0xD8C5F121 | 0x08C0C340 | sceMpegCreate |
| 0x682A619B | 0x08C0BD20 | sceMpegInit |
| 0xB5F6DC87 | 0x08C0C4B0 | sceMpegRingbufferAvailableSize |

### sceMpegRingbufferPut Disassembly

```mips
# Wrapper: checks init state, calls real implementation
sceMpegRingbufferPut:
  jal  check_func       # returns init phase counter
  slti $a0, $v0, 0x3f0  # if counter < 1008...
  bnez $a0, return_zero  # ...return 0 (not ready)
  jal  real_put          # else call real implementation
  jr   $ra

# Real implementation: enters kernel via syscall
real_put:
  jal  get_context       # → returns ptr to 0x08C0DE64
  jal  lock_sema         # sceKernelWaitSema
  jal  read_state_flag   # reads *(0x08C0DC40)
  andi $v1, $v0, 4       # check bit 2
  beqz $v1, skip         # if not set, return 0
  lw   $s0, 4($s1)       # else load result from context
  jal  unlock_sema       # sceKernelSignalSema
  jr   $ra               # return result
```

The actual data processing happens in kernel space, triggered by the
semaphore signal chain.

### MPEG-PS Format Requirements (from PPSSPP MpegDemux.cpp)

The kernel's MPEG-PS demuxer:

1. **Scans for start codes** byte-by-byte: `(code << 8) | read8()` until
   `(code & 0xFFFFFF00) == 0x00000100`

2. **Pack header validation** (`skipPackHeader`):
   - Byte 4: `(val & 0xC4) == 0x44` (MPEG-2 marker)
   - Byte 6: `(val & 0x04) == 0x04` (marker bit)
   - Byte 8: `(val & 0x04) == 0x04` (marker bit)
   - Byte 9: `(val & 0x01) == 0x01` (marker bit)
   - Mux rate: `(read24() & 3) == 3` (marker bits)
   - Stuffing: `read8() & 7` count, each byte must be 0xFF

3. **Video PES (0x1E0-0x1EF)**: reads `length = read16()`, then
   `skip(length)`. **PES length MUST be non-zero** — with length=0 the
   scanner enters the H.264 payload and finds NAL start codes (00 00 01)
   that look like MPEG-PS codes, causing an infinite loop.

4. **Padding stream (0x1BE)**: reads length, skips.

### What We Tried and Results

| Approach | Result |
|----------|--------|
| Raw Annex B in ringbuffer | Hangs (no PSMF structure) |
| MPEG-PS PES wrapping (no PSMF header) | Hangs |
| PSMF header + MPEG-PS with PES length=0 | Hangs (scanner enters H.264 data) |
| PSMF header + MPEG-PS with correct PES lengths | Hangs (kernel parser still rejects) |
| PSMF header + per-sector PES + padding stream | Hangs |
| Two-thread: feeder + GetAvcAu polling | Hangs (both threads block) |
| Callback returns 0 (no data) | Works! Audio plays, no freeze |

### What Works

- `sceMpegInit()` — succeeds (with `preinit_mpeg()` before audio thread)
- `sceMpegCreate()` — succeeds
- `sceMpegRegistStream()` — succeeds
- `sceMpegAvcDecodeMode(Psm8888)` — succeeds
- `sceMpegMallocAvcEsBuf()` — succeeds
- `sceMpegInitAu()` — succeeds
- `sceMpegBaseCscInit()` — succeeds
- `sceMpegQueryStreamOffset()` — validates our PSMF header (returns offset=2048)
- `sceMpegQueryStreamSize()` — returns 67108864 (our declared size)
- `sceMpegRingbufferPut()` with PSMF header packet — succeeds (returns 1)
- `sceMpegRingbufferPut()` callback — invoked, data copies correctly
- Audio playback via sceAudiocodec — fully working

### What Doesn't Work

- `sceMpegRingbufferPut()` with MPEG-PS AU data — kernel parser hangs
  after callback completes. All data is correctly copied to ringbuffer
  memory, D-cache flushed, callback returns correct count. The hang is
  in the kernel's post-callback processing (MPEG-PS demuxing).

## Next Step: Real PMF File as Format Reference

The converter-generated `oasis_demo.pmf` is actually an MP4 file (starts
with `ftypisom`). We need a **real PSMF file** (starts with `PSMF` magic)
to hex-dump the exact MPEG-PS format byte-by-byte.

### Options

1. **MagicISO / UMDGen**: Extract PMF files from PSP game ISOs (game intros)
2. **pmfenc / MPS2PMF**: Community tools that create real PSMF from raw streams
3. **Sony's official tools**: PSP Movie Creator (discontinued)
4. **Hex comparison**: Create minimal PSMF with known H.264 data, compare
   byte-by-byte against our generated format

### What to Compare

Once we have a real PMF, hex-dump offset 0x800+ and compare:
- Pack header byte layout (SCR encoding, mux rate)
- PES header flags, length encoding
- Whether system headers are present
- Padding between packs
- PES length values for video packets
- Any bytes between pack header and PES that we might be missing

## Files

| File | Status |
|------|--------|
| `crates/oasis-backend-psp/src/psmf.rs` | NEW: PSMF header + MPEG-PS wrapper |
| `crates/oasis-backend-psp/src/video.rs` | SceMpegDecoder replaces PspVideoDecoder |
| `crates/oasis-backend-psp/src/main.rs` | preinit_mpeg() before workers |
| `crates/oasis-backend-psp/src/lib.rs` | mod psmf added |
| `crates/oasis-prx-decrypt-psp/src/main.rs` | MPEG module memory dump (Circle button) |
| `scripts/ghidra_mpeg_ringbuffer.py` | Ghidra analysis script for mpeg.prx |

## Decrypted Firmware Dumps

| Module | Location | Method |
|--------|----------|--------|
| sceMpeg_library (user) | `dec_mpeg.bin` on PSP | RAM dump via sceKernelGetModuleIdList |
| mpeg.prx (encrypted) | flash0:/kd/mpeg.prx | Tag EA4E7B90, memlmd/pspdecrypt fail |
| mpegbase_260.prx (encrypted) | flash0:/kd/mpegbase_260.prx | Tag 238EE957, decrypt fails |

Note: The kernel-side MPEG implementation cannot be dumped — it runs in
protected kernel memory. The user-mode `sceMpeg_library` is just syscall
wrappers (confirmed by disassembly).

## References

- PPSSPP: `Core/HW/MpegDemux.cpp` — MPEG-PS parser (obtained, analyzed)
- JPCSP: `SceMpegRingbuffer.java` — struct layout with field offsets
- PSP-libpmfplayer: 4-thread model (Reader, Decoder, Video, Audio)
- psdevwiki: PMF format spec
- MultimediaWiki: PSMF format spec
- uOFW: mpeg.prx listed as orphan (NOT reverse-engineered)
