//! The chat services (and Spotify) ChatDock can host. Each one gets its own WebView2 profile (its own login).
//!   domains       pages that stay inside the panel (anything else opens in the normal browser)
//!   auth_domains  sign-in pages of other sites: as a small window ("Sign in with Google" on X) or
//!                 in the panel itself (Spotify's "Continue with Google" goes there and comes back)
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

pub fn is_auth_popup(id: &str, url: &str) -> bool {
    match (get(id), https_host(url)) {
        (Some(a), Some(host)) => host_matches(&host, a.auth_domains),
        _ => false,
    }
}

/// l.facebook.com/l.php?u=<real link> and friends: the sites' "you are leaving" redirects.
pub fn is_link_shim(url: &str) -> bool {
    let Ok(u) = Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let shim_host = ["l.", "lm."]
        .iter()
        .any(|p| host.strip_prefix(p).is_some_and(|rest| ["facebook.com", "instagram.com", "messenger.com"].contains(&rest)));
    shim_host || u.path() == "/l.php"
}

/// The real link inside a link shim, else the URL itself.
pub fn unshim(url: &str) -> String {
    if is_link_shim(url) {
        if let Ok(u) = Url::parse(url) {
            if let Some((_, inner)) = u.query_pairs().find(|(k, _)| k == "u") {
                if inner.starts_with("http://") || inner.starts_with("https://") {
                    return inner.into_owned();
                }
            }
        }
    }
    url.to_string()
}

pub fn keep_inside(id: &str, url: &str) -> bool {
    (owns(id, url) || is_auth_popup(id, url)) && !is_link_shim(url)
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

/// Origins that may show notifications from the start (they are the app's own pages).
pub fn notification_origins(id: &str) -> Vec<String> {
    let Some(a) = get(id) else { return Vec::new() };
    let mut out = Vec::new();
    if let Ok(u) = Url::parse(a.home) {
        out.push(u.origin().ascii_serialization());
    }
    for d in a.domains {
        for o in [format!("https://{d}"), format!("https://www.{d}")] {
            if !out.contains(&o) {
                out.push(o);
            }
        }
    }
    out
}
