//! Source declarations must describe the codec, not MP4 sample-entry placeholders.

use std::io::Cursor;
use std::sync::atomic::AtomicU64;

use melbad::audio;

fn child(bytes: &[u8], start: usize, end: usize, kind: &[u8; 4]) -> (usize, usize) {
    let mut pos = start;
    while pos + 8 <= end {
        let size = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && pos + size <= end, "invalid fixture box");
        if &bytes[pos + 4..pos + 8] == kind {
            return (pos, pos + size);
        }
        pos += size;
    }
    panic!("fixture box {kind:?} missing");
}

fn alac_entry(bytes: &[u8]) -> (usize, usize) {
    let (mut start, mut end) = (0, bytes.len());
    for kind in [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd"] {
        let (pos, next_end) = child(bytes, start, end, kind);
        start = pos + if kind == b"stsd" { 16 } else { 8 };
        end = next_end;
    }
    child(bytes, start, end, b"alac")
}

fn conflicting_alac() -> Vec<u8> {
    let mut bytes = include_bytes!("../fixtures/96k-24bit.m4a").to_vec();
    let (entry, _) = alac_entry(&bytes);
    assert_eq!(&bytes[entry + 16..entry + 18], &[0, 0]);
    // The ALAC configuration and encoded samples remain 24-bit/96 kHz/stereo.
    bytes[entry + 24..entry + 26].copy_from_slice(&1u16.to_be_bytes());
    bytes[entry + 26..entry + 28].copy_from_slice(&16u16.to_be_bytes());
    bytes[entry + 32..entry + 36].copy_from_slice(&(48_000u32 << 16).to_be_bytes());
    bytes
}

#[test]
fn alac_source_format_ignores_generic_mp4_placeholders() {
    let pf = audio::open_media(
        Box::new(Cursor::new(conflicting_alac())),
        Some("m4a"),
        "stream:fixture",
    )
    .expect("probe ALAC fixture");
    assert_eq!(
        (pf.src.rate, pf.src.bits, pf.src.channels),
        (96_000, 24, 2),
        "ALAC codec configuration must govern reporting and output selection"
    );
    assert_eq!(pf.src.codec, "alac");
    // ALSA's null plugin negotiates the real output path without desktop audio.
    let xruns = AtomicU64::new(0);
    let (pcm, granted) = audio::open_exact("null", &pf.src).expect("open null output");
    assert_eq!((granted.rate, granted.channels), (96_000, 2));
    assert_eq!(granted.format, "S32LE");
    assert!(granted.msbits >= 24);
    assert!(audio::stream_matches(&granted, &pf.src));
    assert!(
        audio::play_into(&pcm, &granted, pf, Some(4096), &xruns).expect("decode into null") > 0
    );
}

#[test]
fn reported_format_matches_the_prepared_decoders_signal() {
    for (bytes, ext, rate, bits, channels) in [
        (conflicting_alac(), "m4a", 96_000, 24, 2),
        (
            include_bytes!("../fixtures/44k-16bit-art.m4a").to_vec(),
            "m4a",
            44_100,
            16,
            2,
        ),
        (
            include_bytes!("../fixtures/96k-mono.flac").to_vec(),
            "flac",
            96_000,
            16,
            1,
        ),
    ] {
        let mut pf = audio::open_media(Box::new(Cursor::new(bytes)), Some(ext), "fixture")
            .expect("prepare decoder");
        assert_eq!(
            (pf.src.rate, pf.src.bits, pf.src.channels),
            (rate, bits, channels)
        );
        let packet = pf.format.next_packet().expect("first audio packet");
        assert_eq!(packet.track_id(), pf.track_id);
        let decoded = pf.decoder.decode(&packet).expect("decode first packet");
        assert!(decoded.frames() > 0);
        assert_eq!(decoded.spec().rate, pf.src.rate);
        assert_eq!(decoded.spec().channels.count() as u32, pf.src.channels);
        if bits == 24 {
            assert!(
                audio::decoded_to_i32(decoded)
                    .iter()
                    .any(|s| s & 0xff00 != 0),
                "24-bit samples must retain precision below 16 bits"
            );
        }
    }
}

#[test]
fn mono_aac_uses_the_prepared_decoders_signal_format() {
    // Synthetic 80ms mono sine; no account media or external fixture dependency.
    // ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.08
    //   -ac 1 -c:a aac -b:a 64k -threads 1 -flags +bitexact -fflags +bitexact
    //   -map_metadata -1 fixtures/48k-mono-aac.m4a
    let pf = audio::open_media(
        Box::new(Cursor::new(
            include_bytes!("../fixtures/48k-mono-aac.m4a").as_slice(),
        )),
        Some("m4a"),
        "stream:mono-aac-fixture",
    )
    .expect("prepare mono AAC");
    assert_eq!((pf.src.rate, pf.src.channels), (48_000, 1));
    assert_eq!(pf.src.codec, "mp4a");
    let xruns = AtomicU64::new(0);
    let (pcm, granted) = audio::open_exact("null", &pf.src).expect("open mono null output");
    assert_eq!((granted.rate, granted.channels), (48_000, 1));
    assert!(
        audio::play_into(&pcm, &granted, pf, None, &xruns).expect("decode mono AAC into null") > 0
    );
}

#[test]
fn shared_cli_receipt_does_not_claim_an_exclusive_hardware_path() {
    let dir = std::env::temp_dir().join(format!(
        "melba-format-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("private CLI directory");
    let config = dir.join("alsa.conf");
    std::fs::write(&config, "pcm.null { type null }\n").expect("null-only ALSA config");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_melbad"))
        .args([
            "play",
            concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/96k-24bit.m4a"),
            "--device",
            "null",
            "--verify",
            "--max-seconds",
            "1",
        ])
        .env("HOME", &dir)
        .env("XDG_CONFIG_HOME", &dir)
        .env("XDG_DATA_HOME", &dir)
        .env("XDG_STATE_HOME", &dir)
        .env("XDG_CACHE_HOME", &dir)
        .env("ALSA_CONFIG_PATH", &config)
        .env_remove("MELBA_EVIDENCE_DIR")
        .output();
    std::fs::remove_dir_all(dir).expect("remove CLI directory");
    let output = result.expect("CLI verifier");
    assert_eq!(
        output.status.code(),
        Some(7),
        "shared output must be refused"
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("format receipt");
    assert_eq!(receipt["source"]["rate"], 96_000);
    assert_eq!(receipt["source"]["bits"], 24);
    assert_eq!(receipt["granted"]["device"], "null");
    assert_eq!(receipt["verdict"], "SHARED");
    assert_eq!(receipt["frames_written"], 0);
    assert!(receipt["notes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|note| !note.as_str().unwrap().starts_with("exclusive hw:")));
}

#[test]
fn invalid_alac_configuration_is_not_guessed_from_generic_fields() {
    for (offset, value) in [(5, vec![0]), (20, vec![0; 4]), (9, vec![0])] {
        let mut bytes = conflicting_alac();
        let (entry, end) = alac_entry(&bytes);
        let (config, _) = child(&bytes, entry + 36, end, b"alac");
        let start = config + 12 + offset;
        bytes[start..start + value.len()].copy_from_slice(&value);
        let result = audio::open_media(Box::new(Cursor::new(bytes)), Some("m4a"), "fixture");
        assert!(
            result.is_err(),
            "an invalid ALAC configuration must not be reported as a valid source"
        );
    }
}
