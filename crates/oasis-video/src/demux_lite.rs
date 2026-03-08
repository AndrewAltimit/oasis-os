//! Lightweight MP4 demuxer for `no_std`-friendly environments (e.g. PSP).
//!
//! Parses ISO BMFF boxes manually using only `std::io::{Read, Seek}`,
//! avoiding symphonia's `lazy_static`/`sync::Once` which panics in PPSSPP HLE.
//!
//! Supports:
//! - H.264 video (avc1) and AAC audio (mp4a) tracks
//! - AVCC→Annex B NAL conversion
//! - Sample-accurate seeking via stss (sync sample table)
//! - stts/ctts-based PTS calculation

use std::io::{Read, Seek, SeekFrom};

/// Errors from the lightweight demuxer.
#[derive(Debug)]
pub enum LiteError {
    Io(std::io::Error),
    Parse(String),
    NoTrack(String),
}

impl From<std::io::Error> for LiteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for LiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Parse(s) => write!(f, "parse: {s}"),
            Self::NoTrack(s) => write!(f, "no track: {s}"),
        }
    }
}

impl std::error::Error for LiteError {}

/// Track kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// Parsed box header.
#[derive(Debug, Clone)]
struct BoxHeader {
    box_type: [u8; 4],
    /// Size of the content (excluding header).
    content_size: u64,
    /// Absolute offset where content starts.
    content_offset: u64,
}

/// AVCC configuration extracted from the avcC box.
#[derive(Debug, Clone)]
pub struct AvccConfig {
    pub nal_length_size: usize,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

/// AAC configuration extracted from the esds box.
#[derive(Debug, Clone)]
pub struct AacConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// Raw AudioSpecificConfig bytes.
    pub config_data: Vec<u8>,
}

/// A time-to-sample (stts) entry.
#[derive(Debug, Clone)]
struct SttsEntry {
    sample_count: u32,
    sample_delta: u32,
}

/// A composition offset (ctts) entry.
#[derive(Debug, Clone)]
struct CttsEntry {
    sample_count: u32,
    sample_offset: i32,
}

/// A sample-to-chunk (stsc) entry.
#[derive(Debug, Clone)]
struct StscEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
}

/// Per-track sample table.
#[derive(Debug, Clone, Default)]
struct SampleTable {
    stsc: Vec<StscEntry>,
    stsz: Vec<u32>,
    /// Chunk offsets (stco or co64).
    stco: Vec<u64>,
    /// Sync sample indices (1-based).
    stss: Vec<u32>,
    stts: Vec<SttsEntry>,
    ctts: Vec<CttsEntry>,
    timescale: u32,
}

/// Information about a discovered track.
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub kind: TrackKind,
    table: SampleTable,
    pub avcc: Option<AvccConfig>,
    pub aac_config: Option<AacConfig>,
}

impl TrackInfo {
    /// Number of samples in this track.
    pub fn sample_count(&self) -> usize {
        self.table.stsz.len()
    }

    /// Get the absolute file offset and size (bytes) for a sample.
    pub fn sample_offset_size(&self, idx: usize) -> Option<(u64, u32)> {
        sample_file_offset(&self.table, idx)
    }

    /// Presentation timestamp in seconds for a sample.
    pub fn sample_timestamp(&self, idx: usize) -> f64 {
        let pts = sample_pts(&self.table, idx);
        let ts = self.table.timescale;
        if ts > 0 { pts as f64 / ts as f64 } else { 0.0 }
    }

    /// Whether a sample is a sync (key) frame.
    pub fn sample_is_keyframe(&self, idx: usize) -> bool {
        is_keyframe(&self.table, idx)
    }
}

/// A raw sample read from the file.
#[derive(Debug)]
pub struct RawSample {
    pub data: Vec<u8>,
    pub timestamp_secs: f64,
    pub is_keyframe: bool,
    pub kind: TrackKind,
}

/// Lightweight MP4 parser.
pub struct Mp4Lite<R: Read + Seek> {
    reader: R,
    video_track: Option<TrackInfo>,
    audio_track: Option<TrackInfo>,
    video_sample_index: usize,
    audio_sample_index: usize,
}

/// Annex B start code (4-byte variant).
const ANNEX_B_START: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

impl<R: Read + Seek> Mp4Lite<R> {
    /// Open and parse an MP4 container.
    pub fn open(mut reader: R) -> Result<Self, LiteError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let mut video_track = None;
        let mut audio_track = None;

