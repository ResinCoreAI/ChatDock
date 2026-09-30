//! The chat services (and Spotify) ChatDock can host. Each one gets its own WebView2 profile (its own login).
//!   domains       pages that stay inside the panel (anything else opens in the normal browser)
//!   auth_domains  sign-in pages of other sites: as a small window ("Sign in with Google" on X) or
//!                 in the panel itself (Spotify's "Continue with Google" goes there and comes back)
//!   media         sites that may use the mic and camera (calls, voice messages): the app's own
//!                 pages, never its CDNs or sandboxes (fbsbx.com runs other people's code)
//!   plain_title   the site's normal tab title, so other titles can be read as "someone messaged you"
//!   colors        gradient for the unread glow on the screen edge
//!   width         starting panel width for sites that need more room (Discord's sidebars)
//! LINE has no web version, so it can't be added here.

use tauri::Url;

pub struct App {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub home: &'static str,
    pub home_path: &'static str,
    pub colors: &'static [&'static str],
    pub domains: &'static [&'static str],
    pub auth_domains: &'static [&'static str],
    pub media: &'static [&'static str],
    pub width: Option<u32>,
    pub enabled_by_default: bool,
}

pub const CATALOG: &[App] = &[
    App {
        id: "instagram",
        name: "Instagram",
        icon: "instagram",
        home: "https://www.instagram.com/direct/inbox/",
        home_path: "/direct/",
        colors: &["#feda75", "#fa7e1e", "#d62976", "#962fbf"],
        domains: &["instagram.com", "cdninstagram.com", "facebook.com", "fb.com", "fbcdn.net", "facebook.net", "fbsbx.com", "meta.com"],
        auth_domains: &[],
        media: &["instagram.com"],
        width: None,
        enabled_by_default: true,
    },
    App {
        id: "facebook",
        name: "Facebook",
        icon: "messenger",
        // messenger.com was retired in April 2026; Facebook chats now live here.
        home: "https://www.facebook.com/messages/",
        home_path: "/messages",
        colors: &["#4d9bff", "#0866ff"],
        domains: &["facebook.com", "messenger.com", "fb.com", "fbcdn.net", "facebook.net", "fbsbx.com", "meta.com"],
        auth_domains: &[],
        media: &["facebook.com", "messenger.com"],
        width: None,
        enabled_by_default: true,
    },
    App {
        id: "x",
        name: "X",
        icon: "x",
        home: "https://x.com/messages",
        home_path: "/messages",
        colors: &["#ffffff", "#a1a1aa"],
        domains: &["x.com", "twitter.com", "twimg.com"],
        auth_domains: &["accounts.google.com", "appleid.apple.com"],
        media: &["x.com", "twitter.com"],
        width: None,
        enabled_by_default: true,
    },
    App {
        id: "discord",
        name: "Discord",
        icon: "discord",
        home: "https://discord.com/channels/@me",
        home_path: "/channels",
        colors: &["#8b93ff", "#5865f2"],
        domains: &["discord.com", "discordapp.com", "discordapp.net", "discord.gg", "discord.media"],
        auth_domains: &[],
        media: &["discord.com"],
        width: Some(800), // server list + channel list + chat don't fit in less
        enabled_by_default: true,
    },
    App {
        id: "telegram",
        name: "Telegram",
        icon: "telegram",
        home: "https://web.telegram.org/k/",
        home_path: "/k/",
        colors: &["#6fcbf7", "#2aabee"],
        domains: &["web.telegram.org", "telegram.org", "t.me"],
        auth_domains: &[],
        media: &["web.telegram.org"],
        width: None,
        enabled_by_default: false,
    },
    App {
        id: "whatsapp",
        name: "WhatsApp",
        icon: "whatsapp",
        home: "https://web.whatsapp.com/",
        home_path: "/",
        colors: &["#6ee89b", "#25d366"],
        domains: &["web.whatsapp.com", "whatsapp.com", "whatsapp.net"],
        auth_domains: &[],
        media: &["web.whatsapp.com"],
        width: Some(700), // chat list + conversation side by side
        enabled_by_default: false,
    },
    App {
        id: "spotify",
        name: "Spotify",
        icon: "spotify",
        home: "https://open.spotify.com/",
        home_path: "/",
        colors: &["#5ee38a", "#1db954"],
        domains: &["spotify.com", "scdn.co", "spotifycdn.com", "spotify.link"],
        auth_domains: &["accounts.google.com", "appleid.apple.com", "facebook.com"],
        media: &[],       // no calls or voice messages
        width: Some(760), // the library on the side, and the player's controls
        enabled_by_default: false,
    },
];

