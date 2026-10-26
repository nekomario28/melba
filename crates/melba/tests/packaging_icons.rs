//! Packaging icons must load in Qt's SVG renderer (system tray / desktop).
use std::fs;
use std::path::PathBuf;

#[test]
fn linux_tray_svg_avoids_filters_qt_rejects() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packaging/icons/melba.svg");
    let svg = fs::read_to_string(&path).expect("melba.svg");
    assert!(
        !svg.contains("feFuncA") && !svg.contains("<filter"),
        "Qt 6 rejects feFuncA inside feComponentTransfer; the tray then shows a checkerboard (#660)"
    );
    assert!(svg.contains("viewBox"), "icon must remain a valid SVG mark");
}

/// The desktop entry may only register URL schemes `link::parse` can
/// actually open, and must pass the URL through as `%u`.
#[test]
fn desktop_entry_schemes_match_what_parse_accepts() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packaging/melba.desktop");
    let desktop = fs::read_to_string(&path).expect("melba.desktop");
    assert!(desktop.contains("Exec=melba %u"), "the entry must take %u");
    let mime = desktop
        .lines()
        .find_map(|line| line.strip_prefix("MimeType="))
        .expect("MimeType line");
    assert!(!mime.is_empty(), "the entry must claim its schemes");
    for entry in mime.trim_end_matches(';').split(';') {
        let scheme = entry
            .strip_prefix("x-scheme-handler/")
            .unwrap_or_else(|| panic!("{entry} is not a scheme handler"));
        let probe = format!("{scheme}://music.apple.com/jp/album/x/1588049004");
        assert!(
            melba::link::parse(&probe).is_some(),
            "{entry} is registered but {probe} does not parse"
        );
    }
}