        // Parse top-level boxes.
        parse_boxes(
            &mut reader,
            0,
            file_size,
            &mut video_track,
            &mut audio_track,
        )?;

        Ok(Self {
            reader,
            video_track,
            audio_track,
            video_sample_index: 0,
            audio_sample_index: 0,
        })
    }

    /// Read the next video sample (with AVCC→Annex B conversion).
    pub fn next_video_sample(&mut self) -> Result<Option<RawSample>, LiteError> {
        let track = match &self.video_track {
            Some(t) => t,
            None => return Err(LiteError::NoTrack("no video track".into())),
        };
        let idx = self.video_sample_index;
        if idx >= track.table.stsz.len() {
            return Ok(None);
        }
        let sample = read_sample(&mut self.reader, track, idx)?;
        self.video_sample_index += 1;
        Ok(Some(sample))
    }

    /// Read the next audio sample.
    pub fn next_audio_sample(&mut self) -> Result<Option<RawSample>, LiteError> {
        let track = match &self.audio_track {
            Some(t) => t,
            None => return Err(LiteError::NoTrack("no audio track".into())),
        };
        let idx = self.audio_sample_index;
        if idx >= track.table.stsz.len() {
            return Ok(None);
        }
        let sample = read_sample(&mut self.reader, track, idx)?;
        self.audio_sample_index += 1;
        Ok(Some(sample))
    }

    /// Seek to the nearest keyframe at or before `secs`.
    pub fn seek(&mut self, secs: f64) -> Result<(), LiteError> {
        // Seek video track.
        if let Some(track) = &self.video_track {
            let target_ts = (secs * track.table.timescale as f64) as u64;
            let sample_idx = find_keyframe_before(track, target_ts);
            self.video_sample_index = sample_idx;
        }
        // Seek audio track to the nearest sample.
        if let Some(track) = &self.audio_track {
            let target_ts = (secs * track.table.timescale as f64) as u64;
            let sample_idx = find_sample_at(track, target_ts);
            self.audio_sample_index = sample_idx;
        }
        Ok(())
    }

    /// Video track info, if present.
    pub fn video_track_info(&self) -> Option<&TrackInfo> {
        self.video_track.as_ref()
    }

    /// Audio track info, if present.
    pub fn audio_track_info(&self) -> Option<&TrackInfo> {
        self.audio_track.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Box parsing
// ---------------------------------------------------------------------------

/// Read a box header at the current position.
fn read_box_header<R: Read + Seek>(r: &mut R) -> Result<Option<BoxHeader>, LiteError> {
    let offset = r.stream_position()?;
    let mut buf = [0u8; 8];
    match r.read_exact(&mut buf) {
        Ok(()) => {},
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }

    let size = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64;
    let box_type = [buf[4], buf[5], buf[6], buf[7]];

    let (content_offset, content_size) = if size == 1 {
        // Extended size.
        let mut ext = [0u8; 8];
        r.read_exact(&mut ext)?;
        let ext_size = u64::from_be_bytes(ext);
        if ext_size < 16 {
            return Err(LiteError::Parse("extended box size < 16".into()));
        }
        (offset + 16, ext_size - 16)
    } else if size == 0 {
        // Box extends to end of file.
        let end = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(offset + 8))?;
        (offset + 8, end - offset - 8)
    } else {
        if size < 8 {
            return Err(LiteError::Parse("box size < 8".into()));
        }
        (offset + 8, size - 8)
    };

    Ok(Some(BoxHeader {
        box_type,
        content_size,
        content_offset,
    }))
}

/// Container box types that should be recursed into.
const CONTAINER_BOXES: &[[u8; 4]] = &[
    *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"dinf", *b"edts",
];

