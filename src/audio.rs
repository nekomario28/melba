//! Slice 01 audio core: decode (symphonia) -> ALSA hw: at exact source format.
//! Never negotiate down: if the granted params differ from the source, the
//! caller sees MISMATCH/CLAMPED instead of silent resampling.

use alsa::pcm::{Access, Format, HwParams, State, IO, PCM};
use alsa::{Direction, ValueOr};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_AAC, CODEC_TYPE_ALAC};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone, Serialize)]
pub struct SourceFormat {
    pub path: String,
    pub codec: String,
    pub rate: u32,
    pub channels: u32,
    pub bits: u32,
    #[serde(default)]
    pub dur_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Granted {
    pub device: String,
    pub rate: u32,
    pub format: String,
    pub msbits: u32,
    pub channels: u32,
    /// ALSA buffer depth actually granted, in frames. This is the slack the
    /// writer has; the receipt keeps it so a regression is visible.
    #[serde(default)]
    pub buffer_frames: u32,
    /// ALSA period actually granted, in frames.
    #[serde(default)]
    pub period_frames: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceCap {
    pub device: String,
    pub card_name: String,
    pub rate_min: u32,
    pub rate_max: u32,
    pub ch_min: u32,
    pub ch_max: u32,
    pub openable: bool,
    pub note: String,
}

impl DeviceCap {
    /// Worth offering in a picker: opens cleanly, or is busy because the
    /// daemon (or something else) already holds it — busy still names a real
    /// device. Phantom subdevice slots ("No such file or directory") and
    /// other probe failures are hidden.
    pub fn usable(&self) -> bool {
        self.openable || self.note.to_lowercase().contains("busy")
    }
}

#[derive(Debug)]
pub enum OpenError {
    Busy(String),
    Rejected { device: String, why: String },
    NoDevice { tried: Vec<String> },
}

// ---- source probing -------------------------------------------------------

/// A source and decoder prepared before output negotiation. Reporting and
/// playback consume the same codec configuration and opened reader.
/// `format` owns the MediaSourceStream internally.
pub struct ProbedFile {
    pub src: SourceFormat,
    pub format: Box<dyn FormatReader>,
    pub decoder: Box<dyn Decoder>,
    pub track_id: u32,
}

/// One open+probe for any MediaSource — files and the stream: AmSource
/// share this so each is opened and probed exactly once.
pub fn open_media(
    ms: Box<dyn MediaSource>,
    ext: Option<&str>,
    name: &str,
) -> Result<ProbedFile, String> {
    let mss = MediaSourceStream::new(ms, Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("probe {name}: {e}"))?;
    let track = probed
        .format
        .default_track()
        .ok_or_else(|| "no default track".to_string())?;
    let track_id = track.id;
    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder {name}: {e}"))?;
    let src = source_format(name, &track.codec_params, &*decoder)?;
    Ok(ProbedFile {
        src,
        format: probed.format,
        decoder,
        track_id,
    })
}

/// Open+probe a local file — the single-open form of `probe_source`.
pub fn open_source(path: &str) -> Result<ProbedFile, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str());
    open_media(Box::new(file), ext, path)
}

/// Format-only probe — callers that don't need the reader (library tag
/// scan) keep the cheap signature; everything else uses `open_source`.
pub fn probe_source(path: &str) -> Result<SourceFormat, String> {
    open_source(path).map(|p| p.src)
}

