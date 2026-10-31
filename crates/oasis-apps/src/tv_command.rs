//! Terminal `tv` command for interacting with the Internet Archive TV Guide.
//!
//! Subcommands:
//! - `tv list`           — list configured channels
//! - `tv now`            — show what's currently playing on each channel
//! - `tv tune <channel>` — tune to a channel (writes IPC request)
//! - `tv guide`          — show a text-mode schedule grid

use oasis_app_tv_guide::{self as tv_guide, ChannelConfig};
use oasis_core::error::{OasisError, Result};
use oasis_core::ipc;
use oasis_core::terminal::{Command, CommandOutput, Environment};

/// Load channel config from VFS.
fn load_channels(env: &mut Environment<'_>) -> Result<ChannelConfig> {
    let path = tv_guide::TV_CHANNELS_PATH;
    if !env.vfs.exists(path) {
        return Err(OasisError::Command(
            "No TV config found. Launch the TV Guide app first.".into(),
        ));
    }
    let data = env.vfs.read(path)?;
    let toml_str = String::from_utf8_lossy(&data);
    let config: ChannelConfig = toml::from_str(&toml_str)
        .map_err(|e| OasisError::Command(format!("bad config: {e}").into()))?;
    Ok(config)
}

pub struct TvCmd;

impl Command for TvCmd {
    fn name(&self) -> &str {
        "tv"
    }
    fn description(&self) -> &str {
        "Internet Archive TV Guide"
    }
    fn usage(&self) -> &str {
        "tv [list|now|tune <ch>|guide]"
    }
    fn category(&self) -> &str {
        "media"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let subcmd = args.first().copied().unwrap_or("list");

        match subcmd {
            "list" => tv_list(env),
            "now" => tv_now(env),
            "tune" => {
                let ch = args.get(1).copied().unwrap_or("");
                tv_tune(ch, env)
            },
            "guide" => tv_guide(env),
            _ => Err(OasisError::Command(
                format!("unknown subcommand: {subcmd}\nusage: tv [list|now|tune <ch>|guide]")
                    .into(),
            )),
        }
    }
}

/// `tv list` — list configured channels.
fn tv_list(env: &mut Environment<'_>) -> Result<CommandOutput> {
    let config = load_channels(env)?;
    if config.channel.is_empty() {
        return Ok(CommandOutput::Text("(no channels configured)".to_string()));
    }
    let mut lines = Vec::new();
    lines.push("TV Channels:".to_string());
    for ch in &config.channel {
        lines.push(format!(
            "  CH {:>2}  [{:<5}]  {} ({})",
            ch.number, ch.call_sign, ch.name, ch.genre
        ));
    }
    Ok(CommandOutput::Text(lines.join("\n")))
}

/// `tv now` — show what's currently playing on each channel.
///
/// Note: schedule data requires launching the TV Guide app to fetch
/// catalogs from Internet Archive. Without catalogs, we show channel info.
fn tv_now(env: &mut Environment<'_>) -> Result<CommandOutput> {
    let config = load_channels(env)?;
    let mut lines = Vec::new();
    lines.push("Now Playing:".to_string());
    lines.push(String::new());

    for ch in &config.channel {
        lines.push(format!(
            "  CH {:>2} [{:<5}]  {} ({})",
            ch.number, ch.call_sign, ch.name, ch.genre
        ));
    }
    lines.push(String::new());
    lines.push("Launch the TV Guide app for live schedule data.".to_string());
    Ok(CommandOutput::Text(lines.join("\n")))
}

/// `tv tune <ch>` — tune to a channel by writing a VFS IPC request.
///
/// Writes a `tune_ch:<number>` request that the app layer can resolve
/// using its in-memory catalogs.
fn tv_tune(ch_str: &str, env: &mut Environment<'_>) -> Result<CommandOutput> {
    if ch_str.is_empty() {
        return Err(OasisError::Command(
            "usage: tv tune <channel_number>".into(),
        ));
    }
    let ch_num: u32 = ch_str
        .parse()
        .map_err(|_| OasisError::Command(format!("invalid channel number: {ch_str}").into()))?;

    let config = load_channels(env)?;
    let channel = config
        .channel
        .iter()
        .find(|c| c.number == ch_num)
        .ok_or_else(|| OasisError::Command(format!("channel {ch_num} not found").into()))?;

    // A host that publishes the guide's status and says it is closed
    // would drop the request: say so instead of claiming to tune.
    let status = env.vfs.read(ipc::TV_STATUS_PATH).ok();
    if status.and_then(|s| ipc::TvStatus::parse(&String::from_utf8_lossy(&s)))
        == Some(ipc::TvStatus::GuideClosed)
    {
        return Err(OasisError::Command(
            "the TV Guide is not open (run 'open TV Guide' first)".into(),
        ));
    }

    // Write channel-based tune request for the app layer to resolve.
    let request = format!("tune_ch:{ch_num}");
    env.vfs
        .write(tv_guide::TV_REQUEST_PATH, request.as_bytes())?;

    Ok(CommandOutput::Text(format!(
        "Tuning to CH {} [{}] -- {}",
        ch_num, channel.call_sign, channel.name
    )))
}

