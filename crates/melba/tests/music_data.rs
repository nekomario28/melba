//! Contract tests through the models consumed by Home, rows and the player bar.
use melba::api::models::Track;
use melba::mk;
use melba::player::StreamQuality;
use serde_json::json;

#[test]
fn home_and_library_songs_keep_art_without_inventing_an_album_link() {
    // Both /me/recent/played/tracks and /me/library/songs omit relationships.
    for kind in ["songs", "library-songs"] {
        let track = mk::song(&json!({
            "id": "123", "type": kind,
            "attributes": {
                "name": "Fixture", "albumName": "Fixture album",
                "artwork": {"url": "https://img/{w}x{h}{c}.{f}"}
            }
        }));
        assert!(track.album.is_none(), "do not invent a clickable album");
        assert_eq!(track.image(64), Some("https://img/64x64bb.jpg"), "{kind}");
        assert_eq!(track.image(200), Some("https://img/300x300bb.jpg"));
        assert_eq!(track.image(1000), Some("https://img/640x640bb.jpg"));
        let roundtrip: Track =
            serde_json::from_value(serde_json::to_value(&track).unwrap()).unwrap();
        assert_eq!(roundtrip.image(64), track.image(64));
    }
    // Older saved models did not have an images field.
    let old: Track = serde_json::from_value(json!({"uri": "melba:track:123"})).unwrap();
    assert!(old.image(64).is_none());
}

#[test]
fn playing_quality_does_not_label_aac_as_lossless_or_pcm_precision_as_aac_depth() {
    let mut quality = StreamQuality {
        codec: "mp4a".into(),
        sample_rate: 44100,
        bits: 32,
        sink: "default".into(),
        verdict: "SHARED".into(),
    };
    assert_eq!(quality.label().as_deref(), Some("AAC · 44.1 kHz"));
    quality.codec = "alac".into();
    quality.bits = 24;
    quality.sample_rate = 96000;
    assert_eq!(
        quality.label().as_deref(),
        Some("ALAC · LOSSLESS · 24-bit · 96 kHz")
    );
    quality.sample_rate = 0;
    assert!(quality.label().is_none());
}
