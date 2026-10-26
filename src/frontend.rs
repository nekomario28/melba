//! MPRIS application actions belong to the existing desktop instance, not
//! another media-control owner or a GUI child of the playback service.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn is_link(uri: &str) -> bool {
    uri.split_once(':').is_some_and(|(scheme, _)| {
        matches!(
            scheme,
            "melba"
                | "spotify"
                | "http"
                | "https"
                | "music"
                | "musics"
                | "apple-music"
                | "itms"
                | "itmss"
        )
    })
}

fn command(executable: Option<std::path::PathBuf>, link: Option<&str>) -> Command {
    let program = executable
        .and_then(|p| p.parent().map(|p| p.join("melba")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "melba".into());
    let mut command = Command::new(program);
    match link {
        Some(uri) => {
            command.args(["play-uri", uri]);
        }
        None => {
            command.arg("show");
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// The CLI only forwards IPC; it never opens a GUI inside melbad's service
/// cgroup (which would also kill the updater helper when the daemon stops).
pub fn forward(link: Option<&str>) -> Result<(), String> {
    let mut child = command(std::env::current_exe().ok(), link)
        .spawn()
        .map_err(|_| "Open Melba to handle this desktop action.".to_owned())?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("Open Melba to handle this desktop action.".into())
                }
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Melba did not answer the desktop action.".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_actions_are_ipc_clients_not_new_windows() {
        let show = command(None, None);
        assert_eq!(show.get_args().collect::<Vec<_>>(), ["show"]);
        let open = command(None, Some("melba:album:fixture"));
        assert_eq!(
            open.get_args().collect::<Vec<_>>(),
            ["play-uri", "melba:album:fixture"]
        );
        assert!(is_link("https://music.apple.com/album/fixture"));
        assert!(is_link("melba:playlist:p.fixture"));
        assert!(!is_link("stream:123"));
        assert!(!is_link("file:///fixture.m4a"));
    }
}