/// Parse boxes within a range, recursing into container boxes.
fn parse_boxes<R: Read + Seek>(
    r: &mut R,
    start: u64,
    end: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(start))?;
    let mut pos = start;

    while pos < end {
        r.seek(SeekFrom::Start(pos))?;
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };

        let box_end = header.content_offset + header.content_size;
        if box_end > end {
            break;
        }

        let bt = header.box_type;

        if CONTAINER_BOXES.contains(&bt) {
            parse_boxes(r, header.content_offset, box_end, video, audio)?;
        } else if bt == *b"stsd" {
            // Skip 8-byte fullbox header (version + flags + entry_count).
            parse_stsd(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stts" {
            parse_stts(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"ctts" {
            parse_ctts(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stsc" {
            parse_stsc(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stsz" {
            parse_stsz(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stco" {
            parse_stco(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"co64" {
            parse_co64(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"stss" {
            parse_stss(r, header.content_offset, header.content_size, video, audio)?;
        } else if bt == *b"mdhd" {
            parse_mdhd(r, header.content_offset, header.content_size, video, audio)?;
        }

        pos = box_end;
    }

    Ok(())
}

/// Get a mutable reference to the "current" (last-added) track being built.
///
/// The most recently created track with an empty sample table is the one
/// currently being parsed. If both have data, prefer audio (added last).
fn current_track_mut<'a>(
    video: &'a mut Option<TrackInfo>,
    audio: &'a mut Option<TrackInfo>,
) -> Option<&'a mut TrackInfo> {
    let audio_empty = audio.as_ref().is_some_and(|a| {
        a.table.stsz.is_empty()
            && a.table.stts.is_empty()
            && a.table.stco.is_empty()
            && a.table.stsc.is_empty()
    });
    if audio_empty {
        return audio.as_mut();
    }

    let video_empty = video.as_ref().is_some_and(|v| {
        v.table.stsz.is_empty()
            && v.table.stts.is_empty()
            && v.table.stco.is_empty()
            && v.table.stsc.is_empty()
    });
    if video_empty {
        return video.as_mut();
    }

    // Both have data — return audio (most recently created), else video.
    if audio.is_some() {
        audio.as_mut()
    } else {
        video.as_mut()
    }
}

// ---------------------------------------------------------------------------
// Individual box parsers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn read_u16_be<R: Read>(r: &mut R) -> Result<u16, LiteError> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_be_bytes(buf))
}

fn read_u32_be<R: Read>(r: &mut R) -> Result<u32, LiteError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

fn read_u64_be<R: Read>(r: &mut R) -> Result<u64, LiteError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_be_bytes(buf))
}

/// Parse stsd (sample description) to discover track codecs.
fn parse_stsd<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let entry_count = read_u32_be(r)?;

    let entries_end = offset + size;

    for _ in 0..entry_count {
        let pos = r.stream_position()?;
        if pos >= entries_end {
            break;
        }
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };

        let codec = header.box_type;
        if codec == *b"avc1" || codec == *b"avc3" {
            // H.264 video track.
            let avcc = parse_avc_sample_entry(r, header.content_offset, header.content_size)?;
            if video.is_none() {
                *video = Some(TrackInfo {
                    kind: TrackKind::Video,
                    table: SampleTable::default(),
                    avcc,
                    aac_config: None,
                });
            }
        } else if codec == *b"mp4a" {
            // AAC audio track.
            let aac = parse_mp4a_sample_entry(r, header.content_offset, header.content_size)?;
            if audio.is_none() {
                *audio = Some(TrackInfo {
                    kind: TrackKind::Audio,
                    table: SampleTable::default(),
                    avcc: None,
                    aac_config: aac,
                });
            }
        }

        let box_end = header.content_offset + header.content_size;
        r.seek(SeekFrom::Start(box_end))?;
    }

    Ok(())
}

/// Parse avc1/avc3 sample entry to extract the avcC box.
fn parse_avc_sample_entry<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<Option<AvccConfig>, LiteError> {
    // avc1 sample entry: 6 reserved + 2 data_ref_idx + 16 pre-defined + 2 w + 2 h + ...
    // The avcC sub-box is somewhere inside. Scan for it.
    let end = offset + size;
    // Skip the fixed 78-byte visual sample entry header to find sub-boxes.
    let sub_start = offset + 78;
    if sub_start >= end {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(sub_start))?;

    let mut pos = sub_start;
    while pos + 8 <= end {
        r.seek(SeekFrom::Start(pos))?;
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };
        if header.box_type == *b"avcC" {
            return parse_avcc_box(r, header.content_offset, header.content_size).map(Some);
        }
        pos = header.content_offset + header.content_size;
    }
    Ok(None)
}

