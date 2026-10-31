use super::*;

fn sample_search_json() -> &'static str {
    r#"{
        "response": {
            "numFound": 2,
            "docs": [
                {
                    "identifier": "item-001",
                    "title": "Old Radio Show",
                    "creator": "Radio Corp"
                },
                {
                    "identifier": "item-002",
                    "title": "Jazz Hour",
                    "creator": "Jazz Inc"
                }
            ]
        }
    }"#
}

fn sample_files_json() -> &'static str {
    r#"{
        "result": [
            {
                "name": "show01.mp3",
                "format": "VBR MP3",
                "title": "Episode 1"
            },
            {
                "name": "cover.jpg",
                "format": "JPEG"
            },
            {
                "name": "show02.mp3",
                "format": "128Kbps MP3",
                "title": "Episode 2"
            }
        ]
    }"#
}

#[test]
fn parse_search_response_valid() {
    let results = ArchiveCatalog::parse_search_response(sample_search_json());
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "item-001");
    assert_eq!(results[0].1, "Old Radio Show");
    assert_eq!(results[0].2, "Radio Corp");
    assert_eq!(results[1].0, "item-002");
}

#[test]
fn parse_search_response_empty() {
    let json = r#"{"response": {"numFound": 0, "docs": []}}"#;
    let results = ArchiveCatalog::parse_search_response(json);
    assert!(results.is_empty());
}

#[test]
fn parse_search_response_invalid_json() {
    let results = ArchiveCatalog::parse_search_response("not json");
    assert!(results.is_empty());
}

#[test]
fn parse_search_response_missing_fields() {
    let json = r#"{"response": {"docs": [{"identifier": "x"}]}}"#;
    let results = ArchiveCatalog::parse_search_response(json);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1, ""); // title missing
    assert_eq!(results[0].2, "Unknown"); // creator missing
}

#[test]
fn parse_files_response_filters_mp3() {
    let tracks =
        ArchiveCatalog::parse_files_response(sample_files_json(), "item-001", "Radio Corp");
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].filename, "show01.mp3");
    assert_eq!(tracks[0].title, "Episode 1");
    assert_eq!(tracks[0].creator, "Radio Corp");
    assert_eq!(tracks[1].filename, "show02.mp3");
}

#[test]
fn parse_files_response_empty() {
    let json = r#"{"result": []}"#;
    let tracks = ArchiveCatalog::parse_files_response(json, "x", "y");
    assert!(tracks.is_empty());
}

#[test]
fn parse_files_response_invalid_json() {
    let tracks = ArchiveCatalog::parse_files_response("bad", "x", "y");
    assert!(tracks.is_empty());
}

#[test]
fn next_track_wraps_around() {
    let mut catalog = ArchiveCatalog::new("test");
    catalog.tracks = vec![
        ArchiveTrack {
            item_id: "a".into(),
            filename: "1.mp3".into(),
            title: "T1".into(),
            creator: "C".into(),
        },
        ArchiveTrack {
            item_id: "a".into(),
            filename: "2.mp3".into(),
            title: "T2".into(),
            creator: "C".into(),
        },
    ];
    catalog.current = 0;

    let t = catalog.next_track().unwrap();
    assert_eq!(t.title, "T2");
    assert_eq!(catalog.current, 1);

    let t = catalog.next_track().unwrap();
    assert_eq!(t.title, "T1");
    assert_eq!(catalog.current, 0);
}

#[test]
fn next_track_empty_catalog() {
    let mut catalog = ArchiveCatalog::new("test");
    assert!(catalog.next_track().is_none());
}

#[test]
fn current_track_returns_correct() {
    let mut catalog = ArchiveCatalog::new("test");
    assert!(catalog.current_track().is_none());

    catalog.tracks.push(ArchiveTrack {
        item_id: "a".into(),
        filename: "1.mp3".into(),
        title: "T1".into(),
        creator: "C".into(),
    });
    assert_eq!(catalog.current_track().unwrap().title, "T1");
}

#[test]
fn shuffle_changes_order() {
    let mut catalog = ArchiveCatalog::new("test");
    for i in 0..20 {
        catalog.tracks.push(ArchiveTrack {
            item_id: format!("item-{i}"),
            filename: format!("{i}.mp3"),
            title: format!("Track {i}"),
            creator: "C".into(),
        });
    }
    let original: Vec<String> = catalog.tracks.iter().map(|t| t.title.clone()).collect();
    catalog.shuffle(42);
    let shuffled: Vec<String> = catalog.tracks.iter().map(|t| t.title.clone()).collect();
    // Very unlikely to be identical with 20 items.
    assert_ne!(original, shuffled);
    assert_eq!(catalog.current, 0);
}

#[test]
fn shuffle_single_item() {
    let mut catalog = ArchiveCatalog::new("test");
    catalog.tracks.push(ArchiveTrack {
        item_id: "a".into(),
        filename: "1.mp3".into(),
        title: "T1".into(),
        creator: "C".into(),
    });
    catalog.shuffle(0);
    assert_eq!(catalog.tracks.len(), 1);
}