fn source_format(
    path: &str,
    cp: &symphonia::core::codecs::CodecParameters,
    decoder: &dyn Decoder,
) -> Result<SourceFormat, String> {
    let (rate, channels) = if matches!(cp.codec, CODEC_TYPE_ALAC | CODEC_TYPE_AAC) {
        // These decoders initialize their signal specification from the codec
        // configuration before consuming packets. codec_params() can still
        // contain MP4 placeholders, including a missing mono AAC layout.
        let spec = *decoder.last_decoded().spec();
        (spec.rate, spec.channels.count() as u32)
    } else {
        (
            cp.sample_rate.ok_or("source has no sample_rate")?,
            cp.channels.map(|c| c.count() as u32).unwrap_or(2),
        )
    };
    if rate == 0 || channels == 0 {
        return Err("invalid source signal specification".into());
    }
    let bits = if cp.codec == CODEC_TYPE_ALAC {
        // The ALAC buffer uses i32 for both 24- and 32-bit PCM, so its sample
        // type is not the encoded depth. Read the decoder-validated cookie.
        let config = cp
            .extra_data
            .as_deref()
            .ok_or("ALAC configuration missing")?;
        if !matches!(config.len(), 24 | 48) {
            return Err("invalid ALAC configuration length".into());
        }
        let bits = u32::from(config[5]);
        if !(1..=32).contains(&bits) {
            return Err("invalid ALAC bit depth".into());
        }
        bits
    } else {
        cp.bits_per_coded_sample
            .or(cp.bits_per_sample)
            .unwrap_or(16)
    };
    let dur_ms = match (cp.n_frames, cp.time_base) {
        (Some(n), Some(tb)) => {
            // u128: n_frames * numer overflows u64 on multi-hour hi-res
            // rips (e.g. >96kHz); clamp to u64 rather than wrap to ~0
            ((n as u128 * tb.numer as u128 * 1000) / tb.denom.max(1) as u128).min(u64::MAX as u128)
                as u64
        }
        _ => 0,
    };
    Ok(SourceFormat {
        path: path.to_string(),
        codec: match cp.codec {
            CODEC_TYPE_ALAC => "alac".into(),
            CODEC_TYPE_AAC => "mp4a".into(),
            codec => format!("{codec:?}"),
        },
        rate,
        channels,
        bits,
        dur_ms,
    })
}

// ---- device enumeration ---------------------------------------------------

/// List PCM playback devices with capability bounds read from hw params.
/// Opening a device fails (Busy) when PipeWire or another client holds it —
/// that is reported, not hidden.
pub fn list_devices() -> Vec<DeviceCap> {
    let mut out = Vec::new();
    for card in alsa::card::Iter::new().filter_map(|c| c.ok()) {
        let card_id = card.get_index();
        let cname = card.get_name().unwrap_or_else(|_| "?".into());
        for dev in 0..16i32 {
            let name = format!("hw:{card_id},{dev}");
            // Probe via non-blocking open; Busy is honest signal.
            match PCM::new(&name, Direction::Playback, true) {
                Ok(pcm) => match HwParams::any(&pcm) {
                    Ok(hwp) => {
                        let rmin = hwp.get_rate_min().unwrap_or(0);
                        let rmax = hwp.get_rate_max().unwrap_or(0);
                        let cmin = hwp.get_channels_min().unwrap_or(0);
                        let cmax = hwp.get_channels_max().unwrap_or(0);
                        out.push(DeviceCap {
                            device: name.clone(),
                            card_name: cname.clone(),
                            rate_min: rmin,
                            rate_max: rmax,
                            ch_min: cmin,
                            ch_max: cmax,
                            openable: true,
                            note: String::new(),
                        });
                    }
                    Err(e) => out.push(DeviceCap {
                        device: name.clone(),
                        card_name: cname.clone(),
                        rate_min: 0,
                        rate_max: 0,
                        ch_min: 0,
                        ch_max: 0,
                        openable: false,
                        note: format!("hwparams: {e}"),
                    }),
                },
                Err(e) => out.push(DeviceCap {
                    device: name,
                    card_name: cname.clone(),
                    rate_min: 0,
                    rate_max: 0,
                    ch_min: 0,
                    ch_max: 0,
                    openable: false,
                    note: e.to_string(),
                }),
            }
        }
    }
    out
}

// ---- exact-format open ----------------------------------------------------

fn fmt_for_depth(bits: u32) -> &'static [Format] {
    match bits {
        0..=16 => &[Format::S16LE],
        17..=24 => &[Format::S32LE, Format::S24LE],
        _ => &[Format::S32LE, Format::FloatLE],
    }
}

/// A previously granted stream can carry `src` unchanged when rate, channel
/// count and the wire format all match — the gapless boundary. The granted
/// format must be one `open_exact` would try for `src.bits` (e.g. an S24LE
/// fallback stream stays reusable for the next 24-bit source); a wider
/// container is intentionally not reused so the verdict stays honest.
pub fn stream_matches(g: &Granted, src: &SourceFormat) -> bool {
    g.rate == src.rate
        && g.channels == src.channels
        && fmt_for_depth(src.bits)
            .iter()
            .any(|f| g.format == format!("{f:?}"))
}