/// Parse the avcC box contents.
fn parse_avcc_box<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<AvccConfig, LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0u8; size as usize];
    r.read_exact(&mut data)?;

    if data.len() < 7 {
        return Err(LiteError::Parse("avcC too short".into()));
    }

    let nal_length_size = (usize::from(data[4]) & 0x03) + 1;
    let num_sps = usize::from(data[5]) & 0x1F;
    let mut pos = 6;
    let mut sps = Vec::new();
    let mut pps = Vec::new();

    for _ in 0..num_sps {
        if pos + 2 > data.len() {
            return Err(LiteError::Parse("avcC SPS truncated".into()));
        }
        let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if pos + len > data.len() {
            return Err(LiteError::Parse("avcC SPS data truncated".into()));
        }
        sps.extend_from_slice(&data[pos..pos + len]);
        pos += len;
    }

    if pos >= data.len() {
        return Err(LiteError::Parse("avcC PPS count missing".into()));
    }
    let num_pps = usize::from(data[pos]);
    pos += 1;

    for _ in 0..num_pps {
        if pos + 2 > data.len() {
            return Err(LiteError::Parse("avcC PPS truncated".into()));
        }
        let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if pos + len > data.len() {
            return Err(LiteError::Parse("avcC PPS data truncated".into()));
        }
        pps.extend_from_slice(&data[pos..pos + len]);
        pos += len;
    }

    Ok(AvccConfig {
        nal_length_size,
        sps,
        pps,
    })
}

/// Parse mp4a sample entry to extract AAC config from esds.
fn parse_mp4a_sample_entry<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<Option<AacConfig>, LiteError> {
    // mp4a: 6 reserved + 2 data_ref_idx + 8 reserved + 2 channels + 2 sample_size
    //       + 2 compression_id + 2 packet_size + 4 sample_rate(fixed16.16)
    // Total fixed header: 28 bytes, then sub-boxes (esds).
    let end = offset + size;
    if offset + 28 > end {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(offset + 8))?; // skip reserved + data_ref_idx
    let mut hdr = [0u8; 20];
    r.read_exact(&mut hdr)?;
    let channels = u16::from_be_bytes([hdr[0], hdr[1]]); // offset 8 in entry
    // sample_rate is at offset 16 in the 20-byte header, as fixed-point 16.16
    let sr_fixed = u32::from_be_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
    let sample_rate = sr_fixed >> 16;

    // Now look for esds sub-box.
    let sub_start = offset + 28;
    let mut pos = sub_start;
    let mut config_data = Vec::new();

    while pos + 8 <= end {
        r.seek(SeekFrom::Start(pos))?;
        let header = match read_box_header(r)? {
            Some(h) => h,
            None => break,
        };
        if header.box_type == *b"esds" {
            config_data = parse_esds_for_asc(r, header.content_offset, header.content_size)?;
            break;
        }
        pos = header.content_offset + header.content_size;
    }

    Ok(Some(AacConfig {
        sample_rate,
        channels,
        config_data,
    }))
}

/// Extract AudioSpecificConfig from esds box.
fn parse_esds_for_asc<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    size: u64,
) -> Result<Vec<u8>, LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0u8; size as usize];
    r.read_exact(&mut data)?;

    // Skip version+flags (4 bytes), then scan for DecoderConfigDescriptor
    // tag (0x04) → DecoderSpecificInfo tag (0x05) → ASC bytes.
    // This is a simplified scan — real esds parsing would walk descriptors.
    let mut i = 4; // skip version+flags
    while i < data.len() {
        if data[i] == 0x05 {
            // DecoderSpecificInfo tag.
            i += 1;
            // Read variable-length size.
            let (len, consumed) = read_desc_len(&data[i..]);
            i += consumed;
            if i + len <= data.len() {
                return Ok(data[i..i + len].to_vec());
            }
        }
        i += 1;
    }
    Ok(Vec::new())
}

/// Read an ISO base media descriptor variable-length size.
fn read_desc_len(data: &[u8]) -> (usize, usize) {
    let mut len = 0usize;
    let mut consumed = 0;
    for &b in data.iter().take(4) {
        consumed += 1;
        len = (len << 7) | (b as usize & 0x7F);
        if b & 0x80 == 0 {
            break;
        }
    }
    (len, consumed)
}

/// Parse mdhd box for timescale.
fn parse_mdhd<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let version_flags = read_u32_be(r)?;
    let version = version_flags >> 24;

    let timescale = if version == 0 {
        // Skip creation_time(4) + modification_time(4).
        let mut skip = [0u8; 8];
        r.read_exact(&mut skip)?;
        read_u32_be(r)?
    } else {
        // v1: skip creation_time(8) + modification_time(8).
        let mut skip = [0u8; 16];
        r.read_exact(&mut skip)?;
        read_u32_be(r)?
    };

    if let Some(track) = current_track_mut(video, audio) {
        track.table.timescale = timescale;
    }
    Ok(())
}

