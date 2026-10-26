//! diag <adam_id> — trace the streaming open path (no CDM needed for parse).
use melba_cdm::stream as ams;

fn main() {
    let adam = std::env::args().nth(1).expect("usage: diag <adam_id>");
    let home = std::env::var("HOME").unwrap();
    let dev = std::fs::read_to_string(format!("{home}/.cache/melba/dev-token"))
        .unwrap()
        .trim()
        .to_string();
    let user = std::fs::read_to_string(format!("{home}/.config/melba/music-user-token"))
        .unwrap()
        .trim()
        .to_string();
    let http = ams::Http {
        dev_token: dev,
        user_token: user,
        storefront: "jp".into(),
    };

    // webPlayback → ctrp playlist
    let wp = melba_cdm::WebPlayback {
        dev_token: http.dev_token.clone(),
        user_token: http.user_token.clone(),
        storefront: http.storefront.clone(),
    };
    let tr = wp.track(&adam).unwrap();
    println!("asset flavor={} m3u8={}", tr.flavor, tr.url_m3u8);
    let pl = http.get_string(&tr.url_m3u8).unwrap();
    for l in pl
        .lines()
        .filter(|l| l.contains("MAP") || l.contains("KEY") || l.contains("BYTERANGE"))
        .take(4)
    {
        println!("  {l}");
    }
    let m = ams::open_media_playlist(&http, &tr.url_m3u8).unwrap();
    let i = &m.init_info;
    println!(
        "init_info: scheme={} iv_size={} codec={} rate={} ch={} bits={} const_iv={:?} kid={:02x?}",
        i.scheme, i.iv_size, i.codec, i.rate, i.channels, i.bits, i.const_iv, i.kid
    );
    println!(
        "segs={} file={} dur_ms={}",
        m.segs.len(),
        m.file_url,
        m.dur_ms
    );
    // fetch frag0 and walk moof ourselves
    let s0 = &m.segs[0];
    let frag = http.get_range(&m.file_url, s0.offset, s0.size).unwrap();
    println!(
        "frag0 {}B  first8={:02x?}",
        frag.len(),
        &frag[..frag.len().min(16)]
    );

    // ---- ALAC: enhanced master → alac variant → parse init ----
    println!("--- ALAC ---");
    let murl = http.enhanced_hls(&adam).unwrap();
    println!("enhancedHls: {murl}");
    let mtext = http.get_string(&murl).unwrap();
    let master = ams::parse_master(&mtext, &murl).unwrap();
    println!("session keys: {:?}", master.keys.keys().collect::<Vec<_>>());
    for v in &master.variants {
        println!(
            "variant codecs={} keys={:?} uri={}",
            v.codecs,
            v.key_ids,
            &v.uri[v.uri.len().saturating_sub(60)..]
        );
    }
    let alac = master
        .variants
        .iter()
        .find(|v| v.codecs.contains("alac"))
        .expect("no alac variant");
    let m2 = ams::open_media_playlist(&http, &alac.uri).unwrap();
    let i2 = &m2.init_info;
    println!(
        "ALAC init: scheme={} iv_size={} codec={} rate={} ch={} bits={} const_iv={:?} kid={:02x?}",
        i2.scheme, i2.iv_size, i2.codec, i2.rate, i2.channels, i2.bits, i2.const_iv, i2.kid
    );
    println!(
        "ALAC segs={} dur_ms={} keys_used={:?}",
        m2.segs.len(),
        m2.dur_ms,
        m2.segs
            .iter()
            .filter_map(|s| s.key_uri.clone())
            .collect::<std::collections::HashSet<_>>()
    );
    // verify rewritten init probes clean
    std::fs::write("/tmp/alac_init.m4a", &m2.init).unwrap();
    println!("clean init written to /tmp/alac_init.m4a");
}