fn fmt_msbits(f: Format) -> u32 {
    match f {
        Format::S16LE => 16,
        Format::S24LE => 24,
        Format::S32LE => 32,
        Format::FloatLE => 32,
        _ => 0,
    }
}

/// Ask the session audio server to release the ALSA card behind `device`.
///
/// WirePlumber owns `org.freedesktop.ReserveDevice1.AudioN` while it holds
/// the card. `RequestRelease` is its cooperative hand-off: it closes the
/// PCM and lets an exclusive `hw:` client open it. Only a busy device is
/// disturbed; a free device, `auto`, and `default` are left alone.
pub fn release_for_takeover(device: &str) -> Result<(), String> {
    if device == "auto" || device == "default" {
        return Ok(());
    }
    let Some(rest) = device.strip_prefix("hw:") else {
        return Ok(());
    };
    let Some((card, _)) = rest.split_once(',') else {
        return Err("invalid hardware device".into());
    };
    let card: u32 = card.parse().map_err(|_| "invalid hardware card")?;
    match PCM::new(device, Direction::Playback, true) {
        Ok(pcm) => {
            drop(pcm);
            return Ok(());
        }
        Err(error) if error.errno() == nix::errno::Errno::EBUSY as i32 => {}
        Err(error) => return Err(format!("{device}: {error}")),
    }
    let service = format!("org.freedesktop.ReserveDevice1.Audio{card}");
    let path = format!("/org/freedesktop/ReserveDevice1/Audio{card}");
    let connection = dbus::blocking::Connection::new_session()
        .map_err(|error| format!("session bus: {error}"))?;
    let proxy = connection.with_proxy(service, path, std::time::Duration::from_secs(2));
    let (released,): (bool,) = proxy
        .method_call(
            "org.freedesktop.ReserveDevice1",
            "RequestRelease",
            (100i32,),
        )
        .map_err(|error| format!("audio server did not release {device}: {error}"))?;
    if released {
        Ok(())
    } else {
        Err(format!("audio server declined to release {device}"))
    }
}

/// The ALSA period requested: small enough that pause and seek stay
/// responsive, large enough to keep wake-ups cheap.
pub const PERIOD_FRAMES: u32 = 4096;

/// Buffer depth to aim for, in milliseconds.
///
/// The playback loop decodes and writes on one thread, and for a `stream:`
/// source each `next_packet()` may wait on the network and the decrypt
/// service. The buffer is the only slack that work gets: a fixed frame count
/// is not enough slack, because the same count is a different amount of time
/// at every rate. 16384 frames is 371 ms at 44.1 kHz but only 85 ms at
/// 192 kHz, where an ordinary scheduling hiccup drained the device and
/// produced an audible gap. Ask for a duration instead.
pub const BUFFER_MS: u32 = 500;

/// The buffer depth to request for `rate`, never below four periods so the
/// period/buffer relationship stays valid for the driver.
pub fn buffer_frames(rate: u32, period: u32) -> u32 {
    let by_time = (u64::from(rate) * u64::from(BUFFER_MS) / 1000) as u32;
    by_time.max(period.saturating_mul(4))
}

