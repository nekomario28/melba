//! Links as they arrive from outside: the desktop's URL handler, the
//! command line, and a second launch handing one to the running instance.
//!
//! Apple Music share links (`https://music.apple.com/{store}/{kind}/…`),
//! the Music.app scheme shapes (`itms://`, `itmss://`, `music://`,
//! `musics://`, `apple-music://`), and the inherited `melba:` URI forms
//! all come back as the one canonical URI, `melba:<kind>:<id>` or
//! `melba:search:<encoded query>`, or nothing when it is not something
//! the app can open.

/// Resource pages. Search links carry text instead of a resource id.
const KINDS: [&str; 7] = [
    "track",
    "album",
    "artist",
    "playlist",
    "show",
    "episode",
    "prerelease",
];

/// The canonical form of a context URI, with the owner stripped.
/// Personalized playlists report their context with the owner embedded,
/// `melba:user:NAME:playlist:ID` (legacy state: `spotify:user:…`), while
/// the app's models hold the plain `melba:playlist:ID`; strict
/// comparisons need the one shape.
pub fn canonical_context_uri(uri: &str) -> String {
    let uri = wire_uri(uri);
    if let Some(rest) = uri.strip_prefix("melba:user:") {
        const PLAYLIST: &str = ":playlist:";
        if let Some(at) = rest.find(PLAYLIST) {
            let id = &rest[at + PLAYLIST.len()..];
            if !id.is_empty() && !id.contains(':') {
                return format!("melba:playlist:{id}");
            }
        }
    }
    uri
}

/// The canonical `melba:<kind>:<id>` behind `text`, or `None` when it is
/// not a link to a track, album, artist, playlist, show, episode,
/// prerelease, or search.
///
/// Accepted: `melba:track:ID` and `melba://track/ID`; the legacy
/// `spotify:`/`spotify://` scheme shapes saved state and old links may
/// still carry; `melba:user:NAME:playlist:ID`; and Apple Music links —
/// `https://music.apple.com/…` plus the Music.app schemes
/// (`itms://`, `itmss://`, `music://`, `musics://`, `apple-music://`).
/// Web links to open.spotify.com are refused: their ids are Spotify's,
/// not Apple's, and would never resolve here.
pub fn parse(text: &str) -> Option<String> {
    if let Some(query) = search_query(text) {
        return Some(format!(
            "melba:search:{}",
            percent_encoding::utf8_percent_encode(&query, percent_encoding::NON_ALPHANUMERIC)
        ));
    }
    let text = text.trim();
    let segments: Vec<&str> = if let Some(rest) = text
        .strip_prefix("melba://")
        .or_else(|| text.strip_prefix("spotify://"))
    {
        let mut segments = path_segments(rest);
        // `X://host/path` is the scheme-swapped shape of a web address;
        // `X://kind/id` carries no host.
        if segments
            .first()
            .is_some_and(|host| host.contains('.') && known_host(host))
        {
            segments.remove(0);
        }
        segments
    } else if let Some(rest) = text
        .strip_prefix("melba:")
        .or_else(|| text.strip_prefix("spotify:"))
    {
        rest.split(':').filter(|part| !part.is_empty()).collect()
    } else {
        let rest = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))
            .or_else(|| apple_scheme(text))?;
        let host = path_segments(rest)
            .first()
            .map(|host| host.to_ascii_lowercase())?;
        if host != "music.apple.com" {
            return None;
        }
        // The query is part of the link: a song shares its album's
        // address plus `?i=<song id>`.
        return apple_music(rest);
    };
    // Old playlist links carry the owner: (melba|spotify):user:NAME:playlist:ID.
    let mut segments = segments;
    if segments.len() >= 4 && segments[0] == "user" && segments[2] == "playlist" {
        segments.drain(..2);
    }
    // A web-address-shaped URI may start with a locale segment.
    if segments
        .first()
        .is_some_and(|first| first.starts_with("intl-"))
    {
        segments.remove(0);
    }
    let [kind, id, ..] = segments.as_slice() else {
        return None;
    };
    let kind = kind.to_ascii_lowercase();
    if !KINDS.contains(&kind.as_str()) || !is_id(id) {
        return None;
    }
    Some(format!("melba:{kind}:{id}"))
}

