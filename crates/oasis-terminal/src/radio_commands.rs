//! Internet radio commands for the terminal.
//!
//! Provides a `radio` command with subcommands for controlling internet
//! radio playback. Uses VFS-based IPC: reads status from `/var/radio/status`
//! and writes requests to `/var/radio/request`.

use oasis_audio::{RADIO_REQUEST_PATH, RADIO_STATUS_PATH};
use oasis_types::error::OasisError;
#[cfg(test)]
use oasis_types::error::Result;

use crate::CommandOutput;
define_command!(
    RadioCmd,
    "radio",
    "Control internet radio",
    "radio [status|stations|tune <name|index>|stop|vol [0-100]|fav <index>|genre [name]|info]",
    "audio",
    |args, env| {
        let subcmd = args.first().copied().unwrap_or("status");

        match subcmd {
            "status" => {
                if env.vfs.exists(RADIO_STATUS_PATH) {
                    let data = env.vfs.read(RADIO_STATUS_PATH)?;
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if text.trim().is_empty() {
                        Ok(CommandOutput::Text(
                            "(no radio status available)".to_string(),
                        ))
                    } else {
                        Ok(CommandOutput::Text(text))
                    }
                } else {
                    Ok(CommandOutput::Text(
                        "(radio subsystem not initialized)".to_string(),
                    ))
                }
            },
            "stations" => {
                if env.vfs.exists(RADIO_STATUS_PATH) {
                    let data = env.vfs.read(RADIO_STATUS_PATH)?;
                    let text = String::from_utf8_lossy(&data);
                    let count = text
                        .lines()
                        .find(|l| l.starts_with("Stations:"))
                        .unwrap_or("Stations: unknown");
                    Ok(CommandOutput::Text(count.to_string()))
                } else {
                    Ok(CommandOutput::Text(
                        "(radio subsystem not initialized)".to_string(),
                    ))
                }
            },
            "tune" => {
                let target = args.get(1).copied().unwrap_or("");
                if target.is_empty() {
                    return Err(OasisError::Command("usage: radio tune <name|index>".into()));
                }
                let request = format!("tune {target}");
                env.vfs.write(RADIO_REQUEST_PATH, request.as_bytes())?;
                Ok(CommandOutput::Text(format!(
                    "Radio tune request queued: {target}"
                )))
            },
            "stop" => {
                env.vfs.write(RADIO_REQUEST_PATH, b"stop")?;
                Ok(CommandOutput::Text("Radio stop request queued".to_string()))
            },
            "vol" => {
                let vol_str = args.get(1).copied().unwrap_or("");
                if vol_str.is_empty() {
                    if env.vfs.exists(RADIO_STATUS_PATH) {
                        let data = env.vfs.read(RADIO_STATUS_PATH)?;
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
                env.vfs.write(RADIO_REQUEST_PATH, request.as_bytes())?;
                Ok(CommandOutput::Text(format!(
                    "Volume request queued: {vol_str}%"
                )))
            },
            "fav" => {
                let idx_str = args.get(1).copied().unwrap_or("");
                if idx_str.is_empty() {
                    return Err(OasisError::Command("usage: radio fav <index>".into()));
                }
                let _idx: usize = idx_str
                    .parse()
                    .map_err(|_| OasisError::Command(format!("invalid index: {idx_str}").into()))?;
                let request = format!("fav {idx_str}");
                env.vfs.write(RADIO_REQUEST_PATH, request.as_bytes())?;
                Ok(CommandOutput::Text(format!(
                    "Favorite toggle request queued: station {idx_str}"
                )))
            },
            "genre" => {
                let genre = args.get(1).copied().unwrap_or("");
                let request = if genre.is_empty() {
                    "genre".to_string()
                } else {
                    format!("genre {genre}")
                };
                env.vfs.write(RADIO_REQUEST_PATH, request.as_bytes())?;
                if genre.is_empty() {
                    Ok(CommandOutput::Text("Genre list request queued".to_string()))
                } else {
                    Ok(CommandOutput::Text(format!(
                        "Genre filter request queued: {genre}"
                    )))
                }
            },
            "info" => {
                if env.vfs.exists(RADIO_STATUS_PATH) {
                    let data = env.vfs.read(RADIO_STATUS_PATH)?;
                    let text = String::from_utf8_lossy(&data);
                    let mut info_lines = Vec::new();
                    for line in text.lines() {
                        if line.starts_with("Station:")
                            || line.starts_with("Now Playing:")
                            || line.starts_with("Buffer:")
                            || line.starts_with("Source:")
                            || line.starts_with("Collection:")
                        {
                            info_lines.push(line.to_string());
                        }
                    }
                    if info_lines.is_empty() {
                        Ok(CommandOutput::Text("(no stream info)".to_string()))
                    } else {
                        Ok(CommandOutput::Text(info_lines.join("\n")))
                    }
                } else {
                    Ok(CommandOutput::Text(
                        "(radio subsystem not initialized)".to_string(),
                    ))
                }
            },
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}\nusage: radio [play|stop|list|status|fav]")
                    .into(),
            )),
        }
    }
);

