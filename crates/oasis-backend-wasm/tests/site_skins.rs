//! The site's skin list has one source, `site/data/skins.json`; the three
//! pages that list skins carry generated copies between
//! `<!-- BEGIN skins:<region> ... -->` / `<!-- END skins:<region> -->`
//! markers so they work as plain static files (no build step, no fetch,
//! fine from `file://`):
//!
//! - `site/index.html`: the gallery filter chips (`skins:chips`) and the
//!   gallery list (`skins:gallery`);
//! - `site/demo/index.html`, `www/index.html`: the demo skin picker
//!   (`skins:options`).
//!
//! This test regenerates every region and fails if a page is out of step,
//! if `skins.json` misses or invents a skin under `skins/`, or if a
//! skin's screenshot no longer matches its `skin.toml` resolution (the
//! gallery sizes its `<img>` from it). `scripts/gen-skin-list.sh` runs it
//! with `OASIS_UPDATE_SITE_SKINS=1`, which rewrites the regions instead.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize)]
struct SkinList {
    categories: Vec<Category>,
    skins: Vec<SkinEntry>,
}

#[derive(Deserialize)]
struct Category {
    id: String,
    /// `<optgroup>` label in the demo picker.
    label: String,
    /// Filter-chip label in the gallery.
    short: String,
}

#[derive(Deserialize)]
struct SkinEntry {
    id: String,
    name: String,
    category: String,
    blurb: String,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Attribute / text escaping for the generated HTML.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Width and height from a PNG's IHDR chunk.
fn png_size(path: &Path) -> (u32, u32) {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        data.len() >= 24 && &data[1..4] == b"PNG",
        "{} is not a PNG",
        path.display()
    );
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    (be(&data[16..20]), be(&data[20..24]))
}

fn load(root: &Path) -> SkinList {
    let path = root.join("site/data/skins.json");
    let text = std::fs::read_to_string(&path).expect("site/data/skins.json");
    serde_json::from_str(&text).expect("valid skins.json")
}

/// Resolution of every listed skin, from its `skin.toml`, checked against
/// its gallery screenshot.
fn resolutions(root: &Path, list: &SkinList) -> Vec<(u32, u32)> {
    list.skins
        .iter()
        .map(|s| {
            let skin =
                oasis_skin::resolve_skin(&s.id).unwrap_or_else(|e| panic!("skin {}: {e}", s.id));
            let res = (skin.manifest.screen_width, skin.manifest.screen_height);
            let shot = root.join(format!("screenshots/{}/01_dashboard.png", s.id));
            assert_eq!(
                png_size(&shot),
                res,
                "{} does not match skins/{}/skin.toml (regenerate the screenshots)",
                shot.display(),
                s.id
            );
            res
        })
        .collect()
}

fn chips(list: &SkinList, indent: &str) -> String {
    let mut out = format!(
        "{indent}<button type=\"button\" class=\"chip is-active\" data-filter=\"all\" \
         aria-pressed=\"true\">All <span>{}</span></button>\n",
        list.skins.len()
    );
    for c in &list.categories {
        let n = list.skins.iter().filter(|s| s.category == c.id).count();
        let _ = writeln!(
            out,
            "{indent}<button type=\"button\" class=\"chip\" data-filter=\"{}\" \
             aria-pressed=\"false\">{} <span>{n}</span></button>",
            esc(&c.id),
            esc(&c.short)
        );
    }
    out
}

fn gallery(list: &SkinList, res: &[(u32, u32)], indent: &str) -> String {
    let mut out = format!(
        "{indent}<!-- data-res is the real PNG size of screenshots/<id>/01_dashboard.png -->\n"
    );
    for (i, (s, (w, h))) in list.skins.iter().zip(res).enumerate() {
        let (class, selected) = if i == 0 {
            ("skin is-active", "true")
        } else {
            ("skin", "false")
        };
        let (id, name) = (esc(&s.id), esc(&s.name));
        let _ = writeln!(
            out,
            "{indent}<div role=\"option\" class=\"{class}\" id=\"skin-{id}\" \
             aria-selected=\"{selected}\" data-skin=\"{id}\" data-cat=\"{}\" data-name=\"{name}\" \
             data-res=\"{w}&times;{h}\" data-src=\"skins/{id}\" data-desc=\"{}\">",
            esc(&s.category),
            esc(&s.blurb)
        );
        let _ = writeln!(
            out,
            "{indent}  <img src=\"screenshots/{id}/01_dashboard.png\" alt=\"\" loading=\"lazy\" \
             width=\"{w}\" height=\"{h}\"><span>{name}</span>"
        );
        let _ = writeln!(out, "{indent}</div>");
    }
    out
}