/// Open `device` for playback demanding exactly `src.rate`/`src.channels` and a
/// format able to carry `src.bits` without truncation. Returns granted params
/// read back from the kernel — the caller compares them to the source.
pub fn open_exact(device: &str, src: &SourceFormat) -> Result<(PCM, Granted), OpenError> {
    let mut busy = false;
    for &fmt in fmt_for_depth(src.bits) {
        let pcm = match PCM::new(device, Direction::Playback, false) {
            Ok(p) => p,
            Err(e) if e.errno() == nix::errno::Errno::EBUSY as i32 => {
                busy = true;
                continue;
            }
            Err(e) => {
                return Err(OpenError::Rejected {
                    device: device.to_string(),
                    why: format!("open: {e}"),
                })
            }
        };
        let hwp = match HwParams::any(&pcm) {
            Ok(h) => h,
            Err(e) => {
                return Err(OpenError::Rejected {
                    device: device.into(),
                    why: format!("hwp.any: {e}"),
                })
            }
        };
        // Restrict everything to exactly what the source is.
        if hwp.set_access(Access::RWInterleaved).is_err() {
            continue;
        }
        if hwp.set_format(fmt).is_err() {
            continue;
        }
        if hwp.set_rate(src.rate, ValueOr::Nearest).is_err() {
            continue;
        }
        if hwp.set_channels(src.channels).is_err() {
            continue;
        }
        if hwp
            .set_period_size(i64::from(PERIOD_FRAMES), ValueOr::Nearest)
            .is_err()
        {
            continue;
        }
        if hwp
            .set_buffer_size_near(i64::from(buffer_frames(src.rate, PERIOD_FRAMES)))
            .is_err()
        {
            continue;
        }
        if pcm.hw_params(&hwp).is_err() {
            continue;
        }
        // Read back what the kernel actually granted (hwp is now committed).
        let granted = Granted {
            device: device.to_string(),
            rate: hwp.get_rate().unwrap_or(0),
            format: format!("{:?}", hwp.get_format().unwrap_or(Format::Unknown)),
            msbits: fmt_msbits(fmt),
            channels: hwp.get_channels().unwrap_or(0),
            buffer_frames: hwp.get_buffer_size().unwrap_or(0) as u32,
            period_frames: hwp.get_period_size().unwrap_or(0) as u32,
        };
        if granted.rate != src.rate || granted.channels != src.channels {
            // Nearest resolved to something else -> try next format.
            continue;
        }
        let _ = pcm.prepare();
        drop(hwp);
        return Ok((pcm, granted));
    }
    if busy {
        return Err(OpenError::Busy(format!("{device}: held by another client")));
    }
    Err(OpenError::Rejected {
        device: device.into(),
        why: format!(
            "no format carries {}bit @ {}Hz x{}ch",
            src.bits, src.rate, src.channels
        ),
    })
}

fn auto_rank(d: &DeviceCap) -> (u8, u32, u32) {
    let name = d.card_name.to_ascii_lowercase();
    let hdmi = name.contains("hdmi") || name.contains("displayport");
    let usb = name.contains("usb");
    // A dedicated USB/analog sink is a better default than an unused HDMI
    // output. Explicit hw:X,Y remains the only way to pick among USB ports.
    let class = if usb && !hdmi {
        0
    } else if !hdmi {
        1
    } else {
        2
    };
    let (card, dev) = d
        .device
        .strip_prefix("hw:")
        .and_then(|rest| rest.split_once(','))
        .and_then(|(c, d)| Some((c.parse().ok()?, d.parse().ok()?)))
        .unwrap_or((u32::MAX, u32::MAX));
    (class, card, dev)
}

/// Ordered hw devices that can carry `src` exactly, USB/analog before HDMI.
/// The list is a preference order; `open_exact` still decides.
pub fn auto_candidates(caps: &[DeviceCap], src: &SourceFormat) -> Vec<DeviceCap> {
    let mut out: Vec<DeviceCap> = caps
        .iter()
        .filter(|d| {
            d.usable()
                && d.device.starts_with("hw:")
                && src.rate >= d.rate_min
                && src.rate <= d.rate_max
                && src.channels >= d.ch_min
                && src.channels <= d.ch_max
        })
        .cloned()
        .collect();
    out.sort_by_key(auto_rank);
    out
}

/// Try each playback device until one opens at the exact source format.
pub fn open_auto(src: &SourceFormat) -> Result<(PCM, Granted), OpenError> {
    let candidates = auto_candidates(&list_devices(), src);
    if candidates.is_empty() {
        return Err(OpenError::NoDevice {
            tried: vec![format!(
                "no openable hw device carries {}bit {}Hz x{}ch",
                src.bits, src.rate, src.channels
            )],
        });
    }
    let mut tried = Vec::new();
    for d in candidates {
        match open_exact(&d.device, src) {
            Ok(ok) => return Ok(ok),
            Err(OpenError::Busy(m)) | Err(OpenError::Rejected { why: m, .. }) => {
                tried.push(format!("{} ({})", d.device, m))
            }
            Err(e) => return Err(e),
        }
    }
    Err(OpenError::NoDevice { tried })
}

