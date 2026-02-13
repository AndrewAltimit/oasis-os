use oasis_core::vfs::MemoryVfs;

/// Create demo VFS content including fake apps.
pub fn populate_demo_vfs(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;

    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.mkdir("/etc").unwrap();
    vfs.mkdir("/tmp").unwrap();
    vfs.write(
        "/home/user/readme.txt",
        b"Welcome to OASIS_OS!\nType 'help' for available commands.",
    )
    .unwrap();
    vfs.write("/etc/hostname", b"oasis").unwrap();
    vfs.write("/etc/version", b"0.1.0").unwrap();
    vfs.write(
        "/etc/hosts.toml",
        b"[[host]]\nname = \"briefcase\"\naddress = \"192.168.0.50\"\nport = 9000\nprotocol = \"oasis-terminal\"\n",
    )
    .unwrap();

    vfs.mkdir("/apps").unwrap();
    for name in &[
        "File Manager",
        "Settings",
        "Network",
        "Terminal",
        "Music Player",
        "Photo Viewer",
        "Package Manager",
        "System Monitor",
        "Browser",
    ] {
        vfs.mkdir(&format!("/apps/{name}")).unwrap();
    }

    // Browser home page content.
    vfs.mkdir("/sites").unwrap();
    vfs.mkdir("/sites/home").unwrap();
    vfs.write(
        "/sites/home/index.html",
        b"<html><head><title>OASIS Home</title></head><body>\
          <h1>Welcome to OASIS Browser</h1>\
          <p>A lightweight HTML/CSS browser for OASIS_OS.</p>\
          <ul>\
          <li><a href=\"/sites/home/about.html\">About OASIS Browser</a></li>\
          </ul>\
          </body></html>",
    )
    .unwrap();
    vfs.write(
        "/sites/home/about.html",
        b"<html><head><title>About</title></head><body>\
          <h1>About OASIS Browser</h1>\
          <p>Supports HTML, CSS, and Gemini protocol.</p>\
          <p><a href=\"/sites/home/index.html\">Back to home</a></p>\
          </body></html>",
    )
    .unwrap();

    vfs.mkdir("/home/user/music").unwrap();
    vfs.mkdir("/home/user/photos").unwrap();

    load_disk_samples(vfs);

    vfs.mkdir("/home/user/scripts").unwrap();
    vfs.write(
        "/home/user/scripts/hello.sh",
        b"# Demo script\necho Hello from OASIS_OS!\nstatus\npwd\n",
    )
    .unwrap();

    vfs.mkdir("/var").unwrap();
    vfs.mkdir("/var/audio").unwrap();
}

/// Try to load real sample files from the `samples/` directory on disk.
fn load_disk_samples(vfs: &mut MemoryVfs) {
    use oasis_core::vfs::Vfs;
    use std::path::Path;

    let samples_dir = Path::new("samples");

    let music_files = ["ambient_dawn.mp3", "nightfall_theme.mp3"];
    for name in &music_files {
        let disk_path = samples_dir.join(name);
        let vfs_path = format!("/home/user/music/{name}");
        if disk_path.exists()
            && let Ok(data) = std::fs::read(&disk_path)
        {
            log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
            vfs.write(&vfs_path, &data).unwrap();
            continue;
        }
        vfs.write(
            &vfs_path,
            format!("(placeholder: run samples/fetch-samples.sh for real audio)\nFile: {name}\n")
                .as_bytes(),
        )
        .unwrap();
    }

    let photo_files = ["sample_landscape.png"];
    for name in &photo_files {
        let disk_path = samples_dir.join(name);
        let vfs_path = format!("/home/user/photos/{name}");
        if disk_path.exists()
            && let Ok(data) = std::fs::read(&disk_path)
        {
            log::info!("Loaded from disk: {vfs_path} ({} bytes)", data.len());
            vfs.write(&vfs_path, &data).unwrap();
            continue;
        }
        vfs.write(
            &vfs_path,
            format!("(placeholder: run samples/fetch-samples.sh for real image)\nFile: {name}\n")
                .as_bytes(),
        )
        .unwrap();
    }

    load_disk_dir(vfs, &samples_dir.join("music"), "/home/user/music");
    load_disk_dir(vfs, &samples_dir.join("photos"), "/home/user/photos");
}

/// Load all files from a real disk directory into the VFS.
fn load_disk_dir(vfs: &mut MemoryVfs, disk_dir: &std::path::Path, vfs_dir: &str) {
    use oasis_core::vfs::Vfs;

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
            vfs.write(&vfs_path, &data).unwrap();
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
            "Photo Viewer",
            "Package Manager",
            "System Monitor",
            "Browser",
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
}
