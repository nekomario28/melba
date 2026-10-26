//! Bit-exactness proof for the decode path: our symphonia→i32 conversion must
//! produce byte-identical output to a reference decoder (ffmpeg) at the same
//! container format. Combined with the ALSA BITPERFECT verdict (rate/channels/
//! depth granted verbatim) this covers the full local-playback claim:
//!
//!   source PCM == decoder output == bytes written to the S32LE/S16LE sink
//!
//! Tests skip (pass trivially, reported as SKIP in output) when ffmpeg or the
//! fixture is unavailable — honesty over green.

use std::path::Path;
use std::process::Command;

fn i32_to_le_bytes(samples: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

fn ffmpeg_pcm(path: &str, fmt: &str) -> Option<Vec<u8>> {
    let out = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            path,
            "-f",
            fmt,
            "-acodec",
            match fmt {
                "s32le" => "pcm_s32le",
                "s16le" => "pcm_s16le",
                _ => return None,
            },
            "-",
        ])
        .output()
        .ok()?;
    if out.status.success() {
        Some(out.stdout)
    } else {
        None
    }
}

/// Compare ours vs reference; container priming/padding can make the tail
/// length differ slightly, so compare the common prefix and report lengths.
fn assert_bit_exact(name: &str, fixture: &str) {
    let path = format!("fixtures/{fixture}");
    if !Path::new(&path).exists() {
        eprintln!("SKIP {name}: fixture {path} missing");
        return;
    }
    let (ours, src) = melbad::audio::decode_all_i32(&path).expect("decode_all_i32");
    let ours_b = i32_to_le_bytes(&ours);
    let Some(ref_b) = ffmpeg_pcm(&path, "s32le") else {
        eprintln!("SKIP {name}: ffmpeg unavailable");
        return;
    };
    let n = ours_b.len().min(ref_b.len());
    assert!(
        n > 0,
        "{name}: empty decode (ours={} ref={})",
        ours_b.len(),
        ref_b.len()
    );
    assert_eq!(
        &ours_b[..n],
        &ref_b[..n],
        "{name}: sample data diverges (codec={} {}Hz {}bit)",
        src.codec,
        src.rate,
        src.bits
    );
    // tail delta tolerance: one packet of priming/terminus difference max
    let delta = (ours_b.len() as i64 - ref_b.len() as i64).unsigned_abs() as usize;
    let ch = src.channels.max(1) as usize;
    assert!(
        delta <= 4096 * ch * 4,
        "{name}: length delta {delta} bytes exceeds one packet budget"
    );
}

#[test]
fn alac_16bit_is_bit_exact() {
    assert_bit_exact("alac16", "88k-16bit.m4a");
}

#[test]
fn alac_24bit_is_bit_exact() {
    assert_bit_exact("alac24", "96k-24bit.m4a");
}

#[test]
fn flac_16bit_is_bit_exact() {
    assert_bit_exact("flac16", "44k-16bit.flac");
}

#[test]
fn flac_24bit_is_bit_exact() {
    assert_bit_exact("flac24", "96k-24bit.flac");
}

/// The S16LE sink path (`i32 >> 16`) must be exact for 16-bit sources.
#[test]
fn s16_sink_path_is_exact_for_16bit_sources() {
    let path = "fixtures/88k-16bit.m4a";
    if !Path::new(path).exists() {
        eprintln!("SKIP s16-path: fixture missing");
        return;
    }
    let Some(ref_b) = ffmpeg_pcm(path, "s16le") else {
        eprintln!("SKIP s16-path: ffmpeg unavailable");
        return;
    };
    let (ours, _) = melbad::audio::decode_all_i32(path).expect("decode");
    let n = ours.len().min(ref_b.len() / 2);
    assert!(n > 0);
    for i in 0..n {
        let s16 = (ours[i] >> 16) as i16;
        let r16 = i16::from_le_bytes([ref_b[2 * i], ref_b[2 * i + 1]]);
        assert_eq!(s16, r16, "sample {i} diverges in S16LE path");
    }
}

/// Embedded cover art must be extracted for MPRIS mpris:artUrl.
#[test]
fn embedded_cover_is_extracted() {
    let path = Path::new("fixtures/44k-16bit-art.m4a");
    if !path.exists() {
        eprintln!("SKIP cover: fixture missing");
        return;
    }
    let m = melbad::library::file_meta(path).expect("file_meta");
    assert_eq!(m.title.as_deref(), Some("Bit Test"));
    assert_eq!(m.artist.as_deref(), Some("melbad fixture"));
    assert_eq!(m.album.as_deref(), Some("Fixtures"));
    let (ext, bytes) = m.cover.expect("embedded cover missing");
    assert_eq!(ext, "png");
    assert!(&bytes[..4] == b"\x89PNG", "cover bytes are not a PNG");
}

/// Volume gain: unity is a no-op (bit-exact preserved); 0.5 halves samples
/// with rounding; clamped at full scale. This is the only DSP between decoder
/// and sink, and only when volume < 1.0.
#[test]
fn apply_gain_math() {
    let mut s = vec![i32::MAX, -1000, 250, i32::MIN];
    let orig = s.clone();
    melbad::audio::apply_gain(&mut s, 1.0);
    assert_eq!(s, orig, "unity gain must be a passthrough");

    melbad::audio::apply_gain(&mut s, 0.5);
    assert_eq!(s[1], -500);
    assert_eq!(s[2], 125);
    assert_eq!(s[0], (i32::MAX as f64 * 0.5).round() as i32);

    let mut z = vec![1000];
    melbad::audio::apply_gain(&mut z, 0.0);
    assert_eq!(z[0], 0);
}

/// Gapless reuse boundary: same rate+channels+format-class => the running
/// stream carries the next track; anything else must reopen.
#[test]
fn stream_matches_gates_gapless_reuse() {
    use melbad::audio::{stream_matches, Granted, SourceFormat};
    let src = |rate: u32, ch: u32, bits: u32| SourceFormat {
        path: String::new(),
        codec: String::new(),
        rate,
        channels: ch,
        bits,
        dur_ms: 0,
    };
    let g = |rate: u32, ch: u32, fmt: &str| Granted {
        device: String::new(),
        rate,
        format: fmt.to_string(),
        msbits: 0,
        channels: ch,
    };
    // identical format-class -> reusable
    assert!(stream_matches(&g(44100, 2, "S16LE"), &src(44100, 2, 16)));
    assert!(stream_matches(&g(96000, 2, "S32LE"), &src(96000, 2, 24)));
    // S24LE fallback grant stays reusable for a 24-bit source
    assert!(stream_matches(&g(96000, 2, "S24LE"), &src(96000, 2, 24)));
    // rate / channels / depth-class changes must reopen
    assert!(!stream_matches(&g(44100, 2, "S16LE"), &src(96000, 2, 16)));
    assert!(!stream_matches(&g(44100, 2, "S16LE"), &src(44100, 1, 16)));
    // 16-bit source must NOT reuse a 32-bit container (verdict honesty)
    assert!(!stream_matches(&g(44100, 2, "S32LE"), &src(44100, 2, 16)));
    // and a 24-bit source never fits an S16 stream
    assert!(!stream_matches(&g(44100, 2, "S16LE"), &src(44100, 2, 24)));
}
