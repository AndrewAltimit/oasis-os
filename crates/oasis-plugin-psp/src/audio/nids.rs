//! NID constants and module lookup tables for runtime function resolution.

// ---------------------------------------------------------------------------
// sceAudio driver NIDs
// ---------------------------------------------------------------------------

pub(super) const NID_AUDIO_CH_RESERVE: u32 = 0x5EC81C55;
pub(super) const NID_AUDIO_OUTPUT_BLOCKING: u32 = 0x136CAF51;
pub(super) const NID_AUDIO_CH_RELEASE: u32 = 0x6FC46853;
pub(super) const NID_AUDIO_SET_CH_VOL: u32 = 0xB7E1D8E7;

// SRC (Sample Rate Conversion) channel -- separate output that
// does NOT conflict with the 8 regular PCM channels games use.
pub(super) const NID_AUDIO_SRC_CH_RESERVE: u32 = 0x01562BA3;
pub(super) const NID_AUDIO_SRC_OUTPUT_BLOCKING: u32 = 0xE0727056;
pub(super) const NID_AUDIO_SRC_CH_RELEASE: u32 = 0x5C37C0AE;

pub(super) const AUDIO_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceAudio_Driver\0", b"sceAudio_driver\0"),
    (b"sceAudio_Driver\0", b"sceAudio\0"),
    (b"sceAudio_Service\0", b"sceAudio_driver\0"),
    (b"sceAudio_Service\0", b"sceAudio\0"),
];

// ---------------------------------------------------------------------------
// sceUtility NIDs (for loading optional AV modules)
// ---------------------------------------------------------------------------

/// sceUtilityLoadModule(module_id) -> 0
pub(super) const NID_UTILITY_LOAD_MODULE: u32 = 0x2A2B3DE0;

pub(super) const UTILITY_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceUtility_Driver\0", b"sceUtility_private\0"),
    (b"sceUtility_Driver\0", b"sceUtility_driver\0"),
    (b"sceUtility_Driver\0", b"sceUtility\0"),
    (b"sceUtility_private\0", b"sceUtility_private\0"),
    (b"sceUtility_private\0", b"sceUtility\0"),
];

/// PSP optional module IDs for sceUtilityLoadModule.
pub(super) const PSP_MODULE_AV_AVCODEC: i32 = 0x0300;
pub(super) const PSP_MODULE_AV_MPEGBASE: i32 = 0x0301;
pub(super) const PSP_MODULE_AV_MP3: i32 = 0x0302;

// ---------------------------------------------------------------------------
// sceMp3 NIDs (preferred -- higher-level streaming API)
// ---------------------------------------------------------------------------

pub(super) const NID_MP3_INIT_RESOURCE: u32 = 0x35750070;
#[allow(dead_code)]
pub(super) const NID_MP3_TERM_RESOURCE: u32 = 0xD0A56296;
pub(super) const NID_MP3_RESERVE_HANDLE: u32 = 0x7F2A1880;
pub(super) const NID_MP3_RELEASE_HANDLE: u32 = 0x0DB149F4;
pub(super) const NID_MP3_INIT: u32 = 0x44E07129;
pub(super) const NID_MP3_DECODE: u32 = 0xD021C0FB;
pub(super) const NID_MP3_CHECK_NEED_DATA: u32 = 0xD8F54A51;
pub(super) const NID_MP3_GET_INFO_TO_ADD: u32 = 0x732B042A;
pub(super) const NID_MP3_NOTIFY_ADD_DATA: u32 = 0x29BFF3EC;

pub(super) const MP3_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceMp3\0", b"sceMp3\0"),
    (b"sceMp3_Library\0", b"sceMp3\0"),
    (b"libmp3\0", b"sceMp3\0"),
    (b"sceMp3_Service\0", b"sceMp3\0"),
];

// ---------------------------------------------------------------------------
// sceAudiocodec NIDs (fallback -- lower-level codec API)
// ---------------------------------------------------------------------------

pub(super) const NID_CODEC_CHECK_NEED_MEM: u32 = 0x9D3F790C;
pub(super) const NID_CODEC_INIT: u32 = 0x5B37EB1D;
pub(super) const NID_CODEC_DECODE: u32 = 0x70A703F8;
pub(super) const NID_CODEC_GET_EDRAM: u32 = 0x3A20A200;
pub(super) const NID_CODEC_RELEASE_EDRAM: u32 = 0x29681260;

pub(super) const CODEC_MODULES: &[(&[u8], &[u8])] = &[
    // mp3play.prx uses this exact module/library pair:
    (b"sceAvcodec_wrapper\0", b"sceAudiocodec\0"),
    (b"sceAVcodec_driver\0", b"sceAudiocodec\0"),
    (b"sceAvcodec_driver\0", b"sceAudiocodec\0"),
    (b"sceAudiocodec_Driver\0", b"sceAudiocodec\0"),
    (b"avcodec\0", b"sceAudiocodec\0"),
    (b"sceAudiocodec\0", b"sceAudiocodec\0"),
];

