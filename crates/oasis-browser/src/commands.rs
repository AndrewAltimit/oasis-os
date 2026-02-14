//! Terminal commands for the browser subsystem.

use oasis_terminal::{Command, CommandOutput, CommandRegistry, Environment};
use oasis_types::error::Result;

/// Register all browser commands into a registry.
pub fn register_browser_commands(reg: &mut CommandRegistry) {
    reg.register(Box::new(BrowseCmd));
    reg.register(Box::new(FetchCmd));
    reg.register(Box::new(GeminiCmd));
    reg.register(Box::new(CurlCmd));
    reg.register(Box::new(SandboxCmd));
}

// -------------------------------------------------------------------
// browse
// -------------------------------------------------------------------
struct BrowseCmd;

impl Command for BrowseCmd {
    fn name(&self) -> &str {
        "browse"
    }

    fn description(&self) -> &str {
        "Open URL in browser or manage browser state"
    }

    fn usage(&self) -> &str {
        "browse <url> | browse bookmarks | browse history | browse home | \
         browse back | browse forward | browse reader | browse sandbox <url>"
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Ok(CommandOutput::Text(
                "Usage: browse <url>\n\
                 Subcommands: bookmarks, history, home, back, forward, \
                 reader, sandbox <url>"
                    .to_string(),
            ));
        }

        match args[0] {
            "bookmarks" => Ok(CommandOutput::Text(
                "[browser] Opening bookmarks page...".to_string(),
            )),
            "history" => Ok(CommandOutput::Text(
                "[browser] Opening history page...".to_string(),
            )),
            "home" => Ok(CommandOutput::Text(
                "[browser] Navigating to home page...".to_string(),
            )),
            "back" => Ok(CommandOutput::Text(
                "[browser] Navigating back...".to_string(),
            )),
            "forward" => Ok(CommandOutput::Text(
                "[browser] Navigating forward...".to_string(),
            )),
            "reader" => Ok(CommandOutput::Text(
                "[browser] Toggling reader mode...".to_string(),
            )),
            "sandbox" => {
                if args.len() < 2 {
                    return Ok(CommandOutput::Text(
                        "Usage: browse sandbox <url>".to_string(),
                    ));
                }
                Ok(CommandOutput::Text(format!(
                    "[browser] Opening {} in sandbox mode...",
                    args[1]
                )))
            },
            url => Ok(CommandOutput::Text(format!("[browser] Opening {}...", url))),
        }
    }
}

// -------------------------------------------------------------------
// fetch
// -------------------------------------------------------------------
struct FetchCmd;

impl Command for FetchCmd {
    fn name(&self) -> &str {
        "fetch"
    }

    fn description(&self) -> &str {
        "Fetch URL and print raw response to terminal"
    }

    fn usage(&self) -> &str {
        "fetch <url> | fetch headers <url>"
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Ok(CommandOutput::Text(
                "Usage: fetch <url> | fetch headers <url>".to_string(),
            ));
        }

        let (show_headers, url) = if args[0] == "headers" {
            if args.len() < 2 {
                return Ok(CommandOutput::Text(
                    "Usage: fetch headers <url>".to_string(),
                ));
            }
            (true, args[1])
        } else {
            (false, args[0])
        };

        // Try to load from VFS first.
        let vfs_path = url_to_vfs_path(url);
        if env.vfs.exists(&vfs_path) {
            match env.vfs.read(&vfs_path) {
                Ok(data) => {
                    if show_headers {
                        return Ok(CommandOutput::Text(format!(
                            "VFS-Path: {}\n\
                             Content-Length: {}\n\
                             Content-Type: {}",
                            vfs_path,
                            data.len(),
                            guess_content_type(&vfs_path),
                        )));
                    }
                    let text = String::from_utf8_lossy(&data);
                    // Truncate very long responses.
                    let display = if text.len() > 4096 {
                        format!(
                            "{}...\n[truncated, {} bytes total]",
                            &text[..text.floor_char_boundary(4096)],
                            text.len()
                        )
                    } else {
                        text.to_string()
                    };
                    return Ok(CommandOutput::Text(display));
                },
                Err(e) => {
                    return Ok(CommandOutput::Text(format!(
                        "Error reading {}: {}",
                        vfs_path, e
                    )));
                },
            }
        }

        // VFS miss -- try HTTP(S) for http:// and https:// URLs.
        if (url.starts_with("http://") || url.starts_with("https://"))
            && let Some(parsed) = super::loader::Url::parse(url)
        {
            match super::loader::http::http_get(&parsed, env.tls) {
                Ok(resp) => {
                    let text = String::from_utf8_lossy(&resp.body);
                    if show_headers {
                        let ct = format!("{:?}", resp.content_type);
                        return Ok(CommandOutput::Text(format!(
                            "HTTP-Status: {}\n\
                             Content-Length: {}\n\
                             Content-Type: {}",
                            resp.status,
                            resp.body.len(),
                            ct,
                        )));
                    }
                    let display = if text.len() > 4096 {
                        format!(
                            "{}...\n[truncated, {} bytes total]",
                            &text[..text.floor_char_boundary(4096)],
                            text.len(),
                        )
                    } else {
                        text.to_string()
                    };
                    return Ok(CommandOutput::Text(display));
                },
                Err(e) => {
                    return Ok(CommandOutput::Text(format!("[fetch] HTTP error: {e}")));
                },
            }
        }