/// Decodes a search link once. A path's `+` is literal, not a form-encoded
/// space. Canonical links encode the whole query so delimiters and Unicode
/// survive command-line, D-Bus, Apple Event and line-based socket delivery.
pub fn search_query(text: &str) -> Option<String> {
    let text = text.trim();
    let encoded = if let Some(rest) = ["melba:", "spotify:"]
        .iter()
        .find_map(|scheme| text.strip_prefix(scheme))
        .filter(|rest| !rest.starts_with("//"))
    {
        let (kind, query) = rest.split_once(':')?;
        if !kind.eq_ignore_ascii_case("search") {
            return None;
        }
        query
    } else {
        let rest = ["melba://", "spotify://", "https://", "http://"]
            .iter()
            .find_map(|scheme| text.strip_prefix(scheme))?;
        let path = match rest.split_once('/') {
            // `X://kind/query` carries no host; `X://host/path` does, and
            // only for a host these links ever come from. A search query
            // is portable text, so a spotify.com search link still yields
            // one even though its track and album ids are refused.
            Some((host, path)) if host.contains('.') => {
                if known_host(host) {
                    path
                } else {
                    return None;
                }
            }
            _ => rest,
        };
        let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
        let path = if path.starts_with("intl-") {
            path.split_once('/')?.1
        } else {
            path
        };
        let (kind, query) = path.split_once('/').unwrap_or((path, ""));
        if !kind.eq_ignore_ascii_case("search") {
            return None;
        }
        query
    };
    let query = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .ok()?;
    (!query.chars().any(char::is_control)).then(|| query.into_owned())
}

/// The id inside a `melba:track:` URI, or a `spotify:track:` one from
/// legacy state or the opt-in local playback path. Other shapes return `None`.
pub fn track_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("melba:track:")
        .or_else(|| uri.strip_prefix("spotify:track:"))
}

/// The id inside a `melba:playlist:` URI, with the same legacy allowance
/// as [`track_id`].
pub fn playlist_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("melba:playlist:")
        .or_else(|| uri.strip_prefix("spotify:playlist:"))
}

/// The id inside a `melba:user:` URI, with the same legacy allowance as
/// [`track_id`].
pub fn user_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("melba:user:")
        .or_else(|| uri.strip_prefix("spotify:user:"))
}

/// The id inside a `melba:album:` URI, with the same legacy allowance as
/// [`track_id`].
pub fn album_id(uri: &str) -> Option<&str> {
    uri.strip_prefix("melba:album:")
        .or_else(|| uri.strip_prefix("spotify:album:"))
}

/// `melba:` → `spotify:` for the Spotify-facing wire: the playback
/// daemon and Spotify's own endpoints only take their own scheme.
/// Anything else passes through unchanged.
pub fn spotify_uri(uri: &str) -> String {
    uri.strip_prefix("melba:")
        .map(|rest| format!("spotify:{rest}"))
        .unwrap_or_else(|| uri.to_owned())
}

/// `spotify:` → `melba:` on an incoming URI; anything else passes through.
/// The playback daemon and the upstream endpoints speak their own scheme,
/// so wire data and pasted links arrive as `spotify:`.
pub fn wire_uri(uri: &str) -> String {
    uri.strip_prefix("spotify:")
        .map(|rest| format!("melba:{rest}"))
        .unwrap_or_else(|| uri.to_owned())
}

/// `serde` adapter for a URI field; see [`wire_uri`]. API answers carry
/// `spotify:` URIs off the wire, so model deserialization normalizes them
/// to the canonical `melba:` scheme.
pub fn wire_uri_string<'de, D>(de: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(wire_uri(&String::deserialize(de)?))
}

/// `serde` adapter for a URI field that treats null as empty; see
/// [`wire_uri_string`].
pub fn wire_uri_default<'de, D>(de: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(Option::<String>::deserialize(de)?
        .map(|uri| wire_uri(&uri))
        .unwrap_or_default())
}

/// `serde` adapter for URI lists; see [`wire_uri`].
pub fn wire_uri_vec<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(Vec::<String>::deserialize(de)?
        .into_iter()
        .map(|uri| wire_uri(&uri))
        .collect())
}

/// `serde` adapter for an optional URI; see [`wire_uri`].
pub fn wire_uri_opt<'de, D>(de: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(Option::<String>::deserialize(de)?.map(|uri| wire_uri(&uri)))
}

/// The path of a web address split at slashes, its query and fragment
/// dropped, empty segments (a trailing slash) with them.
fn path_segments(rest: &str) -> Vec<&str> {
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    rest[..end]
        .split('/')
        .filter(|part| !part.is_empty())
        .collect()
}

/// A host these link shapes genuinely carry: Apple's own, or Spotify's
/// from scheme-swapped web addresses old links and search queries may
/// still arrive in. It gates stripping only; `parse` decides whether a
/// Spotify web address itself is acceptable input.
fn known_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "music.apple.com" | "open.spotify.com" | "play.spotify.com"
    )
}

/// The schemes Music.app registers and the web player's "Open in Music"
/// emits — `itms://`, `itmss://`, `music://`, `musics://`, and
/// `apple-music://` — each carrying a `music.apple.com` path the https
/// branch then validates.
fn apple_scheme(text: &str) -> Option<&str> {
    for scheme in [
        "itms://",
        "itmss://",
        "music://",
        "musics://",
        "apple-music://",
    ] {
        if let Some(rest) = text.strip_prefix(scheme) {
            return Some(rest);
        }
    }
    None
}