/// Parse stts (time-to-sample).
fn parse_stts<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let sample_count = read_u32_be(r)?;
        let sample_delta = read_u32_be(r)?;
        entries.push(SttsEntry {
            sample_count,
            sample_delta,
        });
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stts = entries;
    }
    Ok(())
}

/// Parse ctts (composition time offsets).
fn parse_ctts<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let sample_count = read_u32_be(r)?;
        let sample_offset = read_u32_be(r)? as i32;
        entries.push(CttsEntry {
            sample_count,
            sample_offset,
        });
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.ctts = entries;
    }
    Ok(())
}

/// Parse stsc (sample-to-chunk).
fn parse_stsc<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let first_chunk = read_u32_be(r)?;
        let samples_per_chunk = read_u32_be(r)?;
        let _sample_desc_idx = read_u32_be(r)?;
        entries.push(StscEntry {
            first_chunk,
            samples_per_chunk,
        });
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stsc = entries;
    }
    Ok(())
}

/// Parse stsz (sample sizes).
fn parse_stsz<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let default_size = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    let sizes = if default_size != 0 {
        vec![default_size; count as usize]
    } else {
        let mut v = Vec::with_capacity(count as usize);
        for _ in 0..count {
            v.push(read_u32_be(r)?);
        }
        v
    };
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stsz = sizes;
    }
    Ok(())
}

/// Parse stco (chunk offsets, 32-bit).
fn parse_stco<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    let mut offsets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        offsets.push(read_u32_be(r)? as u64);
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stco = offsets;
    }
    Ok(())
}

/// Parse co64 (chunk offsets, 64-bit).
fn parse_co64<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    let mut offsets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        offsets.push(read_u64_be(r)?);
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stco = offsets;
    }
    Ok(())
}