// ---- playback -------------------------------------------------------------

/// Decode `pf` (already opened + probed) and write PCM to an already-open
/// `pcm` whose Granted layout is in `g`. `max_frames` stops playback early
/// (the `--max-seconds` verify cap). Returns frames written.
pub fn play_into(
    pcm: &PCM,
    g: &Granted,
    pf: ProbedFile,
    max_frames: Option<u64>,
    xruns: &AtomicU64,
) -> Result<u64, String> {
    let mut probed = pf;
    let track_id = probed.track_id;
    let mut dec = probed.decoder;

    let mut frames_total = 0u64;

    while let Ok(packet) = probed.format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode: {e}")),
        };
        let spec: SignalSpec = *decoded.spec();
        let n_frames = decoded.frames() as u64;
        let mut sb = SampleBuffer::<i32>::new(decoded.capacity() as u64, spec);
        sb.copy_interleaved_ref(decoded);
        let samples = sb.samples();

        if g.format == "S16LE" {
            let io = pcm.io_i16().map_err(|e| format!("io_i16: {e}"))?;
            let out: Vec<i16> = samples.iter().map(|&s| (s >> 16) as i16).collect();
            frames_total += write_frames_t(&io, pcm, &out, g.channels as usize, xruns)?;
        } else {
            let io = pcm.io_i32().map_err(|e| format!("io_i32: {e}"))?;
            frames_total += write_frames_t(&io, pcm, samples, g.channels as usize, xruns)?;
        }
        if let Some(m) = max_frames {
            if frames_total >= m {
                break;
            }
        }
        let _ = n_frames;
    }
    let _ = pcm.drain();
    Ok(frames_total)
}

fn write_frames_t<T: Copy>(
    io: &IO<'_, T>,
    pcm: &PCM,
    samples: &[T],
    ch: usize,
    xruns: &AtomicU64,
) -> Result<u64, String> {
    let mut done = 0usize;
    let frames = samples.len() / ch;
    while done < frames {
        match io.writei(&samples[done * ch..]) {
            // 0 frames accepted forever would spin without progress
            Ok(0) => return Err("writei accepted 0 frames".into()),
            Ok(n) => done += n,
            Err(e) => {
                let st = pcm.state();
                if st == State::XRun || st == State::Suspended || st == State::Disconnected {
                    // Recover and keep playing, but never hide it: an
                    // emergency recovery is an audible gap, and a silent one
                    // is impossible to diagnose later.
                    xruns.fetch_add(1, Ordering::SeqCst);
                    pcm.try_recover(e, true)
                        .map_err(|e2| format!("recover: {e2}"))?;
                    continue;
                }
                return Err(format!("writei: {e}"));
            }
        }
    }
    Ok(frames as u64)
}

/// Convert one decoded frame buffer to full-scale interleaved i32 — the exact
/// stream the playback path writes to ALSA as S32LE (symphonia left-justifies:
/// 16-bit <<16, 24-bit <<8). Shared by the player and the bit-exactness test.
pub fn decoded_to_i32(decoded: symphonia::core::audio::AudioBufferRef) -> Vec<i32> {
    let mut sb = SampleBuffer::<i32>::new(decoded.capacity() as u64, *decoded.spec());
    sb.copy_interleaved_ref(decoded);
    sb.samples().to_vec()
}

/// Apply linear volume gain to interleaved i32 samples in place.
/// Gain >= 0.999 is a no-op: unity volume preserves the bit-exact path
/// (the written buffer is identical to the decoder output).
pub fn apply_gain(samples: &mut [i32], gain: f64) {
    if gain >= 0.999 {
        return;
    }
    for s in samples.iter_mut() {
        *s = (*s as f64 * gain)
            .round()
            .clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    }
}