pub fn get(id: &str) -> Option<&'static App> {
    CATALOG.iter().find(|a| a.id == id)
}

pub fn ids() -> impl Iterator<Item = &'static str> {
    CATALOG.iter().map(|a| a.id)
}

fn host_matches(host: &str, list: &[&str]) -> bool {
    list.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

fn https_host(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    if u.scheme() != "https" {
        return None;
    }
    u.host_str().map(|h| h.to_ascii_lowercase())
}

/// Does this https URL belong to the app (so it may stay inside the panel)?
pub fn owns(id: &str, url: &str) -> bool {
    match (get(id), https_host(url)) {
        (Some(a), Some(host)) => host_matches(&host, a.domains),
        _ => false,
    }
}

/// Hosts that only serve files (pictures, video, scripts) or run other people's content in a sandbox
/// (fbsbx.com: Facebook's games and shared files). They stay inside the panel like the rest of the
/// app, but never show notifications (nor use the mic and camera: see `media`).
const CONTENT_ONLY: &[&str] = &[
    "cdninstagram.com",
    "fbcdn.net",
    "facebook.net",
    "fbsbx.com",
    "twimg.com",
    "discordapp.net",
    "scdn.co",
    "spotifycdn.com",
    "whatsapp.net",
];

/// May this page of the app show notifications (its own site, not its file and sandbox hosts)?
pub fn may_notify(id: &str, url: &str) -> bool {
    owns(id, url) && https_host(url).is_some_and(|host| !host_matches(&host, CONTENT_ONLY))
}

/// May this https page use the mic and camera (the app's calls and voice messages)?
pub fn may_use_media(id: &str, url: &str) -> bool {
    match (get(id), https_host(url)) {
        (Some(a), Some(host)) => host_matches(&host, a.media),
        _ => false,
    }
}

/// The origins of `media`, for granting (and taking back) the mic and camera in the app's profile.
pub fn media_origins(id: &str) -> Vec<String> {
    get(id).map(|a| a.media.iter().flat_map(|d| [format!("https://{d}"), format!("https://www.{d}")]).collect()).unwrap_or_default()
}

pub fn is_auth_popup(id: &str, url: &str) -> bool {
    let (Some(a), Some(host)) = (get(id), https_host(url)) else { return false };
    if !host_matches(&host, a.auth_domains) {
        return false;
    }
    // facebook.com is a sign-in only on its sign-in pages (Spotify's "Continue with Facebook");
    // any other Facebook link opens in the browser
    if host_matches(&host, &["facebook.com"]) {
        let path = Url::parse(url).map(|u| u.path().to_ascii_lowercase()).unwrap_or_default();
        return path.contains("/dialog/")
            || path.starts_with("/login")
            || path.starts_with("/checkpoint")
            || path.starts_with("/two_step_verification");
    }
    true
}

/// l.facebook.com/l.php?u=<real link> and friends: Meta's "you are leaving" redirects. Only on
/// Meta's own hosts: any other site's /l.php is just a page.
pub fn is_link_shim(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let meta = ["facebook.com", "instagram.com", "messenger.com"];
    let shim_host = ["l.", "lm."].iter().any(|p| host.strip_prefix(p).is_some_and(|rest| meta.contains(&rest)));
    shim_host || (u.path() == "/l.php" && host_matches(&host, &meta))
}

/// The real link inside a link shim (read as an address and written back in its standard form, so
/// no raw quotes or spaces reach Windows), else the URL itself.
pub fn unshim(url: &str) -> String {
    if is_link_shim(url) {
        if let Ok(u) = Url::parse(url) {
            if let Some((_, inner)) = u.query_pairs().find(|(k, _)| k == "u") {
                if let Ok(real) = Url::parse(&inner) {
                    if real.scheme() == "http" || real.scheme() == "https" {
                        return real.to_string();
                    }
                }
            }
        }
    }
    url.to_string()
}

pub fn keep_inside(id: &str, url: &str) -> bool {
    (owns(id, url) || is_auth_popup(id, url)) && !is_link_shim(url)
}

/// Programs a clicked link may open: e-mail and the apps' own desktop apps. Nothing else, so a
/// link can never start one of Windows' riskier handlers (ms-msdt:, search-ms:, …).
pub const APP_SCHEMES: [&str; 5] = ["mailto", "spotify", "discord", "tg", "whatsapp"];

pub fn app_scheme(url: &str) -> bool {
    // The address goes into the program's command line as "%1": a quote would end it and add
    // arguments of its own, and without quotes a space would. (Web addresses arrive escaped.)
    if url.chars().any(|c| c == '"' || c.is_whitespace() || c.is_control()) {
        return false;
    }
    let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
    APP_SCHEMES.contains(&scheme.as_str())
}

/// An address on this PC (localhost, *.localhost, 127.x, ::1, ::, 0.0.0.0, however it is written):
/// the chat sites have no business with programs listening here.
pub fn is_this_pc(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else { return false };
    let Some(host) = u.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let v4 = match ip {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped(),
        };
        return ip.is_loopback() || ip.is_unspecified() || v4.is_some_and(|v| v.is_loopback() || v.is_unspecified());
    }
    host == "localhost" || host.ends_with(".localhost")
}

