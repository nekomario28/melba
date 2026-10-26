//! In-app Apple Music sign-in. A private WebKit profile retains the session;
//! only Apple's media-user-token cookie is captured, and Apple must accept it
//! before the saved credential is replaced. No page-to-Rust token IPC is exposed.

use std::path::PathBuf;
use std::time::{Duration, Instant};

const SIGNIN_URL: &str = "https://music.apple.com";
const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const POLL_MS: u64 = 400;

/// MusicKit's authorize redirect issues the HttpOnly media-user-token cookie.
/// Do not expose an IPC bridge to pages or frames loaded during authentication.
const INIT_JS: &str = r#"
(function poll() {
    try {
        var mk = window.MusicKit && MusicKit.getInstance && MusicKit.getInstance();
        if (mk) {
            var s = String(mk.authorizationStatus);
            if ((s === "0" || s === "notDetermined") && !window.__melbaAuthFired) {
                window.__melbaAuthFired = true;
                try { mk.authorize().catch(function() {}); } catch (e) {}
            }
        }
    } catch (e) {}
    setTimeout(poll, 800);
})();
"#;

fn data_dir() -> Result<PathBuf, String> {
    let path = crate::config::xdg_dir("XDG_DATA_HOME", ".local/share")
        .ok_or("no private sign-in data directory — set XDG_DATA_HOME or HOME")?
        .join("melba/auth-webview");
    prepare_profile(&path)?;
    Ok(path)
}

fn prepare_profile(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    crate::auth::private_dir(path)?;
    // Tighten a legacy jar too. Reject symlinks instead of changing a target
    // outside this profile; never read the stored credentials here.
    let cookie = path.join("cookies");
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(cookie)
    {
        Ok(file) => {
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("sign-in cookie jar is not a regular file".into());
            }
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(format!("private sign-in cookie jar: {e}")),
    }
}

/// Open the sign-in window and block until a token is stored or the
/// window is closed. Returns Err on webview init failure (caller may
/// fall back to the external-browser path).
#[cfg(target_os = "linux")]
pub fn run() -> Result<(), String> {
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::platform::unix::WindowExtUnix;
    use tao::window::WindowBuilder;
    use wry::{NewWindowResponse, WebContext, WebViewBuilder, WebViewBuilderExtUnix};

    // WebKitGTK's DMA-BUF renderer SIGABRTs the WebProcess on several
    // mesa/amdgpu setups; force the software path for this window.
    // (Respects an explicit user override.)
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let profile = data_dir()?;
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Melba — Apple Music sign-in")
        .with_inner_size(tao::dpi::LogicalSize::new(1024.0, 780.0))
        .build(&event_loop)
        .map_err(|e| format!("window: {e}"))?;

    let mut web_ctx = WebContext::new(Some(profile));

    let webview = WebViewBuilder::new_with_web_context(&mut web_ctx)
        .with_url(SIGNIN_URL)
        // Apple rejects account sign-in from unrecognized embedded
        // WebKit agents; present as desktop Safari.
        .with_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15")
        .with_initialization_script(INIT_JS)
        // Apple sign-in opens authorize.music.apple.com via window.open
        // and the popup postMessages back to its opener — the handshake
        // dies if we deny or hijack it, so give it a real window.
        .with_new_window_req_handler(|_, _| NewWindowResponse::Allow)
        .build_gtk(window.default_vbox().ok_or("no gtk container")?)
        .map_err(|e| format!("webview: {e}"))?;

    let deadline = Instant::now() + TIMEOUT;
    let code = std::cell::Cell::new(1i32);
    // cookie values already probed and found dead server-side — Apple's
    // sign-in flow replaces the cookie on a fresh login, so keep waiting
    // when the jar still holds a stale one.
    let mut rejected: std::collections::HashSet<String> = Default::default();

    event_loop.run(move |event, _, cf| {
        *cf = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(POLL_MS));
        // channel 2: the opaque media-user-token cookie — set by the
        // auth redirect chain even when MusicKit JS stays restricted.
        if let Ok(cookies) = webview.cookies_for_url("https://music.apple.com") {
            if let Some(c) = cookies.iter().find(|c| c.name() == "media-user-token") {
                let v = c.value().to_string();
                if v.len() > 40 && !v.contains('\n') && !rejected.contains(&v) {
                    match crate::auth::set_verified(&v, crate::musickit::validate_token) {
                        Ok(true) => {
                            eprintln!("melbad: session captured — you're signed in");
                            code.set(0);
                            *cf = ControlFlow::Exit;
                            return;
                        }
                        Ok(false) => { rejected.insert(v); }
                        Err(e) => eprintln!("melbad: could not store token: {e}"),
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            eprintln!("melbad: sign-in timed out — run `melbad auth open` again");
            *cf = ControlFlow::Exit;
            return;
        }
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            eprintln!("melbad: sign-in window closed before a token was captured — `melbad auth open --browser` watches your normal browser instead");
            *cf = ControlFlow::Exit;
        }
    });
    #[allow(unreachable_code)]
    std::process::exit(code.get())
}

#[cfg(not(target_os = "linux"))]
pub fn run() -> Result<(), String> {
    Err("in-app sign-in window is only implemented on Linux".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "melba-auth-profile-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sign_in_profile_is_private_before_webkit_opens_it() {
        let dir = Fixture::new();
        let profile = dir.0.join("auth-webview");
        prepare_profile(&profile).unwrap();
        assert_eq!(
            std::fs::metadata(&profile).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let cookie = profile.join("cookies");
        assert_eq!(
            std::fs::metadata(&cookie).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&cookie, "fake-cookie").unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&cookie, std::fs::Permissions::from_mode(0o644)).unwrap();
        prepare_profile(&profile).unwrap();
        assert_eq!(
            std::fs::metadata(&profile).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&cookie).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_to_string(cookie).unwrap(), "fake-cookie");
    }

    #[test]
    fn sign_in_profile_does_not_follow_cookie_or_directory_symlinks() {
        let dir = Fixture::new();
        let victim = dir.0.join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = dir.0.join("profile");
        std::os::unix::fs::symlink(&victim, &profile).unwrap();
        assert!(prepare_profile(&profile).is_err());
        assert_eq!(
            std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777,
            0o755
        );
        std::fs::remove_file(&profile).unwrap();
        std::fs::create_dir(&profile).unwrap();
        let file = dir.0.join("unrelated");
        std::fs::write(&file, "unchanged").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink(&file, profile.join("cookies")).unwrap();
        assert!(prepare_profile(&profile).is_err());
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(std::fs::read_to_string(file).unwrap(), "unchanged");
    }
}