fn options(list: &SkinList, indent: &str) -> String {
    let mut out = String::new();
    for c in &list.categories {
        let _ = writeln!(out, "{indent}<optgroup label=\"{}\">", esc(&c.label));
        for s in list.skins.iter().filter(|s| s.category == c.id) {
            let _ = writeln!(
                out,
                "{indent}  <option value=\"{}\">{}</option>",
                esc(&s.id),
                esc(&s.name)
            );
        }
        let _ = writeln!(out, "{indent}</optgroup>");
    }
    out
}

/// Replace region `name` of `page` with `generate(indent)`, where `indent`
/// is the BEGIN marker's indentation. Returns the new page.
fn regenerate(page: &str, name: &str, generate: impl Fn(&str) -> String) -> String {
    let begin = format!("<!-- BEGIN skins:{name} ");
    let end = format!("<!-- END skins:{name} -->");
    let b = page
        .find(&begin)
        .unwrap_or_else(|| panic!("no {begin} marker"));
    let line_start = page[..b].rfind('\n').map_or(0, |i| i + 1);
    let indent = &page[line_start..b];
    let body_start = b + page[b..].find('\n').expect("marker line ends") + 1;
    let e = page[body_start..]
        .find(&end)
        .unwrap_or_else(|| panic!("no {end} marker"))
        + body_start;
    let body_end = page[..e].rfind('\n').map_or(0, |i| i + 1);
    format!(
        "{}{}{}",
        &page[..body_start],
        generate(indent),
        &page[body_end..]
    )
}

#[test]
fn site_skin_lists_match_skins_json() {
    let root = repo_root();
    let list = load(&root);

    // Every skin under skins/ is listed exactly once, and nothing else.
    let mut on_disk: Vec<String> = std::fs::read_dir(root.join("skins"))
        .expect("skins/")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("skin.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = list.skins.iter().map(|s| s.id.clone()).collect();
    listed.sort();
    assert_eq!(listed, on_disk, "site/data/skins.json vs skins/*/skin.toml");
    for s in &list.skins {
        assert!(
            list.categories.iter().any(|c| c.id == s.category),
            "skin {} has unknown category {}",
            s.id,
            s.category
        );
    }

    let res = resolutions(&root, &list);
    let update = std::env::var_os("OASIS_UPDATE_SITE_SKINS").is_some();
    let mut stale = Vec::new();
    let pages: [(&str, &[&str]); 3] = [
        ("site/index.html", &["chips", "gallery"]),
        ("site/demo/index.html", &["options"]),
        ("www/index.html", &["options"]),
    ];
    for (rel, regions) in pages {
        let path = root.join(rel);
        let page = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let mut new = page.clone();
        for region in regions {
            new = match *region {
                "chips" => regenerate(&new, region, |i| chips(&list, i)),
                "gallery" => regenerate(&new, region, |i| gallery(&list, &res, i)),
                _ => regenerate(&new, region, |i| options(&list, i)),
            };
        }
        if new != page {
            if update {
                std::fs::write(&path, new).unwrap_or_else(|e| panic!("{rel}: {e}"));
            } else {
                stale.push(rel);
            }
        }
    }
    assert!(
        stale.is_empty(),
        "skin lists out of step with site/data/skins.json in {stale:?}: \
         run scripts/gen-skin-list.sh"
    );
}

#[test]
fn regenerate_replaces_only_the_region() {
    let page = "a\n  <!-- BEGIN skins:x (note) -->\n  old\n  <!-- END skins:x -->\nb\n";
    let out = regenerate(page, "x", |indent| format!("{indent}new\n"));
    assert_eq!(
        out,
        "a\n  <!-- BEGIN skins:x (note) -->\n  new\n  <!-- END skins:x -->\nb\n"
    );
}