/// Voice / video call pages open in their own small window.
pub fn is_call_url(id: &str, url: &str) -> bool {
    if !owns(id, url) {
        return false;
    }
    let Ok(u) = Url::parse(url) else { return false };
    u.path_segments()
        .map(|mut segs| segs.any(|s| ["videocall", "groupcall", "call", "calls", "rtc"].contains(&s.to_ascii_lowercase().as_str())))
        .unwrap_or(false)
}

/// The site's normal title (not a "someone messaged you" flash).
pub fn plain_title(id: &str, title: &str) -> bool {
    let t = title.to_lowercase();
    match id {
        "instagram" => t.contains("instagram"),
        "facebook" => t.contains("facebook") || t.contains("messenger"),
        "x" => {
            let trimmed = title.trim_end();
            let ends_with_x = trimmed.ends_with('X') || trimmed.ends_with('x');
            let before = trimmed.chars().rev().nth(1);
            (ends_with_x && (trimmed.len() == 1 || matches!(before, Some(' ') | Some('/')))) || t.contains("twitter")
        }
        "discord" => t.contains("discord"),
        "telegram" => t.contains("telegram"),
        "whatsapp" => t.contains("whatsapp"),
        "spotify" => true, // the song playing
        _ => false,
    }
}

/// Origins that may show notifications from the start: the app's own pages, not its file and
/// sandbox hosts. (The mic and camera: media_origins.)
pub fn notification_origins(id: &str) -> Vec<String> {
    let Some(a) = get(id) else { return Vec::new() };
    let mut out = Vec::new();
    if let Ok(u) = Url::parse(a.home) {
        out.push(u.origin().ascii_serialization());
    }
    for d in a.domains.iter().filter(|d| !CONTENT_ONLY.contains(d)) {
        for o in [format!("https://{d}"), format!("https://www.{d}")] {
            if !out.contains(&o) {
                out.push(o);
            }
        }
    }
    out
}

