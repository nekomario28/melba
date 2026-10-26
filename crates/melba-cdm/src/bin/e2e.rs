//! End-to-end proof: manifest -> license -> decrypt N fragments -> ADTS .aac
//! Usage: e2e <adam_id> [cdm_so_path] [out.aac]
//! Reads tokens from ~/.cache/melba/dev-token and ~/.config/melba/music-user-token.

use melba_cdm::{fmp4, parse_playlist, CdmSession, WebPlayback};
use std::io::Write;
use std::path::PathBuf;

fn read_trimmed(p: &str) -> String {
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn adts(frame_len: usize) -> [u8; 7] {
    // AAC LC, 44100 Hz (idx 4), stereo (2)
    let s = frame_len + 7;
    let mut h = [0u8; 7];
    h[0] = 0xFF;
    h[1] = 0xF1;
    h[2] = ((2 - 1) << 6) | (4 << 2); // ADTS: profile-1, 44100Hz idx; ch high bits of 2ch are 0
    h[3] = ((2 & 3) << 6) | ((s >> 11) as u8);
    h[4] = ((s >> 3) & 0xFF) as u8;
    h[5] = (((s & 7) << 5) | 0x1F) as u8;
    h[6] = 0xFC;
    h
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let adam = std::env::args()
        .nth(1)
        .expect("usage: e2e <adam_id> [so] [out]");
    // A personal checkout path must not be baked into a published file: take
    // the module from the argument, or from the environment.
    let so = match std::env::args().nth(2) {
        Some(path) => path,
        None => std::env::var("WIDEVINE_CDM")
            .expect("usage: e2e <adam_id> [libwidevinecdm.so] [out] (or set WIDEVINE_CDM)"),
    };
    let out_path = std::env::args().nth(3).unwrap_or_else(|| "out.aac".into());
    let nfrags: usize = std::env::var("FRAGS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let home = std::env::var("HOME")?;
    let dev = read_trimmed(&format!("{home}/.cache/melba/dev-token"));
    let mut_ = read_trimmed(&format!("{home}/.config/melba/music-user-token"));
    let wp = WebPlayback {
        dev_token: dev,
        user_token: mut_,
        storefront: "143441-1,29 t:music14".into(),
    };

    eprintln!("[1] webPlayback manifest ...");
    let track = wp.track(&adam)?;
    eprintln!(
        "    flavor={} kid={:02x?} license={}",
        track.flavor,
        &track.kid[..4],
        track.license_url
    );

    eprintln!("[2] widevine cert ...");
    let cert = wp.widevine_cert(&track.cert_url)?;
    eprintln!("    {} bytes", cert.len());

    eprintln!("[3] playlist ...");
    let pl_text = wp.get(&track.url_m3u8)?.into_string()?;
    let pl = parse_playlist(&pl_text)?;
    eprintln!("    {} segments", pl.segments.len());
    let base = track.url_m3u8.rsplit_once('/').unwrap().0.to_string() + "/";

    eprintln!("[4] CDM session ...");
    let store = PathBuf::from(format!("{home}/.local/share/melba/cdm-store"));
    std::fs::create_dir_all(&store)?;
    let mut cdm = CdmSession::open(std::path::Path::new(&so), &store)?;
    cdm.set_server_certificate(&cert)?;
    eprintln!("    certificate accepted");

    eprintln!("[5] challenge -> license ...");
    let chal = cdm.license_challenge(&track.kid)?;
    eprintln!("    challenge {} bytes", chal.len());
    let (lic, renew) = wp.acquire_license(&track.license_url, &chal, &track.kid_data_uri, &adam)?;
    eprintln!("    license {} bytes renew-after={:?}", lic.len(), renew);
    cdm.update(&lic)?;
    eprintln!("    usable_key={}", cdm.has_usable_key());

    eprintln!("[6] fetch media mp4 + decrypt {nfrags} fragments ...");
    let mut mp4 = Vec::new();
    std::io::Read::read_to_end(
        &mut wp.get(&format!("{}{}", base, pl.map_uri))?.into_reader(),
        &mut mp4,
    )?;
    eprintln!("    media file {} bytes", mp4.len());
    let mut out = Vec::new();
    let mut frames = 0usize;
    for (fi, seg) in pl.segments.iter().take(nfrags).enumerate() {
        let seg_end = seg
            .offset
            .checked_add(seg.size as u64)
            .ok_or("segment range overflow")?;
        let frag = mp4
            .get(seg.offset as usize..seg_end as usize)
            .ok_or("segment range outside media file")?;
        let samples = fmp4::samples(frag)?;
        let mut dec_bytes = 0usize;
        for s in &samples {
            let s_end = s
                .data_off
                .checked_add(s.size as u64)
                .ok_or("sample range overflow")?;
            let data = frag
                .get(s.data_off as usize..s_end as usize)
                .ok_or("sample range outside fragment")?;
            let clear = cdm.decrypt(&track.kid, 1, &s.iv, &s.subs, data)?;
            out.extend_from_slice(&adts(clear.len()));
            out.extend_from_slice(&clear);
            dec_bytes += clear.len();
            frames += 1;
        }
        eprintln!(
            "    frag {fi}: {} samples -> {dec_bytes} clear bytes",
            samples.len()
        );
    }
    let mut f = std::fs::File::create(&out_path)?;
    f.write_all(&out)?;
    eprintln!("[done] {frames} frames -> {out_path} ({} bytes)", out.len());
    Ok(())
}