register_commands!(register_radio_commands, [RadioCmd]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::assert_text;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_radio_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/radio").unwrap();
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
    fn radio_status_no_subsystem() {
        let mut reg = CommandRegistry::new();
        register_radio_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "radio status").unwrap());
        assert!(s.contains("not initialized"));
    }

    #[test]
    fn radio_status_default() {
        let (reg, mut vfs) = setup();
        vfs.write(RADIO_STATUS_PATH, b"State: stopped\nVolume: 80%")
            .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "radio").unwrap());
        assert!(s.contains("stopped"));
        assert!(s.contains("80%"));
    }

    #[test]
    fn radio_tune_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "radio tune 0").unwrap());
        assert!(s.contains("tune"));
        let data = vfs.read(RADIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"tune 0");
    }

    #[test]
    fn radio_tune_no_arg() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "radio tune").is_err());
    }

    #[test]
    fn radio_stop_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "radio stop").unwrap());
        assert!(s.contains("stop"));
        let data = vfs.read(RADIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"stop");
    }

    #[test]
    fn radio_vol_set() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "radio vol 42").unwrap());
        assert!(s.contains("42%"));
        let data = vfs.read(RADIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"vol 42");
    }

    #[test]
    fn radio_vol_invalid() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "radio vol abc").is_err());
    }

    #[test]
    fn radio_vol_show_current() {
        let (reg, mut vfs) = setup();
        vfs.write(RADIO_STATUS_PATH, b"State: playing\nVolume: 65%")
            .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "radio vol").unwrap());
        assert!(s.contains("65%"));
    }

    #[test]
    fn radio_fav_queues_request() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "radio fav 2").unwrap());
        assert!(s.contains("2"));
        let data = vfs.read(RADIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"fav 2");
    }

    #[test]
    fn radio_fav_no_arg() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "radio fav").is_err());
    }

    #[test]
    fn radio_genre_filter() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "radio genre ambient").unwrap());
        assert!(s.contains("ambient"));
        let data = vfs.read(RADIO_REQUEST_PATH).unwrap();
        assert_eq!(data, b"genre ambient");
    }

    #[test]
    fn radio_genre_list() {
        let (reg, mut vfs) = setup();
        let s = assert_text!(exec(&reg, &mut vfs, "radio genre").unwrap());
        assert!(s.contains("Genre list"));
    }

    #[test]
    fn radio_info_with_status() {
        let (reg, mut vfs) = setup();
        vfs.write(
            RADIO_STATUS_PATH,
            b"State: playing\nStation: Test FM\nNow Playing: Artist - Song\nBuffer: 32 KB",
        )
        .unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "radio info").unwrap());
        assert!(s.contains("Test FM"));
        assert!(s.contains("Artist - Song"));
    }

    #[test]
    fn radio_info_no_subsystem() {
        let mut reg = CommandRegistry::new();
        register_radio_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        let s = assert_text!(exec(&reg, &mut vfs, "radio info").unwrap());
        assert!(s.contains("not initialized"));
    }

    #[test]
    fn radio_stations_with_status() {
        let (reg, mut vfs) = setup();
        vfs.write(RADIO_STATUS_PATH, b"Stations: 8").unwrap();
        let s = assert_text!(exec(&reg, &mut vfs, "radio stations").unwrap());
        assert!(s.contains("8"));
    }

    #[test]
    fn radio_unknown_subcommand() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "radio badcmd").is_err());
    }
}
