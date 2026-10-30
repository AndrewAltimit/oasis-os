//! Audio playback commands for the terminal.
//!
//! Provides a `music` command with subcommands for controlling audio
//! playback. Uses VFS-based IPC: writes [`MediaRequest`] lines to
//! [`AUDIO_REQUEST_PATH`] and reads the host's status from
//! [`AUDIO_STATUS_PATH`]. On the desktop shell the consumer is
//! `oasis-app/src/media_controller.rs`.

use oasis_types::error::OasisError;
#[cfg(test)]
use oasis_types::error::Result;
use oasis_types::ipc::{AUDIO_REQUEST_PATH, AUDIO_STATUS_PATH, MediaRequest, RepeatMode};

use crate::CommandOutput;
use crate::interpreter::resolve_path;

/// Usage string shared by the command metadata and its error messages.
const MUSIC_USAGE: &str = "music [status|play [file]|pause|resume|stop|next|prev|\
                           vol [0-100]|list|repeat <off|all|one>|shuffle]";

/// Pending request lines kept when nothing drains a request file (a host
/// without the matching controller); older lines are dropped.
const MAX_PENDING_REQUESTS: usize = 32;

/// Append `line` to the line-queued IPC request file `path`. The host
/// drains every line per tick, so two commands in one frame
/// (`music vol 40; music play`, `open Calculator; open Paint`) are both
/// seen instead of the second overwriting the first.
pub(crate) fn queue_request_line(
    vfs: &mut dyn oasis_vfs::Vfs,
    path: &str,
    line: &str,
) -> oasis_types::error::Result<()> {
    let data = vfs.read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&data);
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.push(line);
    let skip = lines.len().saturating_sub(MAX_PENDING_REQUESTS);
    vfs.write(path, lines[skip..].join("\n").as_bytes())
}

/// Queue `request` on [`AUDIO_REQUEST_PATH`] (see [`queue_request_line`]).
fn queue(
    env: &mut crate::Environment<'_>,
    request: &MediaRequest,
) -> oasis_types::error::Result<()> {
    queue_request_line(env.vfs, AUDIO_REQUEST_PATH, &request.to_line())
}

/// The `Key: value` line for `key` from the published status, if any.
fn status_line(env: &mut crate::Environment<'_>, key: &str) -> Option<String> {
    let data = env.vfs.read(AUDIO_STATUS_PATH).ok()?;
    let text = String::from_utf8_lossy(&data);
    text.lines()
        .find(|l| l.starts_with(key))
        .map(str::to_string)
}

define_command!(
    MusicCmd,
    "music",
    "Control audio playback",
    MUSIC_USAGE,
    "audio",
    |args, env| {
        let subcmd = args.first().copied().unwrap_or("status");

        let request = match subcmd {
            "status" => {
                if !env.vfs.exists(AUDIO_STATUS_PATH) {
                    return Ok(CommandOutput::Text(
                        "(audio subsystem not initialized)".to_string(),
                    ));
                }
                let data = env.vfs.read(AUDIO_STATUS_PATH)?;
                let text = String::from_utf8_lossy(&data).into_owned();
                if text.trim().is_empty() {
                    return Ok(CommandOutput::Text(
                        "(no audio status available)".to_string(),
                    ));
                }
                return Ok(CommandOutput::Text(text));
            },
            "list" => {
                let line = if env.vfs.exists(AUDIO_STATUS_PATH) {
                    status_line(env, "Playlist:").unwrap_or_else(|| "Playlist: unknown".into())
                } else {
                    "(no playlist available)".to_string()
                };
                return Ok(CommandOutput::Text(line));
            },
            "play" => match args.get(1) {
                Some(file) => {
                    let path = resolve_path(&env.cwd, file);
                    if !env.vfs.exists(&path) {
                        return Err(OasisError::Command(
                            format!("music: {path}: no such file").into(),
                        ));
                    }
                    MediaRequest::PlayFile(path)
                },
                None => MediaRequest::Play,
            },
            "pause" => MediaRequest::Pause,
            "resume" => MediaRequest::Resume,
            "stop" => MediaRequest::Stop,
            "next" => MediaRequest::Next,
            "prev" => MediaRequest::Prev,
            "shuffle" => MediaRequest::Shuffle,
            "vol" => {
                let Some(vol_str) = args.get(1).copied() else {
                    let line = status_line(env, "Volume:")
                        .unwrap_or_else(|| "Volume: unknown".to_string());
                    return Ok(CommandOutput::Text(line));
                };
                let vol = vol_str
                    .parse::<u8>()
                    .ok()
                    .filter(|v| *v <= 100)
                    .ok_or_else(|| {
                        OasisError::Command(format!("invalid volume: {vol_str} (0-100)").into())
                    })?;
                MediaRequest::Volume(vol)
            },
            "repeat" => {
                let Some(mode) = args.get(1).copied() else {
                    return Err(OasisError::Command(
                        "usage: music repeat <off|all|one>".into(),
                    ));
                };
                let mode = RepeatMode::parse(mode).ok_or_else(|| {
                    OasisError::Command(
                        format!("invalid repeat mode: {mode} (use off/all/one)").into(),
                    )
                })?;
                MediaRequest::Repeat(mode)
            },
            _ => {
                return Err(OasisError::Command(
                    format!("unknown subcommand: {subcmd}\nusage: {MUSIC_USAGE}").into(),
                ));
            },
        };
        queue(env, &request)?;
        Ok(CommandOutput::Text(format!(
            "Audio request queued: {request}"
        )))
    }
);

