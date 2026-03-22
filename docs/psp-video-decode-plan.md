# PSP H.264 Video Decode: Next Phase Plan

## Status

Audio streaming works great (Phase A fixes shipped). Video decode is blocked
because `avcodec.prx` on PSP-3001 (ARK-4 CFW, FW 6.61) has empty ME stubs.
The only viable path is the `sceMpeg` API with PSMF container format.

## Goal

Wrap raw Annex B H.264 NALs from TV Guide streams in PSMF (PSP Movie Format)
so `sceMpegRingbufferPut` accepts the data and `sceMpegAvcDecode` produces
decoded frames.

## Background: Why PSMF?

`sceMpegRingbufferPut` internally parses data looking for PSMF structure.
Without PSMF headers, the parser enters an infinite loop (confirmed by testing
raw Annex B, PES-wrapped, and MPEG-PS pack-wrapped data — all hang).

PSP games use `.pmf` (PSMF) files for video. The format is well-documented
in the PSP homebrew community (JPCSP, PPSSPP source code, psdevwiki).

## PSMF Format Overview

### Header (0x800 bytes, sent once at stream start)

```
Offset  Size  Field
0x000   4     Magic: "PSMF"
0x004   4     Version: "0015" (FW 1.5+) or "0012"
0x008   4     Header offset (big-endian, typically 0x00000800)
0x00C   4     Stream data size (big-endian)
0x010   4     reserved
0x014   4     reserved
0x050   2     Number of streams (big-endian)
0x052   1     Stream 0 type: 0x00 = AVC video
0x053   1     Stream 0 channel: 0x00
0x054   1     Stream 0 specific[0]: AVC profile (0x42=Baseline, 0x4D=Main)
0x055   1     Stream 0 specific[1]: AVC level
0x056-7 2     Video width (big-endian)
0x058-9 2     Video height (big-endian)
...     ...   EPMap and other metadata
0x800   ...   Stream data begins
```

### Stream Data (MPEG-PS packs with PES packets)

Each video AU is wrapped as:
```
00 00 01 BA  [SCR fields]  [mux rate]  [stuffing]   — Pack header (14 bytes)
00 00 01 BB  [header len]  [system header fields]    — System header (first pack only)
00 00 01 E0  [PES len]  [flags]  [PTS/DTS]  [AU]    — PES video packet
```

The key differences from generic MPEG-PS:
1. PSMF header (0x800 bytes) must be present at the start
2. Stream IDs must match those declared in the PSMF header
3. PTS/DTS must be present in PES headers
4. SCR in pack headers should be monotonically increasing

## Implementation Plan

### Step 1: Research PSMF Format

- Read PPSSPP's `Core/HLE/scePsmf.cpp` for the header parser
- Read JPCSP's `jpcsp/HLE/modules/scePsmf.java` for validation rules
- Find a sample `.pmf` file and hexdump the header for reference
- Check if `sceMpegRingbufferPut` validates the PSMF header at the
  start of the ringbuffer data, or if it's only validated by `scePsmf`

### Step 2: Implement PSMF Header Generator

Create `psmf.rs` in `crates/oasis-backend-psp/src/`:
- `fn generate_psmf_header(width, height, fps, profile, level) -> [u8; 0x800]`
- Takes H.264 SPS parameters (extracted from first keyframe's SPS NAL)
- Produces a minimal valid PSMF header

### Step 3: Implement PSMF Stream Wrapper

Extend `psmf.rs`:
- `fn wrap_psmf_pes(au_data, pts_90khz, is_first) -> Vec<u8>`
- Wraps each H.264 AU in pack header + PES with proper timestamps
- First packet includes system header
- Monotonically increasing SCR

### Step 4: Integrate with sceMpeg Decoder

Replace the `PspVideoDecoder` (sceVideocodec-based) with `SceMpegDecoder`:
- On first keyframe: parse SPS to get width/height/profile
- Generate PSMF header and write to ringbuffer start
- For each subsequent AU: wrap in PSMF PES and feed via ringbuffer
- Use `sceMpegGetAvcAu` + `sceMpegAvcDecode` to get decoded frames
- Convert YCbCr output to RGBA via scalar fixed-point (or VFPU once
  the LLVM regression is resolved)

### Step 5: Handle sceMpeg Lifecycle

The sceMpeg decoder is stateful:
- `sceMpegCreate` with PSMF header info (width, height from header)
- `sceMpegRegistStream` for video stream
- Ringbuffer callback feeds PSMF-wrapped data
- `sceMpegDelete` on channel change

Channel switching requires tearing down and recreating the MPEG context
since PSMF headers may differ between streams.

### Step 6: Test and Iterate

- Build EBOOT, deploy to PSP-3001
- Tune TV Guide channel
- Check `eboot.log` for `sceMpegAvcDecode` results
- If decode succeeds: verify YCbCr → RGBA conversion
- If decode fails: check PSMF header validity against PPSSPP/JPCSP

## Key Risk: PSMF vs scePsmf

There are TWO APIs:
- `scePsmf` — PSMF header parsing (returns stream info)
- `sceMpeg` — actual decode

`sceMpegRingbufferPut` may or may not require PSMF headers in the
ringbuffer data. It might only need valid MPEG-PS packs (which we
already tried and hung). The hang could be from something else entirely:

**Alternative hypothesis:** `sceMpegRingbufferPut` hangs because it
waits for the ME to consume data, but the ME isn't running because
no decode was started. The proper flow might be:

1. Fill ringbuffer with SOME data via `sceMpegRingbufferPut`
2. Call `sceMpegGetAvcAu` which starts the ME consumer
3. ME consumes data, freeing ringbuffer space
4. Repeat

If `sceMpegRingbufferPut` blocks because the ringbuffer is "full"
(from the ME's perspective), we need a non-blocking approach or a
separate thread for feeding vs decoding.

## Files to Create/Modify

| File | Changes |
|------|---------|
| `crates/oasis-backend-psp/src/psmf.rs` | NEW: PSMF header generator + PES wrapper |
| `crates/oasis-backend-psp/src/video.rs` | Replace PspVideoDecoder with SceMpegDecoder using PSMF |
| `crates/oasis-backend-psp/src/lib.rs` | Add `mod psmf;` |

## References

- PPSSPP: `Core/HLE/scePsmf.cpp`, `Core/HLE/sceMpeg.cpp`
- JPCSP: `jpcsp/HLE/modules/scePsmf.java`, `sceMpeg.java`
- psdevwiki: https://www.psdevwiki.com/psp/PSMF
- PSP Movie Format spec (community-documented)
- Our Ghidra scripts: `scripts/ghidra_avcodec_*.py`
- Our decrypted PRX: `/home/mikunpc/Downloads/avcodec_decrypted`