#[test]
fn download_path_format() {
    let track = ArchiveTrack {
        item_id: "item-123".into(),
        filename: "audio.mp3".into(),
        title: "T".into(),
        creator: "C".into(),
    };
    assert_eq!(
        ArchiveCatalog::download_path(&track),
        "/download/item-123/audio.mp3"
    );
}

#[test]
fn download_url_format() {
    let track = ArchiveTrack {
        item_id: "item-123".into(),
        filename: "audio.mp3".into(),
        title: "T".into(),
        creator: "C".into(),
    };
    assert_eq!(
        ArchiveCatalog::download_url(&track),
        "https://archive.org/download/item-123/audio.mp3"
    );
}

#[test]
fn search_url_format() {
    let url = ArchiveCatalog::search_url("oldtimeradio");
    assert!(url.contains("collection:oldtimeradio"));
    assert!(url.contains("mediatype:audio"));
    assert!(url.contains("output=json"));
}

#[test]
fn files_api_path_format() {
    assert_eq!(
        ArchiveCatalog::files_api_path("item-123"),
        "/metadata/item-123/files"
    );
}

#[test]
fn percent_encode_spaces() {
    assert_eq!(percent_encode("hello world.mp3"), "hello%20world.mp3");
}

#[test]
fn percent_encode_special_chars() {
    assert_eq!(percent_encode("file#1.mp3"), "file%231.mp3");
    assert_eq!(percent_encode("a?b&c+d%e"), "a%3Fb%26c%2Bd%25e");
}

#[test]
fn percent_encode_safe_string() {
    assert_eq!(percent_encode("simple.mp3"), "simple.mp3");
    assert_eq!(percent_encode("a-b_c.mp3"), "a-b_c.mp3");
}

#[test]
fn percent_encode_unicode() {
    let encoded = percent_encode("caf\u{00e9}.mp3");
    assert!(
        encoded.contains("%"),
        "expected percent-encoded output: {encoded}"
    );
    assert!(!encoded.contains('\u{00e9}'));
}

#[test]
fn download_path_encodes_filename() {
    let track = ArchiveTrack {
        item_id: "item-123".into(),
        filename: "my song #1.mp3".into(),
        title: "T".into(),
        creator: "C".into(),
    };
    let path = ArchiveCatalog::download_path(&track);
    assert_eq!(path, "/download/item-123/my%20song%20%231.mp3");
}

#[test]
fn download_url_encodes_filename() {
    let track = ArchiveTrack {
        item_id: "item-123".into(),
        filename: "track one.mp3".into(),
        title: "T".into(),
        creator: "C".into(),
    };
    let url = ArchiveCatalog::download_url(&track);
    assert_eq!(url, "https://archive.org/download/item-123/track%20one.mp3");
}

// ---------------------------------------------------------------
// Additional edge case tests
// ---------------------------------------------------------------

#[test]
fn parse_search_response_creator_as_array() {
    // Some IA items have creator as an array — should fallback to "Unknown".
    let json = r#"{
        "response": {
            "docs": [
                {
                    "identifier": "item-x",
                    "title": "Multi-Author Work",
                    "creator": ["Author A", "Author B"]
                }
            ]
        }
    }"#;
    let results = ArchiveCatalog::parse_search_response(json);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "item-x");
    // as_str() returns None for array → fallback to "Unknown".
    assert_eq!(results[0].2, "Unknown");
}

#[test]
fn parse_search_response_missing_identifier_skipped() {
    let json = r#"{
        "response": {
            "docs": [
                {"title": "No ID", "creator": "Someone"},
                {"identifier": "valid", "title": "Has ID"}
            ]
        }
    }"#;
    let results = ArchiveCatalog::parse_search_response(json);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "valid");
}

#[test]
fn parse_files_response_no_title_uses_filename() {
    let json = r#"{
        "result": [
            {
                "name": "untitled_track.mp3",
                "format": "VBR MP3"
            }
        ]
    }"#;
    let tracks = ArchiveCatalog::parse_files_response(json, "item-1", "Artist");
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "untitled_track.mp3");
}

#[test]
fn parse_files_response_mixed_formats() {
    let json = r#"{
        "result": [
            {"name": "track.mp3", "format": "VBR MP3"},
            {"name": "track.ogg", "format": "Ogg Vorbis"},
            {"name": "track.flac", "format": "Flac"},
            {"name": "bonus.mp3", "format": "128Kbps MP3"},
            {"name": "cover.jpg", "format": "JPEG Thumb"}
        ]
    }"#;
    let tracks = ArchiveCatalog::parse_files_response(json, "item", "A");
    assert_eq!(tracks.len(), 2); // Only MP3 files.
    assert_eq!(tracks[0].filename, "track.mp3");
    assert_eq!(tracks[1].filename, "bonus.mp3");
}