/// `tv guide` — show a text-mode channel guide.
fn tv_guide(env: &mut Environment<'_>) -> Result<CommandOutput> {
    let config = load_channels(env)?;

    let mut lines = Vec::new();
    lines.push("TV Guide".to_string());
    lines.push("-".repeat(50));

    for ch in &config.channel {
        lines.push(format!(
            "  CH {:>2}  [{:<5}]  {} ({})",
            ch.number, ch.call_sign, ch.name, ch.genre
        ));
        for src in &ch.source {
            lines.push(format!("           Source: {}", src.item_id));
        }
    }

    lines.push("-".repeat(50));
    lines.push("Use 'tv tune <ch>' to watch a channel.".to_string());
    lines.push("Launch the TV Guide app for the full EPG grid.".to_string());

    Ok(CommandOutput::Text(lines.join("\n")))
}

/// Register TV commands into a command registry.
pub fn register_tv_commands(reg: &mut oasis_core::terminal::CommandRegistry) {
    reg.register(Box::new(TvCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_core::terminal::{CommandRegistry, Environment};
    use oasis_core::vfs::{MemoryVfs, Vfs};

    fn setup_tv_env() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_tv_commands(&mut reg);

        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/etc").unwrap();
        vfs.mkdir("/etc/tv").unwrap();
        vfs.mkdir("/var").unwrap();
        vfs.mkdir("/var/tv").unwrap();
        vfs.mkdir("/var/tv/cache").unwrap();
        vfs.write(
            "/etc/tv/channels.toml",
            oasis_app_tv_guide::channel::DEFAULT_CHANNELS_TOML.as_bytes(),
        )
        .unwrap();

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
    fn tv_list_shows_channels() {
        let (reg, mut vfs) = setup_tv_env();
        let __out = exec(&reg, &mut vfs, "tv list").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("RETRO"), "should contain RETRO call sign");
        assert!(s.contains("TECH"), "should contain TECH call sign");
        assert!(s.contains("TV Channels:"));
    }

    #[test]
    fn tv_default_is_list() {
        let (reg, mut vfs) = setup_tv_env();
        let __out = exec(&reg, &mut vfs, "tv").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("TV Channels:"));
    }

    #[test]
    fn tv_now_shows_channels() {
        let (reg, mut vfs) = setup_tv_env();
        let __out = exec(&reg, &mut vfs, "tv now").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("Now Playing"));
        assert!(s.contains("RETRO"));
    }

    #[test]
    fn tv_tune_no_channel() {
        let (reg, mut vfs) = setup_tv_env();
        assert!(exec(&reg, &mut vfs, "tv tune").is_err());
    }

    #[test]
    fn tv_tune_bad_number() {
        let (reg, mut vfs) = setup_tv_env();
        assert!(exec(&reg, &mut vfs, "tv tune abc").is_err());
    }

    #[test]
    fn tv_tune_unknown_channel() {
        let (reg, mut vfs) = setup_tv_env();
        assert!(exec(&reg, &mut vfs, "tv tune 99").is_err());
    }

    #[test]
    fn tv_tune_refuses_when_the_host_says_the_guide_is_closed() {
        let (reg, mut vfs) = setup_tv_env();
        vfs.write(ipc::TV_STATUS_PATH, b"guide closed").unwrap();
        let err = exec(&reg, &mut vfs, "tv tune 2").unwrap_err();
        assert!(err.to_string().contains("not open"), "{err}");
        assert!(
            vfs.read(ipc::TV_REQUEST_PATH).is_err(),
            "no request written"
        );
        // Once the guide is open the request goes through.
        vfs.write(ipc::TV_STATUS_PATH, b"guide open").unwrap();
        assert!(exec(&reg, &mut vfs, "tv tune 2").is_ok());
    }

    #[test]
    fn tv_tune_writes_request() {
        let (reg, mut vfs) = setup_tv_env();
        let __out = exec(&reg, &mut vfs, "tv tune 2").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("Tuning"));
        assert!(s.contains("RETRO"));
        let data = vfs.read("/var/tv/request").unwrap();
        assert_eq!(String::from_utf8_lossy(&data), "tune_ch:2");
    }

    #[test]
    fn tv_guide_shows_channels() {
        let (reg, mut vfs) = setup_tv_env();
        let __out = exec(&reg, &mut vfs, "tv guide").unwrap();
        let CommandOutput::Text(s) = __out else {
            panic!("expected CommandOutput::Text, got {__out:?}");
        };
        assert!(s.contains("TV Guide"));
        assert!(s.contains("RETRO"));
        assert!(s.contains("tv tune"));
    }

    #[test]
    fn tv_unknown_subcommand() {
        let (reg, mut vfs) = setup_tv_env();
        assert!(exec(&reg, &mut vfs, "tv foo").is_err());
    }

    #[test]
    fn tv_no_config() {
        let mut reg = CommandRegistry::new();
        register_tv_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        assert!(exec(&reg, &mut vfs, "tv list").is_err());
    }
}
