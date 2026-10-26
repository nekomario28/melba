//! tools: capture the active window via KWin ScreenShot2 (wayland-safe).
//! usage: shot <out.png>
use dbus::arg::{OwnedFd, PropMap, Variant};
use dbus::blocking::Connection;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::time::Duration;

/// One PNG chunk: length, four-byte type, payload, CRC32 of type+payload.
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// Encode tightly packed RGBA8 scanlines as a PNG. `stride` may exceed
/// `width * 4`; only the first `width` pixels of each row are real.
fn encode_png(width: u32, height: u32, stride: usize, pixels: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let row_bytes = width as usize * 4;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&pixels[y * stride..y * stride + row_bytes]);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw).expect("zlib vec write");
    let idat = encoder.finish().expect("zlib finish");

    let mut out = Vec::with_capacity(idat.len() + 64);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, deflate, no interlace
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &idat);
    write_chunk(&mut out, b"IEND", &[]);
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).expect("usage: shot <out.png>");
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err("pipe failed".into());
    }
    let (rfd, wfd) = (fds[0], fds[1]);
    // reader thread
    let rh = std::thread::spawn(move || {
        let mut f = unsafe { File::from_raw_fd(rfd) };
        let mut data = Vec::new();
        f.read_to_end(&mut data).ok();
        data
    });
    let conn = Connection::new_session()?;
    let proxy = conn.with_proxy(
        "org.kde.KWin.ScreenShot2",
        "/org/kde/KWin/ScreenShot2",
        Duration::from_secs(20),
    );
    let opts: HashMap<String, Variant<Box<dyn dbus::arg::RefArg>>> = HashMap::new();
    let w = unsafe { OwnedFd::new(wfd) };
    let (info,): (PropMap,) =
        proxy.method_call("org.kde.KWin.ScreenShot2", "CaptureActiveWindow", (opts, w))?;
    let data = rh.join().unwrap();

    let meta = |key: &str| -> Result<u32, String> {
        info.get(key)
            .and_then(|v| v.0.as_u64())
            .map(|n| n as u32)
            .ok_or_else(|| format!("ScreenShot2 gave no {key}"))
    };
    let (width, height, stride, format) = (
        meta("width")?,
        meta("height")?,
        meta("stride")?,
        meta("format")?,
    );
    for (key, value) in &info {
        eprintln!("{key}: {value:?}");
    }

    // KWin format 17/18 is RGBA8888 (straight / premultiplied); both are
    // byte-compatible for an opaque desktop window.
    if !(17..=18).contains(&format) {
        return Err(format!(
            "unhandled ScreenShot2 pixel format {format}: only RGBA8888 is encoded"
        )
        .into());
    }
    let need = stride as usize * (height as usize - 1) + width as usize * 4;
    if data.len() < need {
        return Err(format!("short capture: {} bytes for {need}", data.len()).into());
    }
    let png = encode_png(width, height, stride as usize, &data);
    File::create(&out)?.write_all(&png)?;
    eprintln!("{}x{} -> {}", width, height, out);
    Ok(())
}