/// `https://music.apple.com/{store}/{kind}/…/{id}`, the share link Apple
/// Music hands out: `/album/{slug}/{id}`, `/artist/{slug}/{id}`,
/// `/playlist/{slug}/{id}`, `/song/{id}` — and a song's own link, which
/// is the album's address plus `?i={song id}`.
fn apple_music(rest: &str) -> Option<String> {
    let (path, query) = rest
        .split_once(['?', '#'])
        .map_or((rest, ""), |(path, tail)| (path, tail));
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    // host, storefront, kind, then an optional slug and the id.
    if segments.len() < 4 {
        return None;
    }
    let (kind, id) = (segments[2], segments.last()?);
    let song = query.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(key, _)| *key == "i")
            .map(|(_, value)| value)
    });
    match kind {
        "album" => {
            if let Some(song) = song.filter(|song| is_apple_id(song)) {
                return Some(format!("melba:track:{song}"));
            }
            is_apple_id(id).then(|| format!("melba:album:{id}"))
        }
        "song" => is_apple_id(id).then(|| format!("melba:track:{id}")),
        "artist" => is_apple_id(id).then(|| format!("melba:artist:{id}")),
        "playlist" => is_apple_id(id).then(|| format!("melba:playlist:{id}")),
        _ => None,
    }
}