/// Parse stss (sync sample table).
fn parse_stss<R: Read + Seek>(
    r: &mut R,
    offset: u64,
    _size: u64,
    video: &mut Option<TrackInfo>,
    audio: &mut Option<TrackInfo>,
) -> Result<(), LiteError> {
    r.seek(SeekFrom::Start(offset))?;
    let _version_flags = read_u32_be(r)?;
    let count = read_u32_be(r)?;
    let mut samples = Vec::with_capacity(count as usize);
    for _ in 0..count {
        samples.push(read_u32_be(r)?);
    }
    if let Some(track) = current_track_mut(video, audio) {
        track.table.stss = samples;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sample reading and timestamp calculation
// ---------------------------------------------------------------------------

/// Locate a sample's file offset and size using the sample table.
fn sample_file_offset(table: &SampleTable, sample_idx: usize) -> Option<(u64, u32)> {
    if sample_idx >= table.stsz.len() {
        return None;
    }
    let size = table.stsz[sample_idx];

    // Walk stsc to find which chunk this sample is in and the offset within it.
    let (chunk_idx, intra_offset) = sample_to_chunk(table, sample_idx)?;

    if chunk_idx >= table.stco.len() {
        return None;
    }
    let chunk_offset = table.stco[chunk_idx];

    // Sum sizes of preceding samples in this chunk.
    let mut offset = chunk_offset;
    let first_sample_in_chunk = intra_offset.0;
    for i in first_sample_in_chunk..sample_idx {
        if i < table.stsz.len() {
            offset += table.stsz[i] as u64;
        }
    }

    Some((offset, size))
}

/// Find the chunk index and first-sample-in-chunk for a given sample index.
fn sample_to_chunk(table: &SampleTable, sample_idx: usize) -> Option<(usize, (usize, u32))> {
    let mut sample_accum = 0usize;
    let num_chunks = table.stco.len();

    for (i, entry) in table.stsc.iter().enumerate() {
        let first_chunk = (entry.first_chunk as usize).saturating_sub(1); // 1-based → 0-based
        let next_first = if i + 1 < table.stsc.len() {
            (table.stsc[i + 1].first_chunk as usize).saturating_sub(1)
        } else {
            num_chunks
        };

        for chunk in first_chunk..next_first {
            let samples_in_chunk = entry.samples_per_chunk as usize;
            if sample_idx < sample_accum + samples_in_chunk {
                return Some((chunk, (sample_accum, entry.samples_per_chunk)));
            }
            sample_accum += samples_in_chunk;
        }
    }
    None
}

/// Calculate the PTS in timescale units for a given sample index.
fn sample_pts(table: &SampleTable, sample_idx: usize) -> u64 {
    // DTS from stts.
    let mut dts = 0u64;
    let mut sample = 0usize;
    for entry in &table.stts {
        let count = entry.sample_count as usize;
        if sample + count > sample_idx {
            dts += (sample_idx - sample) as u64 * entry.sample_delta as u64;
            break;
        }
        dts += count as u64 * entry.sample_delta as u64;
        sample += count;
    }

    // CTS offset from ctts.
    let cts_offset = if table.ctts.is_empty() {
        0i64
    } else {
        let mut s = 0usize;
        let mut offset = 0i64;
        for entry in &table.ctts {
            let count = entry.sample_count as usize;
            if s + count > sample_idx {
                offset = entry.sample_offset as i64;
                break;
            }
            s += count;
        }
        offset
    };

    (dts as i64 + cts_offset).max(0) as u64
}

/// Check if a sample is a keyframe.
fn is_keyframe(table: &SampleTable, sample_idx: usize) -> bool {
    if table.stss.is_empty() {
        // No stss = all samples are sync samples.
        return true;
    }
    let one_based = (sample_idx + 1) as u32;
    table.stss.binary_search(&one_based).is_ok()
}

/// Read a sample from the file and apply AVCC→Annex B if needed.
fn read_sample<R: Read + Seek>(
    r: &mut R,
    track: &TrackInfo,
    sample_idx: usize,
) -> Result<RawSample, LiteError> {
    let (offset, size) = sample_file_offset(&track.table, sample_idx)
        .ok_or_else(|| LiteError::Parse(format!("sample {sample_idx} not found in table")))?;

    r.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0u8; size as usize];
    r.read_exact(&mut data)?;

    let keyframe = is_keyframe(&track.table, sample_idx);

    // AVCC→Annex B for video.
    if track.kind == TrackKind::Video
        && let Some(avcc) = &track.avcc
    {
        data = avcc_to_annex_b(&data, avcc, keyframe)?;
    }

    let pts = sample_pts(&track.table, sample_idx);
    let ts = track.table.timescale;
    let timestamp_secs = if ts > 0 { pts as f64 / ts as f64 } else { 0.0 };

    Ok(RawSample {
        data,
        timestamp_secs,
        is_keyframe: keyframe,
        kind: track.kind,
    })
}

/// Convert AVCC-formatted NALs to Annex B, prepending SPS/PPS on keyframes.
/// Convert AVCC-formatted NAL units to Annex B format with start codes.
/// Prepends SPS + PPS on keyframes.
pub fn avcc_to_annex_b(
    data: &[u8],
    avcc: &AvccConfig,
    is_keyframe: bool,
) -> Result<Vec<u8>, LiteError> {
    let nls = avcc.nal_length_size;
    let mut out = Vec::with_capacity(data.len() + 64);

    // Prepend SPS + PPS on keyframes.
    if is_keyframe {
        if !avcc.sps.is_empty() {
            out.extend_from_slice(&ANNEX_B_START);
            out.extend_from_slice(&avcc.sps);
        }
        if !avcc.pps.is_empty() {
            out.extend_from_slice(&ANNEX_B_START);
            out.extend_from_slice(&avcc.pps);
        }
    }

    let mut offset = 0;
    while offset + nls <= data.len() {
        let nal_len = match nls {
            1 => usize::from(data[offset]),
            2 => u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            3 => {
                ((data[offset] as usize) << 16)
                    | ((data[offset + 1] as usize) << 8)
                    | (data[offset + 2] as usize)
            },
            4 => u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize,
            _ => return Err(LiteError::Parse("invalid NAL length size".into())),
        };
        offset += nls;

        if offset + nal_len > data.len() {
            return Err(LiteError::Parse("NAL unit exceeds sample bounds".into()));
        }

        out.extend_from_slice(&ANNEX_B_START);
        out.extend_from_slice(&data[offset..offset + nal_len]);
        offset += nal_len;
    }

    Ok(out)
}

/// Find the keyframe at or before a target timestamp (in timescale units).
fn find_keyframe_before(track: &TrackInfo, target_ts: u64) -> usize {
    if track.table.stss.is_empty() {
        // No stss — find the sample nearest the target time.
        return find_sample_at(track, target_ts);
    }

    let mut best = 0usize;
    for &sync_sample in &track.table.stss {
        let idx = (sync_sample as usize).saturating_sub(1);
        let pts = sample_pts(&track.table, idx);
        if pts <= target_ts {
            best = idx;
        } else {
            break;
        }
    }
    best
}

/// Find the sample nearest a target timestamp (linear scan of stts).
fn find_sample_at(track: &TrackInfo, target_ts: u64) -> usize {
    let mut dts = 0u64;
    let mut sample = 0usize;
    for entry in &track.table.stts {
        for _ in 0..entry.sample_count {
            if dts >= target_ts {
                return sample;
            }
            dts += entry.sample_delta as u64;
            sample += 1;
        }
    }
    sample.saturating_sub(1)
}

// ---------------------------------------------------------------------------
// Public moov-parsing API for streaming seek estimation
// ---------------------------------------------------------------------------

/// Parse raw moov atom bytes and return track information.
///
/// The input should be the complete moov atom including the 8-byte header
/// (size + `moov` fourcc).  Returns `(video_track, audio_track)`.
///
/// This is useful for streaming players that have pre-fetched the moov atom
/// and need to compute exact sample byte offsets for seeking without having
/// the full file available.
pub fn parse_moov_tracks(
    moov_data: &[u8],
) -> Result<(Option<TrackInfo>, Option<TrackInfo>), LiteError> {
    if moov_data.len() < 8 {
        return Err(LiteError::Parse("moov too short".into()));
    }
    let mut cursor = std::io::Cursor::new(moov_data);
    let moov_len = moov_data.len() as u64;

    // Skip the moov atom header (8 bytes: size + fourcc).
    // parse_boxes will recursively parse trak→mdia→minf→stbl children.
    let mut video = None;
    let mut audio = None;
    parse_boxes(&mut cursor, 8, moov_len, &mut video, &mut audio)?;
    Ok((video, audio))
}

/// Find the byte offset of the keyframe nearest to `seek_secs` using
/// sample tables parsed from moov data.
///
/// Returns `Some(byte_offset)` of the keyframe's file position, or `None`
/// if the moov doesn't contain enough information.
pub fn seek_byte_from_moov(moov_data: &[u8], seek_secs: f64) -> Option<u64> {
    let (video, _audio) = parse_moov_tracks(moov_data).ok()?;
    let track = video?;
    let count = track.sample_count();
    if count == 0 {
        return None;
    }

    // Find sample nearest to seek_secs via binary search on timestamp.
    let target_sample = {
        let mut lo = 0usize;
        let mut hi = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let ts = track.sample_timestamp(mid);
            if ts < seek_secs {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > 0 { lo - 1 } else { 0 }
    };

    // Find the nearest keyframe at or before target_sample.
    let keyframe_sample = if track.table.stss.is_empty() {
        // All frames are keyframes.
        target_sample
    } else {
        // stss is 1-based and sorted. Find largest entry <= target_sample+1.
        let one_based = (target_sample + 1) as u32;
        match track.table.stss.binary_search(&one_based) {
            Ok(_) => target_sample,
            Err(0) => {
                // Before first keyframe — use first keyframe.
                (track.table.stss[0] as usize).saturating_sub(1)
            },
            Err(i) => {
                // stss[i-1] is the largest keyframe <= target.
                (track.table.stss[i - 1] as usize).saturating_sub(1)
            },
        }
    };

    let (offset, _size) = track.sample_offset_size(keyframe_sample)?;
    Some(offset)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/test_320x240_2s.mp4")
    }

    fn fixture_bytes() -> Vec<u8> {
        std::fs::read(fixture_path()).expect("fixture file missing")
    }

    #[test]
    fn box_header_ftyp() {
        let data = fixture_bytes();
        let mut cursor = Cursor::new(&data);
        let header = read_box_header(&mut cursor).unwrap().unwrap();
        // First box in a valid MP4 is usually ftyp.
        assert_eq!(&header.box_type, b"ftyp");
        assert!(header.content_size > 0);
        assert_eq!(header.content_offset, 8);
    }

    #[test]
    fn box_header_extended_size() {
        // Fake an extended-size box: size=1, type='test', extended=24.
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes()); // size=1 → extended
        data.extend_from_slice(b"test");
        data.extend_from_slice(&24u64.to_be_bytes()); // ext size=24 → content=8
        data.extend_from_slice(&[0xAA; 8]); // content

        let mut cursor = Cursor::new(&data);
        let header = read_box_header(&mut cursor).unwrap().unwrap();
        assert_eq!(&header.box_type, b"test");
        assert_eq!(header.content_offset, 16);
        assert_eq!(header.content_size, 8);
    }

    #[test]
    fn sample_pts_from_stts() {
        let table = SampleTable {
            stts: vec![
                SttsEntry {
                    sample_count: 3,
                    sample_delta: 1000,
                },
                SttsEntry {
                    sample_count: 2,
                    sample_delta: 2000,
                },
            ],
            timescale: 30000,
            ..Default::default()
        };
        assert_eq!(sample_pts(&table, 0), 0);
        assert_eq!(sample_pts(&table, 1), 1000);
        assert_eq!(sample_pts(&table, 2), 2000);
        assert_eq!(sample_pts(&table, 3), 3000 + 0); // first in 2nd run
        assert_eq!(sample_pts(&table, 4), 3000 + 2000);
    }

    #[test]
    fn sample_pts_with_ctts() {
        let table = SampleTable {
            stts: vec![SttsEntry {
                sample_count: 3,
                sample_delta: 1000,
            }],
            ctts: vec![
                CttsEntry {
                    sample_count: 1,
                    sample_offset: 500,
                },
                CttsEntry {
                    sample_count: 2,
                    sample_offset: -200,
                },
            ],
            timescale: 1000,
            ..Default::default()
        };
        assert_eq!(sample_pts(&table, 0), 500); // DTS=0 + CTS=500
        assert_eq!(sample_pts(&table, 1), 800); // DTS=1000 + CTS=-200
    }

    #[test]
    fn keyframe_lookup_stss() {
        let table = SampleTable {
            stss: vec![1, 5, 10], // 1-based
            stsz: vec![100; 15],
            stts: vec![SttsEntry {
                sample_count: 15,
                sample_delta: 1000,
            }],
            timescale: 1000,
            ..Default::default()
        };
        assert!(is_keyframe(&table, 0)); // sample 1 (0-based=0)
        assert!(!is_keyframe(&table, 1));
        assert!(is_keyframe(&table, 4)); // sample 5 (0-based=4)
        assert!(is_keyframe(&table, 9)); // sample 10 (0-based=9)
    }

    #[test]
    fn open_fixture_finds_tracks() {
        let data = fixture_bytes();
        let mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
        assert!(mp4.video_track_info().is_some(), "should find video track");
        assert!(mp4.audio_track_info().is_some(), "should find audio track");

        let vt = mp4.video_track_info().unwrap();
        assert!(vt.avcc.is_some(), "video track should have avcC");
        assert!(vt.table.timescale > 0);
        assert!(!vt.table.stsz.is_empty());

        let at = mp4.audio_track_info().unwrap();
        assert!(
            at.aac_config.is_some(),
            "audio track should have AAC config"
        );
    }

    #[test]
    fn read_all_video_samples() {
        let data = fixture_bytes();
        let mut mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
        let mut count = 0;
        let mut last_ts = -1.0f64;
        while let Some(sample) = mp4.next_video_sample().expect("read error") {
            count += 1;
            assert_eq!(sample.kind, TrackKind::Video);
            assert!(!sample.data.is_empty());
            assert!(sample.timestamp_secs >= last_ts, "timestamp went backwards");
            last_ts = sample.timestamp_secs;
        }
        // The fixture is ~2s at ~15fps, so expect at least 10 samples.
        assert!(count >= 10, "expected >=10 video samples, got {count}");
    }

    #[test]
    fn seek_to_midstream() {
        let data = fixture_bytes();
        let mut mp4 = Mp4Lite::open(Cursor::new(data)).expect("open failed");
        mp4.seek(1.0).expect("seek failed");
        let sample = mp4
            .next_video_sample()
            .expect("read error")
            .expect("no sample");
        // Should be near 1.0s (within 0.5s tolerance).
        assert!(
            (sample.timestamp_secs - 1.0).abs() < 0.5,
            "post-seek timestamp {} not near 1.0s",
            sample.timestamp_secs
        );
    }
}
