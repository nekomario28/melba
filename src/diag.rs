//! Diagnostics for `melbad`.
//!
//! Two kinds of output live in this binary and they are not the same thing.
//! What a command *answers* — a receipt, a version, why a flag was refused —
//! is prose on stdout or stderr, and stays that way. What the process
//! *observed* while getting there is a diagnostic: a fallback taken, a retry,
//! a value ignored, a lookup that found nothing. Those are events here, with a
//! stable name and fields, so a bug report can be read by a person and parsed
//! by a tool without guessing at prose.
//!
//! ```text
//! melbad: level=warn event=alac_fallback reason="wrapper unreachable"
//! ```
//!
//! `MELBA_LOG=debug` raises the level, `MELBA_LOG=off` silences it, and
//! `MELBA_LOG=json` emits one JSON object per line instead. The default is
//! `warn`: these lines exist to explain a failure, not to narrate a success.
use std::fmt::Write as _;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Json,
}

struct Settings {
    /// `None` when `MELBA_LOG=off`: nothing is written.
    level: Option<Level>,
    format: Format,
}

fn settings() -> &'static Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let raw = std::env::var("MELBA_LOG").unwrap_or_default();
        let mut level = Some(Level::Warn);
        let mut format = Format::Text;
        for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "debug" => level = Some(Level::Debug),
                "info" => level = Some(Level::Info),
                "warn" | "warning" => level = Some(Level::Warn),
                "error" => level = Some(Level::Error),
                "off" | "none" => level = None,
                "json" => format = Format::Json,
                "text" => format = Format::Text,
                _ => {}
            }
        }
        Settings { level, format }
    })
}

/// Whether an event at `level` is written when `minimum` is configured.
fn enabled(level: Level, minimum: Option<Level>) -> bool {
    minimum.is_some_and(|minimum| level >= minimum)
}

/// Record an event, if the level is enabled.
pub fn event(level: Level, name: &str, fields: &[(&str, &str)]) {
    let settings = settings();
    if !enabled(level, settings.level) {
        return;
    }
    let line = match settings.format {
        Format::Text => text_line(level, name, fields),
        Format::Json => json_line(level, name, fields),
    };
    eprintln!("{line}");
}

pub fn debug(name: &str, fields: &[(&str, &str)]) {
    event(Level::Debug, name, fields);
}

pub fn info(name: &str, fields: &[(&str, &str)]) {
    event(Level::Info, name, fields);
}

pub fn warn(name: &str, fields: &[(&str, &str)]) {
    event(Level::Warn, name, fields);
}

/// `melbad: level=warn event=name key=value key="two words"`
fn text_line(level: Level, name: &str, fields: &[(&str, &str)]) -> String {
    let mut line = format!("melbad: level={} event={name}", level.as_str());
    for (key, value) in fields {
        let _ = write!(line, " {key}={}", quote(value));
    }
    line
}

/// A value is quoted when leaving it bare would change its meaning.
fn quote(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '=' | '\\'));
    if !needs_quotes {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_line(level: Level, name: &str, fields: &[(&str, &str)]) -> String {
    let mut line = format!(
        "{{\"level\":\"{}\",\"event\":{}}}",
        level.as_str(),
        json_string(name)
    );
    if !fields.is_empty() {
        line.pop();
        line.push_str(",\"fields\":{");
        for (index, (key, value)) in fields.iter().enumerate() {
            if index > 0 {
                line.push(',');
            }
            let _ = write!(line, "{}:{}", json_string(key), json_string(value));
        }
        line.push_str("}}");
    }
    line
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `off` means nothing at all, including errors; the default is `warn`.
    #[test]
    fn the_level_filter_has_an_off_position() {
        assert!(enabled(Level::Warn, Some(Level::Warn)));
        assert!(enabled(Level::Error, Some(Level::Warn)));
        assert!(!enabled(Level::Info, Some(Level::Warn)));
        assert!(!enabled(Level::Error, None), "off silences everything");
        assert!(enabled(Level::Debug, Some(Level::Debug)));
    }

    /// A field value that would otherwise change the line's shape is quoted, so
    /// `key=value` stays parseable when the value has a space in it.
    #[test]
    fn a_value_that_needs_quotes_gets_them() {
        assert_eq!(quote("hw:0,0"), "hw:0,0");
        assert_eq!(quote("wrapper unreachable"), "\"wrapper unreachable\"");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("a=b"), "\"a=b\"");
        assert_eq!(quote("says \"hi\""), "\"says \\\"hi\\\"\"");
        assert_eq!(quote("two\nlines"), "\"two\\nlines\"");
    }

    #[test]
    fn the_text_line_names_the_level_and_the_event() {
        let line = text_line(Level::Warn, "alac_fallback", &[("reason", "no wrapper")]);
        assert_eq!(
            line,
            "melbad: level=warn event=alac_fallback reason=\"no wrapper\""
        );
        assert_eq!(
            text_line(Level::Info, "queue_loaded", &[]),
            "melbad: level=info event=queue_loaded"
        );
    }

    /// JSON mode is one object per line, so a log can be read by a tool without
    /// guessing where a field ends.
    #[test]
    fn the_json_line_is_one_object() {
        let line = json_line(
            Level::Error,
            "album_unavailable",
            &[("id", "123"), ("why", "no \"such\" album")],
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(parsed["level"], "error");
        assert_eq!(parsed["event"], "album_unavailable");
        assert_eq!(parsed["fields"]["id"], "123");
        assert_eq!(parsed["fields"]["why"], "no \"such\" album");
        assert_eq!(
            json_line(Level::Info, "ready", &[]),
            "{\"level\":\"info\",\"event\":\"ready\"}"
        );
    }

    /// Control characters must not reach a terminal or a JSON document raw.
    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(json_string("a\u{1}b"), "\"a\\u0001b\"");
        assert!(text_line(Level::Warn, "e", &[("v", "a\tb")]).contains("\\t"));
    }
}