        Ok(CommandOutput::Text(format!(
            "[fetch] {} not found in VFS. \
             Network fetching requires an http:// or https:// URL.",
            url,
        )))
    }
}

/// Convert URL to a VFS path for the fetch command.
fn url_to_vfs_path(url: &str) -> String {
    // Strip protocol.
    let path = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .or_else(|| url.strip_prefix("vfs://"))
        .unwrap_or(url);

    if url.starts_with("vfs://") {
        format!("/{}", path)
    } else {
        // Map host/path to /sites/host/path.
        let mut result = format!("/sites/{}", path);
        // Check whether the URL path (after the hostname) ends with
        // a filename.  Split host from path-portion at the first `/`.
        let url_path_part = path.find('/').map(|i| &path[i..]).unwrap_or("");
        let has_file = url_path_part.rsplit('/').next().unwrap_or("").contains('.');
        if result.ends_with('/') || !has_file {
            if !result.ends_with('/') {
                result.push('/');
            }
            result.push_str("index.html");
        }
        result
    }
}

/// Guess content type from file path.
fn guess_content_type(path: &str) -> &'static str {
    if let Some(ext) = path.rsplit('.').next() {
        match ext {
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" => "application/javascript",
            "json" => "application/json",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "txt" => "text/plain",
            "xml" => "application/xml",
            _ => "application/octet-stream",
        }
    } else {
        "application/octet-stream"
    }
}

// -------------------------------------------------------------------
// gemini
// -------------------------------------------------------------------
struct GeminiCmd;

impl Command for GeminiCmd {
    fn name(&self) -> &str {
        "gemini"
    }

    fn description(&self) -> &str {
        "Open a Gemini URL in the browser"
    }

    fn usage(&self) -> &str {
        "gemini <url>"
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Ok(CommandOutput::Text(
                "Usage: gemini <url>\n\
                 Example: gemini gemini://geminispace.info/"
                    .to_string(),
            ));
        }

        let url = args[0];
        let url = if url.starts_with("gemini://") {
            url.to_string()
        } else {
            format!("gemini://{}", url)
        };

        Ok(CommandOutput::Text(format!(
            "[browser] Opening Gemini page: {}...",
            url
        )))
    }
}

// -------------------------------------------------------------------
// curl (alias for fetch)
// -------------------------------------------------------------------
struct CurlCmd;

impl Command for CurlCmd {
    fn name(&self) -> &str {
        "curl"
    }

    fn description(&self) -> &str {
        "Fetch URL and print raw response (alias for fetch)"
    }

    fn usage(&self) -> &str {
        "curl <url>"
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        FetchCmd.execute(args, env)
    }
}

// -------------------------------------------------------------------
// sandbox
// -------------------------------------------------------------------
struct SandboxCmd;

impl Command for SandboxCmd {
    fn name(&self) -> &str {
        "sandbox"
    }

    fn description(&self) -> &str {
        "Toggle browser sandbox mode (block/allow network requests)"
    }