#[test]
fn catalog_full_iteration() {
    let mut catalog = ArchiveCatalog::new("test");
    for i in 0..5 {
        catalog.tracks.push(ArchiveTrack {
            item_id: format!("item-{i}"),
            filename: format!("{i}.mp3"),
            title: format!("Track {i}"),
            creator: "C".into(),
        });
    }

    // Iterate through all tracks and back to start.
    let mut titles = Vec::new();
    for _ in 0..5 {
        let t = catalog.next_track().unwrap();
        titles.push(t.title.clone());
    }
    // Should have seen tracks 1,2,3,4,0 (wraps around).
    assert_eq!(titles.len(), 5);
    assert_eq!(catalog.current, 0); // Wrapped back to 0.
}

#[test]
fn shuffle_deterministic() {
    // Same seed should produce same order.
    let make_catalog = || {
        let mut cat = ArchiveCatalog::new("test");
        for i in 0..10 {
            cat.tracks.push(ArchiveTrack {
                item_id: format!("item-{i}"),
                filename: format!("{i}.mp3"),
                title: format!("Track {i}"),
                creator: "C".into(),
            });
        }
        cat
    };

    let mut cat1 = make_catalog();
    let mut cat2 = make_catalog();
    cat1.shuffle(12345);
    cat2.shuffle(12345);

    let order1: Vec<&str> = cat1.tracks.iter().map(|t| t.title.as_str()).collect();
    let order2: Vec<&str> = cat2.tracks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(order1, order2, "same seed should produce same shuffle");
}

#[test]
fn shuffle_different_seeds_differ() {
    let make_catalog = || {
        let mut cat = ArchiveCatalog::new("test");
        for i in 0..20 {
            cat.tracks.push(ArchiveTrack {
                item_id: format!("item-{i}"),
                filename: format!("{i}.mp3"),
                title: format!("Track {i}"),
                creator: "C".into(),
            });
        }
        cat
    };

    let mut cat1 = make_catalog();
    let mut cat2 = make_catalog();
    cat1.shuffle(111);
    cat2.shuffle(222);

    let order1: Vec<&str> = cat1.tracks.iter().map(|t| t.title.as_str()).collect();
    let order2: Vec<&str> = cat2.tracks.iter().map(|t| t.title.as_str()).collect();
    assert_ne!(order1, order2);
}

#[test]
fn hostile_identifiers_and_filenames_cannot_inject_into_requests() {
    let json = r#"{"response": {"docs": [
        {"identifier": "ok-item_1.2"},
        {"identifier": "../etc"},
        {"identifier": "a b"},
        {"identifier": "x\r\nHost: evil"},
        {"identifier": "a/b"}
    ]}}"#;
    let ids: Vec<String> = ArchiveCatalog::parse_search_response(json)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(ids, ["ok-item_1.2"]);
    assert_eq!(percent_encode("a\r\nb\t\x7f"), "a%0D%0Ab%09%7F");
}

#[test]
fn percent_encode_empty_string() {
    assert_eq!(percent_encode(""), "");
}

#[test]
fn percent_encode_all_special() {
    assert_eq!(percent_encode(" #?&%+"), "%20%23%3F%26%25%2B");
}

#[test]
fn percent_encode_path_separators_preserved() {
    // Forward slashes should NOT be encoded (they're path separators).
    assert_eq!(percent_encode("path/to/file.mp3"), "path/to/file.mp3");
}

#[test]
fn download_path_complex_filename() {
    let track = ArchiveTrack {
        item_id: "OTR-collection".into(),
        filename: "The Shadow - Episode #42 (Part 1 & 2).mp3".into(),
        title: "T".into(),
        creator: "C".into(),
    };
    let path = ArchiveCatalog::download_path(&track);
    assert!(path.starts_with("/download/OTR-collection/"));
    assert!(path.contains("%20")); // Spaces encoded.
    assert!(path.contains("%23")); // # encoded.
    assert!(path.contains("%26")); // & encoded.
    assert!(!path.contains(' ')); // No raw spaces.
    assert!(!path.contains('#')); // No raw #.
}

#[test]
fn search_url_contains_required_params() {
    let url = ArchiveCatalog::search_url("testcollection");
    assert!(url.starts_with("https://archive.org/advancedsearch.php?"));
    assert!(url.contains("collection:testcollection"));
    assert!(url.contains("mediatype:audio"));
    assert!(url.contains("fl=identifier,title,creator"));
    assert!(url.contains("output=json"));
    assert!(url.contains("rows=50"));
}

#[test]
fn parse_search_response_large_result_set() {
    // Simulate 50 results.
    let mut docs = Vec::new();
    for i in 0..50 {
        docs.push(format!(
            r#"{{"identifier":"item-{i}","title":"Title {i}","creator":"Creator {i}"}}"#
        ));
    }
    let json = format!(
        r#"{{"response":{{"numFound":50,"docs":[{}]}}}}"#,
        docs.join(",")
    );
    let results = ArchiveCatalog::parse_search_response(&json);
    assert_eq!(results.len(), 50);
}