/// The file and sandbox hosts' origins that versions before 1.7.3 let show notifications.
pub fn content_only_origins(id: &str) -> Vec<String> {
    let Some(a) = get(id) else { return Vec::new() };
    a.domains.iter().filter(|d| CONTENT_ONLY.contains(d)).flat_map(|d| [format!("https://{d}"), format!("https://www.{d}")]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_only_from_the_apps_own_pages() {
        assert!(may_notify("instagram", "https://www.instagram.com/direct/inbox/"));
        assert!(may_notify("facebook", "https://www.facebook.com/messages/"));
        assert!(may_notify("discord", "https://discord.com/channels/@me"));
        // file and sandbox hosts (Facebook runs other people's games and files on fbsbx.com)
        assert!(!may_notify("facebook", "https://apps-123.apps.fbsbx.com/game"));
        assert!(!may_notify("instagram", "https://scontent.cdninstagram.com/v/t51.jpg"));
        assert!(!may_notify("discord", "https://media.discordapp.net/attachments/1.png"));
        assert!(!may_notify("spotify", "https://i.scdn.co/image/1"));
        // not the app's at all, or not https
        assert!(!may_notify("discord", "https://discord.com.evil.example/"));
        assert!(!may_notify("instagram", "http://www.instagram.com/"));
    }

    #[test]
    fn content_hosts_get_no_standing_permissions() {
        let granted = notification_origins("facebook");
        assert!(granted.contains(&"https://www.facebook.com".to_string()));
        assert!(!granted.iter().any(|o| o.contains("fbsbx.com") || o.contains("fbcdn.net")));
        assert!(content_only_origins("facebook").contains(&"https://www.fbsbx.com".to_string()));
        assert!(content_only_origins("x").iter().all(|o| o.contains("twimg.com")));
    }

    #[test]
    fn link_shims_only_on_metas_hosts_and_in_standard_form() {
        let fb = "https://l.facebook.com/l.php?u=https%3A%2F%2Fexample.com%2Fa%20b%22c&h=x";
        assert!(is_link_shim(fb));
        assert_eq!(unshim(fb), "https://example.com/a%20b%22c");
        assert!(is_link_shim("https://www.facebook.com/l.php?u=https%3A%2F%2Fexample.com"));
        assert!(!is_link_shim("https://evil.example/l.php?u=https%3A%2F%2Fx.com")); // not Meta's
        assert_eq!(unshim("https://evil.example/l.php?u=https%3A%2F%2Fx.com"), "https://evil.example/l.php?u=https%3A%2F%2Fx.com");
        assert_eq!(
            unshim("https://l.facebook.com/l.php?u=file%3A%2F%2F%2FC%3A%2Fx"),
            "https://l.facebook.com/l.php?u=file%3A%2F%2F%2FC%3A%2Fx"
        );
    }

    #[test]
    fn addresses_on_this_pc_however_they_are_written() {
        for url in [
            "http://127.0.0.1:6463/rpc",
            "http://127.0.0.2:8080/",
            "https://localhost/",
            "http://evil.localhost:3000/",
            "http://[::1]/",
            "http://0.0.0.0:8000/",
            "http://2130706433/",
            "http://[::ffff:127.0.0.1]/",
            "http://localhost./",
            "https://127.1/",
            "http://[::]/",
        ] {
            assert!(is_this_pc(url), "{url}");
        }
        for url in [
            "https://example.com/?next=http://localhost",
            "http://192.168.1.1/",
            "https://localhost.example.com/",
            "https://127.example.com/",
            "mailto:a@b.c",
        ] {
            assert!(!is_this_pc(url), "{url}");
        }
    }

    #[test]
    fn only_email_and_the_apps_own_programs() {
        assert!(
            app_scheme("mailto:a@b.c")
                && app_scheme("spotify:track:1")
                && app_scheme("discord://-/channels/1")
                && app_scheme("TG://resolve")
        );
        assert!(
            !app_scheme("ms-msdt:/id PCWDiagnostic")
                && !app_scheme("search-ms:query=x")
                && !app_scheme("file:///C:/x")
                && !app_scheme("https://x.com")
        );
        // a quote or a space would add arguments to the program's command line
        assert!(!app_scheme("spotify:x\" --gpu-launcher=\"calc") && !app_scheme("discord://x y") && !app_scheme("tg://x\ty"));
    }

    #[test]
    fn mic_and_camera_only_on_the_apps_own_call_pages() {
        assert!(may_use_media("facebook", "https://www.facebook.com/groupcall/ROOM"));
        assert!(may_use_media("instagram", "https://www.instagram.com/direct/t/1/"));
        assert!(may_use_media("discord", "https://discord.com/channels/@me"));
        assert!(!may_use_media("facebook", "https://www.fbsbx.com/x")); // other people's code
        assert!(!may_use_media("instagram", "https://scontent.cdninstagram.com/v.mp4"));
        assert!(!may_use_media("discord", "https://cdn.discordapp.com/x"));
        assert!(!may_use_media("spotify", "https://open.spotify.com/"));
        assert!(!may_use_media("facebook", "http://www.facebook.com/")); // https only
        assert!(media_origins("spotify").is_empty());
        assert!(media_origins("facebook").contains(&"https://www.facebook.com".to_string()));
    }
}