    fn usage(&self) -> &str {
        "sandbox on | sandbox off"
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        match args.first().copied() {
            Some("on") => Ok(CommandOutput::BrowserSandbox { enable: true }),
            Some("off") => Ok(CommandOutput::BrowserSandbox { enable: false }),
            _ => Ok(CommandOutput::Text(
                "Usage: sandbox on | sandbox off\n\
                 sandbox on  — block all network requests (VFS only)\n\
                 sandbox off — allow HTTP network requests"
                    .to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_terminal::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_browser_commands(&mut reg);
        (reg, MemoryVfs::new())
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
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn browse_no_args_shows_usage() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Usage"));
                assert!(s.contains("browse"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_url_returns_opening_message() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse https://example.com").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Opening"));
                assert!(s.contains("https://example.com"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_bookmarks() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse bookmarks").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("bookmarks"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_history() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse history").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("history"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_home() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse home").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("home"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_back() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse back").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("back"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_forward() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse forward").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("forward"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_reader() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse reader").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("reader"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn browse_sandbox_with_url() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "browse sandbox https://untrusted.example").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("sandbox"));
                assert!(s.contains("https://untrusted.example"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fetch_no_args_shows_usage() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "fetch").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Usage"));
                assert!(s.contains("fetch"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fetch_headers_subcommand() {
        let (reg, mut vfs) = setup();
        vfs.mkdir("/sites").unwrap();
        vfs.mkdir("/sites/example.com").unwrap();
        vfs.write("/sites/example.com/index.html", b"<html>hello</html>")
            .unwrap();
        match exec(&reg, &mut vfs, "fetch headers https://example.com").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Content-Length:"));
                assert!(s.contains("Content-Type: text/html"));
                assert!(s.contains("VFS-Path:"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn url_to_vfs_path_mapping() {
        assert_eq!(
            url_to_vfs_path("https://example.com/page.html"),
            "/sites/example.com/page.html"
        );
        assert_eq!(
            url_to_vfs_path("http://example.com/"),
            "/sites/example.com/index.html"
        );
        assert_eq!(url_to_vfs_path("vfs://data/file.txt"), "/data/file.txt");
        // No protocol -- treated as host path.
        assert_eq!(
            url_to_vfs_path("example.com"),
            "/sites/example.com/index.html"
        );
    }

    #[test]
    fn gemini_url_auto_prefix() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "gemini geminispace.info").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("gemini://geminispace.info"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn gemini_url_already_has_prefix() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "gemini gemini://geminispace.info/").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("gemini://geminispace.info/"));
                // Should NOT double-prefix.
                assert!(!s.contains("gemini://gemini://"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn curl_delegates_to_fetch() {
        let (reg, mut vfs) = setup();
        // curl with no args should show fetch usage.
        match exec(&reg, &mut vfs, "curl").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Usage"));
                assert!(s.contains("fetch"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fetch_reads_from_vfs() {
        let (reg, mut vfs) = setup();
        vfs.mkdir("/sites").unwrap();
        vfs.mkdir("/sites/example.com").unwrap();
        vfs.write("/sites/example.com/page.html", b"<html>content</html>")
            .unwrap();
        match exec(&reg, &mut vfs, "fetch https://example.com/page.html").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("<html>content</html>"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fetch_url_not_in_vfs() {
        let (reg, mut vfs) = setup();
        // A non-HTTP(S) URL that is not in the VFS triggers the fallback message.
        match exec(&reg, &mut vfs, "fetch gopher://missing.example.com").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("not found in VFS"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fetch_https_without_tls_shows_error_page() {
        let (reg, mut vfs) = setup();
        // With tls: None, HTTPS URLs return the "HTTPS Required" error page.
        match exec(&reg, &mut vfs, "fetch https://missing.example.com").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("HTTPS Required"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn guess_content_type_known_extensions() {
        assert_eq!(guess_content_type("/foo/bar.html"), "text/html");
        assert_eq!(guess_content_type("/foo/bar.css"), "text/css");
        assert_eq!(guess_content_type("/foo/bar.js"), "application/javascript");
        assert_eq!(guess_content_type("/foo/bar.json"), "application/json");
        assert_eq!(guess_content_type("/foo/bar.png"), "image/png");
        assert_eq!(guess_content_type("/foo/bar.jpg"), "image/jpeg");
        assert_eq!(guess_content_type("/foo/bar.gif"), "image/gif");
        assert_eq!(guess_content_type("/foo/bar.bmp"), "image/bmp");
        assert_eq!(guess_content_type("/foo/bar.txt"), "text/plain");
    }

    #[test]
    fn guess_content_type_unknown_extension() {
        assert_eq!(
            guess_content_type("/foo/bar.xyz"),
            "application/octet-stream"
        );
    }

    #[test]
    fn sandbox_on() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "sandbox on").unwrap() {
            CommandOutput::BrowserSandbox { enable } => assert!(enable),
            _ => panic!("expected BrowserSandbox"),
        }
    }

    #[test]
    fn sandbox_off() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "sandbox off").unwrap() {
            CommandOutput::BrowserSandbox { enable } => assert!(!enable),
            _ => panic!("expected BrowserSandbox"),
        }
    }

    #[test]
    fn sandbox_no_args_shows_usage() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "sandbox").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Usage"));
                assert!(s.contains("sandbox"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fetch_gemini_url_not_supported() {
        let (reg, mut vfs) = setup();
        // fetch does not handle gemini:// URLs -- they fall through
        // to the "not found in VFS" message.
        match exec(&reg, &mut vfs, "fetch gemini://geminispace.info/").unwrap() {
            CommandOutput::Text(s) => {
                assert!(
                    s.contains("not found in VFS"),
                    "expected VFS fallback message for gemini URL, got: {s}",
                );
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn curl_https_without_tls_shows_error_page() {
        let (reg, mut vfs) = setup();
        // curl delegates to fetch, which calls http_get with tls: None.
        match exec(&reg, &mut vfs, "curl https://missing.example.com").unwrap() {
            CommandOutput::Text(s) => {
                assert!(
                    s.contains("HTTPS Required"),
                    "expected HTTPS Required page from curl, got: {s}",
                );
            },
            _ => panic!("expected text"),
        }
    }
}
