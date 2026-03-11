//! Audio playback commands for the terminal.
//!
//! Provides a `music` command with subcommands for controlling audio
//! playback. Uses VFS-based IPC: reads status from `/var/audio/status`
//! and writes requests to `/var/audio/request`.

use oasis_audio::{AUDIO_REQUEST_PATH, AUDIO_STATUS_PATH};
use oasis_types::error::{OasisError, Result};

use crate::{Command, CommandOutput, Environment};

/// Terminal command for controlling audio playback via VFS-based IPC.
pub struct MusicCmd;
impl Command for MusicCmd {
    fn name(&self) -> &str {
        "music"
    }
    fn description(&self) -> &str {
        "Control audio playback"
    }
    fn usage(&self) -> &str {
        "music [status|play|pause|resume|stop|next|prev|vol <0-100>|list|repeat <off|all|one>|shuffle]"
    }
    fn category(&self) -> &str {
        "audio"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("status");

        match subcmd {
            "status" => {
                if env.vfs.exists(AUDIO_STATUS_PATH) {
                    let data = env.vfs.read(AUDIO_STATUS_PATH)?;
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if text.trim().is_empty() {
                        Ok(CommandOutput::Text(
                            "(no audio status available)".to_string(),
                        ))
                    } else {
                        Ok(CommandOutput::Text(text))
                    }
                } else {
                    Ok(CommandOutput::Text(
                        "(audio subsystem not initialized)".to_string(),
                    ))
                }
            },
            "play" | "pause" | "resume" | "stop" | "next" | "prev" | "shuffle" => {
                env.vfs.write(AUDIO_REQUEST_PATH, subcmd.as_bytes())?;
                Ok(CommandOutput::Text(format!(
                    "Audio request queued: {subcmd}"
                )))
            },
            "vol" => {
                let vol_str = args.get(1).copied().unwrap_or("");
                if vol_str.is_empty() {
                    // Just show current volume from status.
                    if env.vfs.exists(AUDIO_STATUS_PATH) {
                        let data = env.vfs.read(AUDIO_STATUS_PATH)?;
                        let text = String::from_utf8_lossy(&data);
                        let vol_line = text
                            .lines()
                            .find(|l| l.starts_with("Volume:"))
                            .unwrap_or("Volume: unknown");
                        return Ok(CommandOutput::Text(vol_line.to_string()));
                    }
                    return Ok(CommandOutput::Text("Volume: unknown".to_string()));
                }
                let _vol: u8 = vol_str.parse().map_err(|_| {
                    OasisError::Command(format!("invalid volume: {vol_str}").into())
                })?;
                let request = format!("vol {vol_str}");
                env.vfs.write(AUDIO_REQUEST_PATH, request.as_bytes())?;
                Ok(CommandOutput::Text(format!(
                    "Volume request queued: {vol_str}%"
                )))
            },
            "repeat" => {
                let mode = args.get(1).copied().unwrap_or("");
                if mode.is_empty() {
                    return Err(OasisError::Command(
                        "usage: music repeat <off|all|one>".into(),
                    ));
                }
                // Validate the mode locally before sending.
                match mode {
                    "off" | "all" | "one" => {},
                    _ => {
                        return Err(OasisError::Command(
                            format!("invalid repeat mode: {mode} (use off/all/one)").into(),
                        ));
                    },
                }
                let request = format!("repeat {mode}");
                env.vfs.write(AUDIO_REQUEST_PATH, request.as_bytes())?;
                Ok(CommandOutput::Text(format!(
                    "Repeat mode request queued: {mode}"
                )))
            },
            "list" => {
                if env.vfs.exists(AUDIO_STATUS_PATH) {
                    let data = env.vfs.read(AUDIO_STATUS_PATH)?;
                    let text = String::from_utf8_lossy(&data);
                    let playlist_line = text
                        .lines()
                        .find(|l| l.starts_with("Playlist:"))
                        .unwrap_or("Playlist: unknown");
                    Ok(CommandOutput::Text(playlist_line.to_string()))
                } else {
                    Ok(CommandOutput::Text("(no playlist available)".to_string()))
                }
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}\nusage: {}", self.usage()).into(),
            )),
        }
    }
}

/// Register audio commands into a registry.
pub fn register_audio_commands(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(MusicCmd));
}

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
        assert!(s.contains("42%"));
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
}