/// Whole-file decode through the same conversion the player uses. Returns
/// interleaved full-scale i32 samples plus the probed source format. Used by
/// the bit-exactness tests (compare against a reference decoder byte-for-byte).
pub fn decode_all_i32(path: &str) -> Result<(Vec<i32>, SourceFormat), String> {
    let mut probed = open_source(path)?;
    let track_id = probed.track_id;
    let mut dec = probed.decoder;
    let src = probed.src;
    let mut out = Vec::new();
    while let Ok(packet) = probed.format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode: {e}")),
        };
        out.extend_from_slice(&decoded_to_i32(decoded));
    }
    Ok((out, src))
}

/// BITPERFECT iff the kernel granted exactly what the source needs — and
/// only on a direct hw: path. A plug device (default/pipewire/pulse/…) routes
/// through the OS mixer, which may resample past the ALSA boundary; claiming
/// bit-perfect there would be a lie, so it reports SHARED.
pub fn verdict_str(src: &SourceFormat, g: &Granted) -> &'static str {
    if !g.device.starts_with("hw:") {
        return "SHARED";
    }
    let rate_ok = g.rate == src.rate;
    let ch_ok = g.channels == src.channels;
    let depth_ok = g.msbits >= src.bits;
    if rate_ok && ch_ok && depth_ok {
        "BITPERFECT"
    } else if g.rate < src.rate || g.msbits < src.bits {
        "MISMATCH"
    } else {
        "CLAMPED"
    }
}

#[cfg(test)]
mod auto_tests {
    use super::*;

    fn cap(device: &str, card: &str, rmin: u32, rmax: u32, cmin: u32, cmax: u32) -> DeviceCap {
        DeviceCap {
            device: device.into(),
            card_name: card.into(),
            rate_min: rmin,
            rate_max: rmax,
            ch_min: cmin,
            ch_max: cmax,
            openable: true,
            note: String::new(),
        }
    }

    fn src() -> SourceFormat {
        SourceFormat {
            path: "fixture".into(),
            codec: "alac".into(),
            rate: 96_000,
            channels: 2,
            bits: 24,
            dur_ms: 1_000,
        }
    }

    #[test]
    fn auto_prefers_a_usb_dac_over_an_hdmi_output() {
        let caps = vec![
            cap("hw:0,3", "HDA ATI HDMI", 32_000, 192_000, 2, 8),
            cap("hw:3,3", "USB Audio", 44_100, 192_000, 2, 2),
        ];
        let picked: Vec<_> = auto_candidates(&caps, &src())
            .into_iter()
            .map(|d| d.device)
            .collect();
        assert_eq!(picked, ["hw:3,3", "hw:0,3"]);
    }

    #[test]
    fn auto_skips_unusable_and_out_of_range_devices() {
        let mut dead = cap("hw:1,0", "USB Audio", 44_100, 192_000, 2, 2);
        dead.openable = false;
        let caps = vec![
            cap("hw:2,0", "USB Audio", 44_100, 192_000, 2, 2),
            dead,
            cap("hw:4,0", "USB Audio", 44_100, 48_000, 2, 2),
            cap("hw:5,0", "USB Audio", 44_100, 192_000, 1, 1),
        ];
        let picked: Vec<_> = auto_candidates(&caps, &src())
            .into_iter()
            .map(|d| d.device)
            .collect();
        assert_eq!(picked, ["hw:2,0"]);
    }

    #[test]
    fn the_requested_buffer_is_a_duration_not_a_frame_count() {
        // 500 ms at every rate, never below four periods.
        for rate in [44100u32, 48000, 96000, 176400, 192000] {
            let frames = buffer_frames(rate, PERIOD_FRAMES);
            assert!(frames >= PERIOD_FRAMES * 4, "rate {rate}: {frames}");
            let ms = u64::from(frames) * 1000 / u64::from(rate);
            assert!((450..=550).contains(&ms), "rate {rate} buffers {ms} ms");
        }
        // The old fixed size was far too little slack at high rates.
        assert!(buffer_frames(192000, PERIOD_FRAMES) > 16384);
        // A very low rate still leaves room for four periods.
        assert_eq!(buffer_frames(8000, PERIOD_FRAMES), PERIOD_FRAMES * 4);
    }

    #[test]
    fn auto_never_selects_a_plug_device() {
        let caps = vec![cap("default", "PipeWire", 0, 384_000, 1, 8)];
        assert!(auto_candidates(&caps, &src()).is_empty());
    }
}