register_commands!(register_audio_commands, [MusicCmd]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_audio_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/audio").unwrap();
        (reg, vfs)
    }

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,

            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn music_status_no_audio() {
        let mut reg = CommandRegistry::new();
        register_audio_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "music status").unwrap());
        assert!(s.contains("not initialized"));
    }

    #[test]
    fn music_status_default() {
        let (reg, mut vfs) = setup();
        vfs.write(AUDIO_STATUS_PATH, b"State: stopped\nVolume: 80%")
            .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "music").unwrap());
        assert!(s.contains("stopped"));
        assert!(s.contains("80%"));
    }

    #[test]
    fn music_play_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music play").unwrap());
        assert!(s.contains("play"));
        let data = vfs.read(AUDIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"play");
    }

    #[test]
    fn music_pause_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music pause").unwrap());
        assert!(s.contains("pause"));
        let data = vfs.read(AUDIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"pause");
    }

    #[test]
    fn music_stop_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music stop").unwrap());
        assert!(s.contains("stop"));
    }

    #[test]
    fn music_next_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music next").unwrap());
        assert!(s.contains("next"));
    }

    #[test]
    fn music_prev_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music prev").unwrap());
        assert!(s.contains("prev"));
    }

    #[test]
    fn music_vol_set() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music vol 42").unwrap());
        assert!(s.contains("vol 42"), "{s}");
        let data = vfs.read(AUDIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"vol 42");
    }

    #[test]
    fn music_vol_invalid() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "music vol abc").is_err());
    }

    #[test]
    fn music_vol_show_current() {
        let (reg, mut vfs) = setup();
        vfs.write(AUDIO_STATUS_PATH, b"State: playing\nVolume: 65%")
            .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "music vol").unwrap());
        assert!(s.contains("65%"));
    }

    #[test]
    fn music_repeat_valid() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music repeat all").unwrap());
        assert!(s.contains("all"));
        let data = vfs.read(AUDIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"repeat all");
    }

    #[test]
    fn music_repeat_invalid() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "music repeat xyz").is_err());
    }

    #[test]
    fn music_repeat_no_arg() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "music repeat").is_err());
    }

    #[test]
    fn music_shuffle_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "music shuffle").unwrap());
        assert!(s.contains("shuffle"));
    }

    #[test]
    fn music_list_with_status() {
        let (reg, mut vfs) = setup();
        vfs.write(AUDIO_STATUS_PATH, b"Playlist: 5 tracks").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "music list").unwrap());
        assert!(s.contains("5 tracks"));
    }

    #[test]
    fn music_list_no_status() {
        let mut reg = CommandRegistry::new();
        register_audio_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "music list").unwrap());
        assert!(s.contains("no playlist"));
    }

    #[test]
    fn music_unknown_subcommand() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "music badcmd").is_err());
    }

    #[test]
    fn music_play_file_resolves_relative_path() {
        let (reg, mut vfs) = setup();
        vfs.mkdir("/music").unwrap();
        vfs.write("/music/a.mp3", b"ID3").unwrap();
        let mut env = Environment {
            cwd: "/music".to_string(),
            vfs: &mut vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
            stderr: String::new(),
        };
        reg.execute("music play a.mp3", &mut env).unwrap();
        let data = vfs.read(AUDIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"play_file /music/a.mp3");
    }

    #[test]
    fn music_play_missing_file_errors() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "music play /nope.mp3").is_err());
        assert!(vfs.read(AUDIO_REQUEST_PATH).is_err());
    }

    #[test]
    fn music_vol_out_of_range() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "music vol 101").is_err());
    }

    #[test]
    fn requests_in_one_frame_queue_up_and_parse() {
        let (reg, mut vfs) = setup();
        exec(
            &reg,
            &mut vfs,
            "music vol 40; music repeat one; music shuffle",
        )
        .unwrap();
        let data = String::from_utf8(vfs.read(AUDIO_REQUEST_PATH).unwrap()).unwrap();
        let parsed: Vec<MediaRequest> = data.lines().filter_map(MediaRequest::parse).collect();
        assert_eq!(
            parsed,
            vec![
                MediaRequest::Volume(40),
                MediaRequest::Repeat(RepeatMode::One),
                MediaRequest::Shuffle,
            ]
        );
    }

    #[test]
    fn undrained_queue_is_bounded() {
        let (reg, mut vfs) = setup();
        for _ in 0..(MAX_PENDING_REQUESTS + 10) {
            exec(&reg, &mut vfs, "music next").unwrap();
        }
        let data = String::from_utf8(vfs.read(AUDIO_REQUEST_PATH).unwrap()).unwrap();
        assert_eq!(data.lines().count(), MAX_PENDING_REQUESTS);
    }
}