pub(super) const CODEC_TYPE_MP3: i32 = 0x1002;

// ---------------------------------------------------------------------------
// Network driver NIDs (resolved at runtime for radio streaming)
// ---------------------------------------------------------------------------

/// sceNetInit(poolsize, calloutpri, calloutstack, netintrpri, netintrstack)
pub(super) const NID_NET_INIT: u32 = 0x39AF39A6;

pub(super) const NET_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceNet_Library\0", b"sceNet\0"),
    (b"sceNet\0", b"sceNet\0"),
    (b"sceNet_Service\0", b"sceNet\0"),
];

/// sceNetInet NIDs
pub(super) const NID_INET_INIT: u32 = 0x17943399;
pub(super) const NID_INET_SOCKET: u32 = 0x8B7B220F;
pub(super) const NID_INET_CONNECT: u32 = 0x410B34AA;
pub(super) const NID_INET_SEND: u32 = 0x7AA671BC;
pub(super) const NID_INET_RECV: u32 = 0xCDA85C99;
pub(super) const NID_INET_CLOSE: u32 = 0x8D7284EA;

pub(super) const INET_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceNetInet_Library\0", b"sceNetInet\0"),
    (b"sceNet_Inet\0", b"sceNetInet\0"),
    (b"sceNetInet\0", b"sceNetInet\0"),
];

/// sceNetApctl NIDs
pub(super) const NID_APCTL_INIT: u32 = 0xE2F91F9B;
pub(super) const NID_APCTL_CONNECT: u32 = 0xCFB957C6;
pub(super) const NID_APCTL_GET_STATE: u32 = 0x5DEAC81B;

pub(super) const APCTL_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceNetApctl_Library\0", b"sceNetApctl\0"),
    (b"sceNet_Apctl\0", b"sceNetApctl\0"),
    (b"sceNetApctl\0", b"sceNetApctl\0"),
];

/// sceNetResolver NIDs
pub(super) const NID_RESOLVER_INIT: u32 = 0xF3370E61;
pub(super) const NID_RESOLVER_CREATE: u32 = 0x244172AF;
pub(super) const NID_RESOLVER_START_N2A: u32 = 0x629E2FB7;
pub(super) const NID_RESOLVER_DELETE: u32 = 0x94523E09;

pub(super) const RESOLVER_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceNetResolver_Library\0", b"sceNetResolver\0"),
    (b"sceNet_Resolver\0", b"sceNetResolver\0"),
    (b"sceNetResolver\0", b"sceNetResolver\0"),
];

/// Network module IDs for sceUtilityLoadModule.
pub(super) const PSP_MODULE_NET_COMMON: i32 = 0x0100;
pub(super) const PSP_MODULE_NET_INET: i32 = 0x0102;

// ---------------------------------------------------------------------------
// Module enumeration (for discovering loaded AV modules)
// ---------------------------------------------------------------------------

/// sceKernelGetModuleIdList(readbuf, readbufsize, idcount)
pub(super) const NID_GET_MODULE_ID_LIST: u32 = 0x644CF325;
/// sceKernelQueryModuleInfo(uid, info)
pub(super) const NID_QUERY_MODULE_INFO: u32 = 0x748CBED9;

/// Module/library pairs for ModuleMgrForKernel.
pub(super) const MOD_MGR_MODULES: &[(&[u8], &[u8])] = &[
    (b"sceModuleManager\0", b"ModuleMgrForKernel\0"),
    (b"ModuleMgrForKernel\0", b"ModuleMgrForKernel\0"),
];

/// Target module name substrings to match when enumerating modules.
/// If a loaded module's name contains one of these, we try to walk
/// its exports for sceMp3 / sceAudiocodec NIDs.
pub(super) const MP3_NAME_PATTERNS: &[&[u8]] = &[b"mp3", b"Mp3", b"MP3"];
pub(super) const CODEC_NAME_PATTERNS: &[&[u8]] = &[b"codec", b"Codec", b"avcodec", b"Avcodec"];

/// SceKernelModuleInfo struct size.
pub(super) const MODULE_INFO_SIZE: u32 = 96;
/// Offset of text_addr in SceKernelModuleInfo.
pub(super) const MODINFO_TEXT_ADDR: usize = 0x30;
/// Offset of name in SceKernelModuleInfo.
pub(super) const MODINFO_NAME: usize = 0x44;

/// SceModuleInfo (embedded in module binary) offsets for export table.
/// ent_top at +0x24, ent_end at +0x28 (NOT size -- subtract for size).
pub(super) const SCEMODINFO_ENT_TOP: usize = 0x24;
pub(super) const SCEMODINFO_ENT_END: usize = 0x28;