/// Apple ids are numeric for songs, albums, and artists, and `pl.*`/`p.*`
/// for playlists — dots, dashes, and underscores are part of the id.
fn is_apple_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// Canonical ids come from whichever catalog minted them: Spotify's
/// base62 on legacy URIs, Apple's numeric or `pl.*`/`p.*` shapes on new
/// ones — the character set is the same either way.
fn is_id(id: &str) -> bool {
    is_apple_id(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_protocol_links_keep_the_prerelease_id() {
        for link in [
            "melba:prerelease:0kRaNkRxpO16BjxJU0IQAL",
            "melba://prerelease/0kRaNkRxpO16BjxJU0IQAL",
            "spotify:prerelease:0kRaNkRxpO16BjxJU0IQAL",
            "spotify://prerelease/0kRaNkRxpO16BjxJU0IQAL",
        ] {
            assert_eq!(
                parse(link).as_deref(),
                Some("melba:prerelease:0kRaNkRxpO16BjxJU0IQAL")
            );
        }
        // Web links to Spotify carry Spotify's ids, which never resolve
        // against Apple's catalog, so they are refused outright.
        assert!(parse("https://open.spotify.com/prerelease/0kRaNkRxpO16BjxJU0IQAL").is_none());
    }

    /// Apple Music share links land on the same canonical URIs.
    #[test]
    fn apple_music_share_links_become_canonical_uris() {
        for (link, uri) in [
            (
                "https://music.apple.com/jp/album/some-album/1588049004",
                "melba:album:1588049004",
            ),
            // A song link is the album's address plus ?i=<song id>.
            (
                "https://music.apple.com/jp/album/some-album/1588049004?i=1588049005",
                "melba:track:1588049005",
            ),
            (
                "https://music.apple.com/us/song/1440903558",
                "melba:track:1440903558",
            ),
            (
                "https://music.apple.com/jp/artist/lisa/573943518",
                "melba:artist:573943518",
            ),
            (
                "https://music.apple.com/jp/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb0eb5bffc",
                "melba:playlist:pl.f4d106fed2bd41149aaacabb0eb5bffc",
            ),
        ] {
            assert_eq!(parse(link).as_deref(), Some(uri), "{link}");
        }
        // Library pages and non-resource paths are not share links.
        assert!(parse("https://music.apple.com/jp/library/songs").is_none());
        assert!(parse("https://music.apple.com/jp/browse").is_none());
        assert!(parse("https://music.apple.com").is_none());
        // The Music.app scheme shapes carry the same paths.
        for link in [
            "itms://music.apple.com/jp/album/some-album/1588049004",
            "itmss://music.apple.com/us/song/1440903558",
            "music://music.apple.com/jp/artist/lisa/573943518",
            "musics://music.apple.com/jp/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb0eb5bffc",
            "apple-music://music.apple.com/jp/album/some-album/1588049004?i=1588049005",
        ] {
            assert!(parse(link).is_some(), "{link}");
        }
        assert!(parse("musics://example.com/jp/album/x/1").is_none());
    }

    /// Every scheme shape lands on the one URI the app navigates by:
    /// canonical `melba:` links, and the `spotify:` forms the
    /// daemon's own scheme and old links may still carry.
    #[test]
    fn every_link_shape_becomes_the_one_uri() {
        // #given / #when / #then
        for (link, uri) in [
            (
                "melba:track:4uLU6hMCjMI75M1A2tKUQC",
                "melba:track:4uLU6hMCjMI75M1A2tKUQC",
            ),
            (
                "melba:playlist:pl.f4d106fed2bd41149aaacabb0eb5bffc",
                "melba:playlist:pl.f4d106fed2bd41149aaacabb0eb5bffc",
            ),
            (
                "melba:user:carmine:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "melba:playlist:37i9dQZF1DXcBWIGoYBM5M",
            ),
            (
                "melba://album/1DFixLWuPkv3KT3TnV35m3",
                "melba:album:1DFixLWuPkv3KT3TnV35m3",
            ),
            // Legacy spotify: inputs normalize to the same canonical form.
            (
                "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
                "melba:track:4uLU6hMCjMI75M1A2tKUQC",
            ),
            (
                "spotify:user:carmine:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "melba:playlist:37i9dQZF1DXcBWIGoYBM5M",
            ),
            (
                "spotify://album/1DFixLWuPkv3KT3TnV35m3",
                "melba:album:1DFixLWuPkv3KT3TnV35m3",
            ),
            (
                "spotify://open.spotify.com/artist/4Z8W4fKeB5YxbusRsdQVPb",
                "melba:artist:4Z8W4fKeB5YxbusRsdQVPb",
            ),
            (
                "  melba:Artist:4Z8W4fKeB5YxbusRsdQVPb\n",
                "melba:artist:4Z8W4fKeB5YxbusRsdQVPb",
            ),
        ] {
            assert_eq!(parse(link).as_deref(), Some(uri), "{link}");
        }
    }

    /// What is not a page here, or a foreign catalog's resource id, is
    /// refused rather than guessed at.
    #[test]
    fn what_is_not_a_page_is_refused() {
        // #given / #when / #then
        for link in [
            "",
            "melba:",
            "melba:track",
            "melba:track:",
            "melba:user:carmine",
            "melba:search:bad%0Aquery",
            "melba:station:track:4uLU6hMCjMI75M1A2tKUQC",
            "melba:local:Artist:Album:Song:180",
            "melba:track:4uLU6hMCjMI75M1A2tKUQC/../etc",
            "melba:track:a b",
            "https://example.com/track/4uLU6hMCjMI75M1A2tKUQC",
            // spotify.com resource links are refused: the ids are
            // Spotify's and cannot resolve against Apple's catalog.
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=abc123&nd=1",
            "https://open.spotify.com/intl-it/album/1DFixLWuPkv3KT3TnV35m3/",
            "https://open.spotify.com/user/carmine/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "http://play.spotify.com/show/4rOoJ6Egrf8K2IrywzwOMk",
            "https://OPEN.SPOTIFY.COM/episode/512ojhOuo1ktJprKbVcKyQ#top",
            "https://open.spotify.com/",
            "https://open.spotify.com/user/carmine",
            "https://open.spotify.com/intl-it/",
            "file:///etc/passwd",
            "4uLU6hMCjMI75M1A2tKUQC",
        ] {
            assert_eq!(parse(link), None, "{link:?}");
        }
        let long = format!("melba:track:{}", "x".repeat(65));
        assert_eq!(parse(&long), None);
    }

    #[test]
    fn search_links_preserve_the_query_across_normalization_and_delivery() {
        for (link, query) in [
            (
                "https://open.spotify.com/search/here%20comes%20the%20sun",
                "here comes the sun",
            ),
            (
                "https://open.spotify.com/intl-de/search/artist%3ABj%C3%B6rk?si=share#top",
                "artist:Björk",
            ),
            (
                "melba:search:artist:Radiohead year:1997",
                "artist:Radiohead year:1997",
            ),
            ("melba://search/%E6%9D%B1%E4%BA%AC", "東京"),
            ("spotify://open.spotify.com/search/AC%2FDC", "AC/DC"),
            ("http://play.spotify.com/search/C%2B%2B+100%25", "C+++100%"),
            ("melba:search:%2520", "%20"),
            ("https://open.spotify.com/search", ""),
            ("https://open.spotify.com/search/", ""),
            ("melba:search:", ""),
        ] {
            assert_eq!(search_query(link).as_deref(), Some(query), "{link}");
            let canonical = parse(link).unwrap();
            assert_eq!(search_query(&canonical).as_deref(), Some(query));
            assert_eq!(parse(&canonical).as_ref(), Some(&canonical));
            assert!(canonical.is_ascii() && !canonical.contains(['\n', ' ']));
        }
        for invalid in [
            "https://example.com/search/song",
            "https://open.spotify.com.evil/search/song",
            "https://user@open.spotify.com/search/song",
            "file:///search/song",
            "melba:search:bad%FFutf8",
            "https://open.spotify.com/search/line%0Abreak",
            "melba:search:zero%00byte",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }
}
