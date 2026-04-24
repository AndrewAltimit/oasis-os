use oasis_core::vfs::MemoryVfs;

// Bundled sample media — shipped in the binary so every fresh VFS has real
// content in My Music / My Pictures / My Documents without needing to run
// `samples/fetch-samples.sh`. Keep these small (we embed them into the
// binary); larger samples can still be dropped into the on-disk `samples/`
// tree and loaded via `spawn_disk_sample_loader`.
const SAMPLE_WELCOME_TXT: &[u8] = include_bytes!("../assets/samples/welcome.txt");
const SAMPLE_NOTES_TXT: &[u8] = include_bytes!("../assets/samples/notes.txt");
const SAMPLE_HELLO_SH: &[u8] = include_bytes!("../assets/samples/hello.sh");
const SAMPLE_CHIME_WAV: &[u8] = include_bytes!("../assets/samples/oasis_chime.wav");
const SAMPLE_AMBIENT_MP3: &[u8] = include_bytes!("../assets/samples/ambient_dawn.mp3");
const SAMPLE_SUNSET_PNG: &[u8] = include_bytes!("../assets/samples/oasis_sample.png");

/// Create demo VFS content including fake apps.
pub fn populate_demo_vfs(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;

    vfs.mkdir("/home").expect("vfs mkdir /home");
    vfs.mkdir("/home/user").expect("vfs mkdir /home/user");
    vfs.mkdir("/etc").expect("vfs mkdir /etc");
    vfs.mkdir("/tmp").expect("vfs mkdir /tmp");
    vfs.write("/home/user/readme.txt", SAMPLE_WELCOME_TXT)
        .expect("vfs write /home/user/readme.txt");
    vfs.write("/etc/hostname", b"oasis")
        .expect("vfs write /etc/hostname");
    vfs.write("/etc/version", b"0.1.0")
        .expect("vfs write /etc/version");
    vfs.write(
        "/etc/hosts.toml",
        b"[[host]]\nname = \"briefcase\"\naddress = \"192.168.0.50\"\nport = 9000\nprotocol = \"oasis-terminal\"\n",
    )
    .expect("vfs write /etc/hosts.toml");

    vfs.mkdir("/apps").expect("vfs mkdir /apps");
    for name in &[
        "File Manager",
        "Settings",
        "Network",
        "Terminal",
        "Music Player",
        "Internet Radio",
        "Photo Viewer",
        "Package Manager",
        "System Monitor",
        "Browser",
        "TV Guide",
    ] {
        vfs.mkdir(&format!("/apps/{name}"))
            .expect("vfs mkdir app dir");
    }

    // Radio configuration directory and default station list.
    vfs.mkdir("/etc/radio").expect("vfs mkdir /etc/radio");
    vfs.mkdir("/var/radio").expect("vfs mkdir /var/radio");
    let default_stations = oasis_audio::radio::station::StationRegistry::defaults();
    if let Ok(toml_data) = default_stations.to_toml() {
        vfs.write("/etc/radio/stations.toml", toml_data.as_bytes())
            .expect("vfs write /etc/radio/stations.toml");
    }

    // TV Guide configuration directory and default channel list.
    vfs.mkdir("/etc/tv").expect("vfs mkdir /etc/tv");
    vfs.mkdir("/var/tv").expect("vfs mkdir /var/tv");
    vfs.mkdir("/var/tv/cache").expect("vfs mkdir /var/tv/cache");
    vfs.write(
        "/etc/tv/channels.toml",
        oasis_core::apps::tv_guide::channel::DEFAULT_CHANNELS_TOML.as_bytes(),
    )
    .expect("vfs write /etc/tv/channels.toml");

    // Browser home page content.
    vfs.mkdir("/sites").expect("vfs mkdir /sites");
    vfs.mkdir("/sites/home").expect("vfs mkdir /sites/home");
    vfs.write(
        "/sites/home/index.html",
        br#"<html><head><title>OASIS Home</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
h2 { color: #80d0a0; }
a { color: #64c8ff; }
code { background-color: rgba(100,200,255,30); }
pre { background-color: rgba(100,200,255,15); border: 1px solid rgba(100,200,255,30); }
blockquote { border-left-color: #64c8ff; color: #a0a0c0; }
table { border-collapse: collapse; }
th { background-color: rgba(100,200,255,20); border: 1px solid rgba(255,255,255,30); }
td { border: 1px solid rgba(255,255,255,20); }
</style>
</head><body>
<h1>Welcome to OASIS Browser</h1>
<p>A lightweight <strong>HTML/CSS</strong> rendering engine for
<em>OASIS_OS</em>. Supports block, inline, flex, and table layout.</p>

<h2>Features</h2>
<ul>
<li>CSS cascade with <code>specificity</code></li>
<li>Block, inline, flex, and table layout</li>
<li>Text wrapping and decoration</li>
<li>Smooth scrolling with mouse wheel</li>
</ul>

<h2>Shortcuts</h2>
<table>
<tr><th>Key</th><th>Action</th></tr>
<tr><td>Tab</td><td>Focus URL bar</td></tr>
<tr><td>Left/Right</td><td>Navigate links</td></tr>
<tr><td>Up/Down</td><td>Scroll page</td></tr>
</table>

<blockquote>Built from scratch in Rust (2026), inspired by PSP homebrew shells like PSIX.</blockquote>

<h2>Real-world test pages</h2>
<p>Sites we're actively tuning the engine against &mdash; one click to open:</p>
<ul>
<li><a href="https://old.reddit.com/">old.reddit.com</a> &mdash; float sidebars, sprite votes, nested comments</li>
<li><a href="https://en.wikipedia.org/wiki/Main_Page">wikipedia.org</a> &mdash; infoboxes, @media queries, 62.5% rem baseline</li>
<li><a href="https://www.google.com/">google.com</a> &mdash; search form, centered layout</li>
</ul>

<h2>Internal test pages</h2>
<ol>
<li><a href="about.html">About OASIS Browser</a></li>
<li><a href="features.html">CSS Feature Test</a></li>
<li><a href="js-test.html">JavaScript DOM Test</a></li>
</ol>
</body></html>"#,
    )
    .expect("vfs write /sites/home/index.html");
    vfs.write(
        "/sites/home/about.html",
        br#"<html><head><title>About OASIS Browser</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
a { color: #64c8ff; }
</style>
</head><body>
<h1>About OASIS Browser</h1>
<p>A lightweight HTML/CSS engine for embedded systems:</p>
<ul>
<li><strong>HTML</strong> -- WHATWG tokenizer, 70+ tags</li>
<li><strong>CSS</strong> -- cascade, specificity, media queries</li>
<li><strong>Layout</strong> -- block, inline, flex, table, float</li>
<li><strong>Gemini</strong> -- lightweight text protocol</li>
</ul>
<p><a href="index.html">Back to home</a></p>
</body></html>"#,
    )
    .expect("vfs write /sites/home/about.html");
    vfs.write(
        "/sites/home/features.html",
        br#"<html><head><title>CSS Features</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
h2 { color: #80d0a0; font-size: 1.2em; }
a { color: #64c8ff; }
</style>
</head><body>
<h1>CSS Feature Test</h1>
<h2>Text Formatting</h2>
<p><strong>Bold</strong>, <em>italic</em>, <u>underline</u>,
<s>strikethrough</s>, <code>inline code</code>,
<mark>highlighted</mark>, <small>small</small>.</p>
<h2>Blockquote</h2>
<blockquote>Blockquote with left border.</blockquote>
<h2>Ordered List</h2>
<ol><li>First</li><li>Second</li><li>Third</li></ol>
<h2>Preformatted</h2>
<pre>fn main() {
    println!("Hello!");
}</pre>
<p><a href="index.html">Back to home</a></p>
</body></html>"#,
    )
    .expect("vfs write /sites/home/features.html");

    // JavaScript DOM manipulation test page.
    vfs.write(
        "/sites/home/js-test.html",
        br#"<html><head><title>JS DOM Test</title>
<style>
body { color: #e0e0e0; background-color: #1a1a2e; }
h1 { color: #64c8ff; }
h2 { color: #80d0a0; }
a { color: #64c8ff; }
.pass { color: #80ff80; }
.fail { color: #ff8080; }
</style>
</head><body>
<h1>JavaScript DOM Test</h1>
<div id="output"></div>
<div id="created"></div>
<p><a href="index.html">Back to home</a></p>
<script>
var out = document.getElementById("output");
var results = [];
function test(name, ok) { results.push((ok ? "PASS" : "FAIL") + ": " + name); }

// Test 1: getElementById
test("getElementById finds element", out !== null);

// Test 2: tagName
test("tagName is DIV", out.tagName === "DIV");

// Test 3: textContent set
out.textContent = "Tests running...";
test("textContent set works", out.textContent === "Tests running...");

// Test 4: setAttribute / getAttribute
out.setAttribute("data-count", "42");
test("setAttribute/getAttribute", out.getAttribute("data-count") === "42");

// Test 5: removeAttribute
out.removeAttribute("data-count");
test("removeAttribute", out.getAttribute("data-count") === null);

// Test 6: id property
out.id = "results";
test("id property set", document.getElementById("results") !== null);
out.id = "output";

// Test 7: createElement + appendChild
var created = document.getElementById("created");
var span = document.createElement("span");
span.textContent = "Dynamic element!";
created.appendChild(span);
test("createElement+appendChild", created.textContent.indexOf("Dynamic") >= 0);

// Test 8: createTextNode
var t = document.createTextNode(" And text node!");
created.appendChild(t);
test("createTextNode+appendChild", created.textContent.indexOf("text node") >= 0);

// Test 9: document.title get
test("document.title get", document.title === "JS DOM Test");

// Test 10: document.title set
document.title = "Tests Complete";
test("document.title set", document.title === "Tests Complete");

// Test 11: document.body
test("document.body exists", document.body !== null);

// Test 12: children
test("body has children", document.body.children.length > 0);

// Render results
var html = "";
var pass = 0;
for (var i = 0; i < results.length; i++) {
  var r = results[i];
  if (r.indexOf("PASS") === 0) pass++;
  html = html + r + "\n";
}
html = pass + "/" + results.length + " tests passed\n\n" + html;
out.textContent = html;
</script>
</body></html>"#,
    )
    .expect("vfs write /sites/home/js-test.html");

    // Add JS test link to home page navigation is handled by the existing
    // home page (users can navigate via URL bar to /sites/home/js-test.html).

    vfs.mkdir("/home/user/music")
        .expect("vfs mkdir /home/user/music");
    vfs.mkdir("/home/user/photos")
        .expect("vfs mkdir /home/user/photos");
    vfs.mkdir("/home/user/documents")
        .expect("vfs mkdir /home/user/documents");

    write_bundled_samples(vfs);

    vfs.mkdir("/home/user/scripts")
        .expect("vfs mkdir /home/user/scripts");
    vfs.write("/home/user/scripts/hello.sh", SAMPLE_HELLO_SH)
        .expect("vfs write /home/user/scripts/hello.sh");

    vfs.mkdir("/var").expect("vfs mkdir /var");
    vfs.mkdir("/var/audio").expect("vfs mkdir /var/audio");
    vfs.mkdir("/var/app").expect("vfs mkdir /var/app");
}

/// Install the bundled-in-binary sample files so a fresh VFS has
/// working media for Photo Viewer, Music Player, and Text Editor.
fn write_bundled_samples(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;

    vfs.write("/home/user/music/ambient_dawn.mp3", SAMPLE_AMBIENT_MP3)
        .expect("vfs write /home/user/music/ambient_dawn.mp3");
    vfs.write("/home/user/music/oasis_chime.wav", SAMPLE_CHIME_WAV)
        .expect("vfs write /home/user/music/oasis_chime.wav");
    vfs.write("/home/user/photos/oasis_sample.png", SAMPLE_SUNSET_PNG)
        .expect("vfs write /home/user/photos/oasis_sample.png");
    vfs.write("/home/user/documents/notes.txt", SAMPLE_NOTES_TXT)
        .expect("vfs write /home/user/documents/notes.txt");
    vfs.write("/home/user/documents/welcome.txt", SAMPLE_WELCOME_TXT)
        .expect("vfs write /home/user/documents/welcome.txt");
}

/// Spawn a background thread that reads real sample files from disk.
///
/// Returns a receiver that yields `(vfs_path, data)` pairs as they are read.
/// The main loop should poll this with `try_recv()` and write results to the VFS.
pub fn spawn_disk_sample_loader() -> std::sync::mpsc::Receiver<(String, Vec<u8>)> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        use std::path::Path;

        // Optional on-disk samples (not shipped in the binary): fetched by
        // `samples/fetch-samples.sh`. The bundled samples in
        // `populate_demo_vfs` are already in the VFS; anything found here
        // will replace or augment them.
        let samples_dir = Path::new("samples");

        let music_files = ["ambient_dawn.mp3", "nightfall_theme.mp3"];
        for name in &music_files {
            let disk_path = samples_dir.join(name);
            if let Ok(data) = std::fs::read(&disk_path) {
                let vfs_path = format!("/home/user/music/{name}");
                log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
                if tx.send((vfs_path, data)).is_err() {
                    return;
                }
            }
        }

        let photo_files = ["sample_landscape.png"];
        for name in &photo_files {
            let disk_path = samples_dir.join(name);
            if let Ok(data) = std::fs::read(&disk_path) {
                let vfs_path = format!("/home/user/photos/{name}");
                log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
                if tx.send((vfs_path, data)).is_err() {
                    return;
                }
            }
        }

        load_disk_dir_to_channel(&tx, &samples_dir.join("music"), "/home/user/music");
        load_disk_dir_to_channel(&tx, &samples_dir.join("photos"), "/home/user/photos");

        log::info!("Disk samples loaded");
    });

    rx
}

/// Load all files from a real disk directory and send via channel.
fn load_disk_dir_to_channel(
    tx: &std::sync::mpsc::Sender<(String, Vec<u8>)>,
    disk_dir: &std::path::Path,
    vfs_dir: &str,
) {
    let Ok(entries) = std::fs::read_dir(disk_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
            && let Ok(data) = std::fs::read(&path)
        {
            let vfs_path = format!("{vfs_dir}/{name}");
            log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
            if tx.send((vfs_path, data)).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use oasis_core::vfs::{MemoryVfs, Vfs};

    #[test]
    fn populate_creates_home_user() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(vfs.readdir("/home/user").is_ok(), "/home/user should exist");
    }

    #[test]
    fn populate_creates_etc_hostname() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/etc/hostname")
            .expect("/etc/hostname should exist");
        assert_eq!(data, b"oasis");
    }

    #[test]
    fn populate_creates_etc_version() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs.read("/etc/version").expect("/etc/version should exist");
        assert_eq!(data, b"0.1.0");
    }

    #[test]
    fn populate_creates_all_app_dirs() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let expected = [
            "File Manager",
            "Settings",
            "Network",
            "Terminal",
            "Music Player",
            "Internet Radio",
            "Photo Viewer",
            "Package Manager",
            "System Monitor",
            "Browser",
            "TV Guide",
        ];
        for name in &expected {
            let path = format!("/apps/{name}");
            assert!(vfs.readdir(&path).is_ok(), "app dir should exist: {path}",);
        }
    }

    #[test]
    fn populate_creates_browser_home() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/sites/home/index.html")
            .expect("/sites/home/index.html should exist");
        let text = std::str::from_utf8(&data).unwrap();
        assert!(
            text.contains("OASIS Browser"),
            "index.html should contain 'OASIS Browser', got: {text}",
        );
    }

    #[test]
    fn populate_creates_music_dir() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(
            vfs.readdir("/home/user/music").is_ok(),
            "/home/user/music should exist",
        );
    }

    #[test]
    fn populate_creates_photos_dir() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(
            vfs.readdir("/home/user/photos").is_ok(),
            "/home/user/photos should exist",
        );
    }

    #[test]
    fn populate_creates_scripts() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/home/user/scripts/hello.sh")
            .expect("/home/user/scripts/hello.sh should exist");
        let text = std::str::from_utf8(&data).unwrap();
        assert!(
            text.contains("echo"),
            "hello.sh should contain 'echo', got: {text}",
        );
    }

    #[test]
    fn populate_creates_radio_config() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/etc/radio/stations.toml")
            .expect("/etc/radio/stations.toml should exist");
        let text = std::str::from_utf8(&data).unwrap();
        assert!(
            text.contains("Old Time Radio"),
            "stations.toml should contain 'Old Time Radio', got: {text}",
        );
    }

    #[test]
    fn populate_creates_var_radio() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(vfs.readdir("/var/radio").is_ok(), "/var/radio should exist",);
    }

    #[test]
    fn populate_creates_var_audio() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        assert!(vfs.readdir("/var/audio").is_ok(), "/var/audio should exist",);
    }

    #[test]
    fn populate_creates_hosts_toml() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let data = vfs
            .read("/etc/hosts.toml")
            .expect("/etc/hosts.toml should exist");
        let text = std::str::from_utf8(&data).unwrap();
        assert!(
            text.contains("briefcase"),
            "hosts.toml should contain 'briefcase', got: {text}",
        );
    }

    #[test]
    fn bundled_music_is_real_audio() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let mp3 = vfs
            .read("/home/user/music/ambient_dawn.mp3")
            .expect("bundled mp3 should exist");
        // ID3v2 tag or MPEG frame sync — rules out the old placeholder
        // text content we used to write here.
        assert!(
            mp3.starts_with(b"ID3") || (mp3.len() >= 2 && mp3[0] == 0xFF && mp3[1] & 0xE0 == 0xE0),
            "ambient_dawn.mp3 should be real audio, got first bytes: {:?}",
            &mp3[..mp3.len().min(4)]
        );

        let wav = vfs
            .read("/home/user/music/oasis_chime.wav")
            .expect("bundled wav should exist");
        assert!(wav.starts_with(b"RIFF"), "oasis_chime.wav should be WAV");
        assert!(&wav[8..12] == b"WAVE");
    }

    #[test]
    fn bundled_photo_is_real_image() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let png = vfs
            .read("/home/user/photos/oasis_sample.png")
            .expect("bundled png should exist");
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n"),
            "oasis_sample.png should be real PNG",
        );
    }

    #[test]
    fn bundled_documents_are_readable_text() {
        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let welcome = vfs
            .read("/home/user/documents/welcome.txt")
            .expect("welcome.txt should exist");
        let text = std::str::from_utf8(&welcome).unwrap();
        assert!(text.contains("Welcome to OASIS_OS"));

        let notes = vfs
            .read("/home/user/documents/notes.txt")
            .expect("notes.txt should exist");
        let notes_text = std::str::from_utf8(&notes).unwrap();
        assert!(notes_text.contains("scratchpad"));
    }

    // ---------------------------------------------------------------
    // End-to-end dispatch tests against the real bundled VFS
    // ---------------------------------------------------------------

    /// Photo Viewer launches with the bundled sample pre-opened, and the
    /// metadata text names the actual PNG dimensions.
    #[test]
    fn bundled_png_opens_in_photo_viewer() {
        use oasis_app_media::BrowsingApp;
        use oasis_core::apps::App;

        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let app = BrowsingApp::photo_viewer_at(
            "/apps/Photo Viewer",
            "/home/user/photos/oasis_sample.png",
            &vfs,
        );
        assert_eq!(
            App::viewing_file(&app),
            Some("/home/user/photos/oasis_sample.png")
        );
        assert!(app.lines().iter().any(|l| l.contains("PNG")));
    }

    /// Music Player opens the bundled MP3 and emits a `play_file` IPC so
    /// `media_controller::tick` can kick off actual playback.
    #[test]
    fn bundled_mp3_opens_in_music_player_with_ipc() {
        use oasis_app_media::{BrowsingApp, MEDIA_REQUEST_PATH};

        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let mut app = BrowsingApp::music_player_at(
            "/apps/Music Player",
            "/home/user/music/ambient_dawn.mp3",
            &vfs,
        );
        let req = app
            .content
            .pending_vfs_request
            .take()
            .expect("music player should emit play_file IPC");
        assert_eq!(req.0, MEDIA_REQUEST_PATH);
        assert_eq!(req.1, "play_file /home/user/music/ambient_dawn.mp3");
    }

    /// Text Editor opens a bundled document — routed through the
    /// `AppRunner::launch_with_file` path to mirror what the real
    /// application does when File Manager dispatches the file.
    #[test]
    fn bundled_text_file_opens_in_text_editor() {
        use oasis_core::apps::AppRunner;
        use oasis_core::dashboard::AppEntry;

        let mut vfs = MemoryVfs::new();
        super::populate_demo_vfs(&mut vfs);
        let entry = AppEntry {
            title: "Text Editor".to_string(),
            path: "/apps/Text Editor".to_string(),
            icon_png: Vec::new(),
            color: oasis_core::backend::Color::rgb(100, 100, 100),
        };
        let runner = AppRunner::launch_with_file(&entry, "/home/user/documents/welcome.txt", &vfs);
        assert_eq!(
            runner.viewing_file.as_deref(),
            Some("/home/user/documents/welcome.txt")
        );
        // The text editor renders the file as display lines with line
        // numbers — the welcome content shows up in the rendered output.
        assert!(
            runner
                .lines
                .iter()
                .any(|l| l.contains("Welcome to OASIS_OS")),
            "editor should render welcome text, got: {:?}",
            runner.lines,
        );
    }
}
