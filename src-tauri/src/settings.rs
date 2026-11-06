//! Tiny JSON settings store: <data>/settings.json, the same file (and the same keys) the Electron
//! builds used, so an update keeps everything. Unknown keys are kept as they are.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::{apps, core::version_less, log};

/// Per-app switches (Settings → Notifications → Per app, and Apps). Missing = these defaults.
pub const APP_PREF_KEYS: [&str; 6] = [
    "popups",  // pop-ups from this app
    "preview", // show the message text in them
    "chime",   // ChatDock's chime with them
    "badge",   // unread count on the tab/header/tray and the edge glow
    "sound",   // the site's own sounds
    "sleep",   // unload the app after a while unused, to save RAM
];

pub fn app_pref_default(key: &str) -> bool {
    key != "sleep"
}

fn defaults() -> Map<String, Value> {
    let apps: Map<String, Value> = apps::CATALOG.iter().map(|a| (a.id.to_string(), json!(a.enabled_by_default))).collect();
    let v = json!({
        "lang": "auto",             // 'auto' (Windows language) | 'en' | 'th' | 'zh' | 'ja' | 'de'
        "apps": apps,               // which chat services are switched on
        "width": 460,               // panel width (DIP) for apps without their own
        "widths": {},               // per-app panel width the user dragged to
        "active": "instagram",      // last app shown in the panel
        "pinned": false,            // keep the panel open when clicking elsewhere
        "hotkey": "Control+Alt+C",  // global toggle
        "side": "right",            // which screen edge the dock lives on: 'right' | 'left'
        "edgeMode": "always",       // 'always' | 'no-fullscreen' | 'off'
        "edgeHold": 0.5,            // seconds the cursor is held on the edge before the tab comes out (0 = right away)
        "edgeWheel": true,          // the mouse wheel turning on the edge = scrolling a page there: no tab
        "displayId": null,          // null = automatic, else the monitor the chat opens on (its device path)
        "displayLabel": "",         // that monitor's name, for Settings while it isn't connected
        "popupDisplay": "chat",     // pop-ups show on 'chat' (the chat's monitor) | 'mouse' | 'main'
        "glow": true,               // light strip on the screen edge when there are unread chats
        "popups": true,             // ChatDock's own always-on-top pop-up when someone messages you
        "popupText": true,          // show the message text in the pop-up (off = only who wrote)
        "popupAvatar": true,        // show the sender's profile picture
        "popupSound": false,        // soft chime with each pop-up
        "popupPosition": "top-right",
        "popupDuration": 8,         // seconds on screen; 0 = until clicked / closed
        "popupMax": 3,              // cards shown at once (the rest are summed up as "+N more")
        "appPrefs": {},
        "popupQuietFullscreen": false, // game mode: pop-ups wait while a game / video fills the screen, then one card
        "gameModeIntro": true,      // switching game mode on explains it (until "don't show this again")
        "dndUntil": 0,              // do-not-disturb: 0 = off, -1 = until switched off, else epoch ms
        "updateAutoCheck": true,    // look for new versions on GitHub
        "updateAutoDownload": true, // fetch them in the background (installing always waits for a click)
        "muted": false,             // mute sounds coming from the chat pages
        "opacity": 1,               // panel opacity (1 = solid)
        "theme": "system",          // 'system' | 'dark' | 'light'
        "hideFromCapture": false,   // exclude ChatDock's own windows from screenshots / OBS / Discord streams
        "zoom": {},                 // per-app zoom factor (missing = 100%)
        "seenCounts": {},           // per app: how much of the site's unread count the user has already seen
        "volumes": {},              // per app: its volume in ChatDock, 0-100 (100 when not set)
        "discordServers": {},       // Discord servers seen in notifications: name -> pop-ups on (this PC only)
        "discordDms": true,         // pop-ups for Discord direct and group messages
        "discordMuteKey": "",       // global hotkey that mutes / unmutes in Discord ("" = none)
        "discordDeafenKey": "",     // global hotkey that deafens / undeafens in Discord
        "quickReplies": [],         // one-click replies on pop-ups; empty = the language's three (Core::quick_replies)
        "replyKey": "",             // hotkey that opens the reply box of the newest pop-up ("" = none)
        "vips": [],                 // important people: [{ app, name }]; their pop-ups come through game mode
        "pttOn": false,             // Discord push-to-talk
        "pttKey": "",               // its key: "KeyV", "F13", "Mouse4", "Mouse5" ... ("" = not chosen yet)
        "onboarded": false
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// What settings.json holds; nothing for a new install. A file that isn't settings (cut short,
/// edited by hand) is kept as settings.json.bad and the defaults take its place; a BOM from an
/// editor is fine. A file that can't be read at all (locked for a moment at sign-in) is tried
/// again, and if it still can't be: None, and this run leaves it alone (Settings::keep_file).
fn read_saved(file: &Path) -> Option<Map<String, Value>> {
    let not_settings = |why: &str| {
        let kept = std::fs::copy(file, file.with_extension("json.bad")).is_ok();
        log!("settings.json {why}: starting from the defaults (the old file kept as settings.json.bad: {kept})");
        Some(Map::new())
    };
    for attempt in 1..=3 {
        match std::fs::read_to_string(file) {
            Ok(text) => {
                return match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
                    Ok(Value::Object(m)) => Some(m),
                    _ => not_settings("isn't settings"),
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(Map::new()),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return not_settings("isn't text"),
            Err(e) if attempt < 3 => {
                log!("settings.json not readable ({e}): trying again");
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            Err(e) => log!("settings.json not readable ({e}): the defaults for now, the file is left as it is"),
        }
    }
    None
}

/// A name as compared: composed (NFC), invisible characters dropped, trimmed, spaces squeezed,
/// lower case, and Thai written the way most keyboards type it: SARA AM as one letter (not
/// NIKHAHIT + SARA AA) after its tone mark, SARA AE as one letter (not SARA E twice).
fn name_key(name: &str) -> String {
    let visible: String =
        nfc(name).chars().filter(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}')).collect();
    let mut s = visible.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    s = s.replace("\u{0E40}\u{0E40}", "\u{0E41}").replace("\u{0E4D}\u{0E32}", "\u{0E33}");
    for tone in ['\u{0E48}', '\u{0E49}', '\u{0E4A}', '\u{0E4B}'] {
        s = s
            .replace(&format!("\u{0E4D}{tone}\u{0E32}"), &format!("{tone}\u{0E33}"))
            .replace(&format!("\u{0E33}{tone}"), &format!("{tone}\u{0E33}"));
    }
    s
}

/// Unicode NFC (Windows' own NormalizeString); the text as it is if Windows can't.
fn nfc(text: &str) -> String {
    use windows::Win32::Globalization::{NormalizationC, NormalizeString};
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return String::new();
    }
    unsafe {
        let need = NormalizeString(NormalizationC, &wide, None);
        if need <= 0 {
            return text.to_string();
        }
        let mut out = vec![0u16; need as usize];
        let got = NormalizeString(NormalizationC, &wide, Some(&mut out));
        if got <= 0 {
            return text.to_string();
        }
        String::from_utf16_lossy(&out[..got as usize])
    }
}

/// The pop-up's sender is one of the important people of that app.
pub fn vip_match(vips: &Value, app: &str, name: &str) -> bool {
    let key = name_key(name);
    !key.is_empty()
        && vips.as_array().is_some_and(|list| {
            list.iter().any(|v| {
                v.get("app").and_then(Value::as_str) == Some(app)
                    && v.get("name").and_then(Value::as_str).map(name_key).as_deref() == Some(key.as_str())
            })
        })
}

/// A settings file from before 1.7.5 with the hold time at the edge still at 3 s, the default until then.
fn old_hold_default(saved: &Map<String, Value>) -> bool {
    let last = saved.get("lastVersion").and_then(Value::as_str).unwrap_or("");
    saved.get("edgeHold").and_then(Value::as_f64) == Some(3.0) && version_less(last, "1.7.5")
}

pub struct Settings {
    data: Map<String, Value>,
    file: PathBuf,
    pub dirty: bool,
    /// settings.json couldn't be read: this run goes on the defaults and never writes over it
    keep_file: bool,
}

impl Settings {
    pub fn load(dir: &Path) -> Self {
        let file = dir.join("settings.json");
        let (saved, keep_file) = match read_saved(&file) {
            Some(m) => (m, false),
            None => (Map::new(), true),
        };
        let mut data = defaults();
        for (k, v) in &saved {
            data.insert(k.clone(), v.clone());
        }
        // nested objects: defaults plus what was saved
        let mut apps_on = defaults().remove("apps").and_then(|v| v.as_object().cloned()).unwrap_or_default();
        if let Some(Value::Object(m)) = saved.get("apps") {
            for (k, v) in m {
                apps_on.insert(k.clone(), v.clone());
            }
        }
        data.insert("apps".into(), Value::Object(apps_on));
        let saved_prefs = saved.get("appPrefs").and_then(Value::as_object).cloned().unwrap_or_default();
        let popup_apps = saved.get("popupApps").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut prefs = Map::new();
        for id in apps::ids() {
            let mut p = Map::new();
            for k in APP_PREF_KEYS {
                p.insert(k.into(), json!(app_pref_default(k)));
            }
            if let Some(Value::Object(own)) = saved_prefs.get(id) {
                for (k, v) in own {
                    if v.is_boolean() {
                        p.insert(k.clone(), v.clone());
                    }
                }
            } else if popup_apps.get(id) == Some(&json!(false)) {
                p.insert("popups".into(), json!(false)); // 1.1 only had a per-app pop-up switch
            }
            prefs.insert(id.to_string(), Value::Object(p));
        }
        data.insert("appPrefs".into(), Value::Object(prefs));
        // 1.0 and 1.1 spoke Thai only: people updating from them keep Thai; new installs follow Windows.
        if !saved.contains_key("lang") && saved.get("onboarded") == Some(&json!(true)) {
            data.insert("lang".into(), json!("th"));
        }
        // Electron builds stored a numeric display id (only ever for a monitor that isn't the main
        // one); monitors are now named by their device. It stays until it can be matched (see
        // Core::resolve_legacy_display); until then the main monitor is used.
        if !matches!(data.get("displayId"), Some(Value::String(_)) | Some(Value::Null) | Some(Value::Number(_))) {
            data.insert("displayId".into(), Value::Null);
        }
        for gone in ["notifications", "popupApps", "cookiesMigrated"] {
            data.remove(gone);
        }
        // 1.7.5: the hold time at the edge became 0.5 s. From 1.4 to 1.7.4 it was 3 s, so a 3 in a
        // file those versions wrote is almost surely that default, never picked: it moves along,
        // once (this start records the new version in lastVersion).
        let old_hold = old_hold_default(&saved);
        if old_hold {
            data.insert("edgeHold".into(), json!(0.5));
        }
        Settings { data, file, dirty: old_hold, keep_file }
    }

    pub fn get(&self, key: &str) -> &Value {
        self.data.get(key).unwrap_or(&Value::Null)
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key).as_bool().unwrap_or(false)
    }

    pub fn str(&self, key: &str) -> &str {
        self.get(key).as_str().unwrap_or("")
    }

    pub fn f64(&self, key: &str) -> f64 {
        self.get(key).as_f64().unwrap_or(0.0)
    }

    pub fn i64(&self, key: &str) -> i64 {
        self.get(key).as_i64().or_else(|| self.get(key).as_f64().map(|f| f as i64)).unwrap_or(0)
    }

    pub fn obj(&self, key: &str) -> Map<String, Value> {
        self.get(key).as_object().cloned().unwrap_or_default()
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.data.insert(key.to_string(), value);
        self.dirty = true;
    }

    /// Set one entry of an object setting (apps, widths, zoom, ...)
    pub fn set_in(&mut self, key: &str, sub: &str, value: Value) {
        let mut m = self.obj(key);
        m.insert(sub.to_string(), value);
        self.set(key, Value::Object(m));
    }

    pub fn app_pref(&self, id: &str, key: &str) -> bool {
        self.get("appPrefs").get(id).and_then(|p| p.get(key)).and_then(Value::as_bool).unwrap_or_else(|| app_pref_default(key))
    }

    pub fn app_prefs(&self, id: &str) -> Value {
        let mut p = Map::new();
        for k in APP_PREF_KEYS {
            p.insert(k.into(), json!(self.app_pref(id, k)));
        }
        Value::Object(p)
    }

    pub fn set_app_pref(&mut self, id: &str, key: &str, value: bool) {
        let mut all = self.obj("appPrefs");
        let mut own = all.get(id).and_then(Value::as_object).cloned().unwrap_or_default();
        own.insert(key.into(), json!(value));
        all.insert(id.into(), Value::Object(own));
        self.set("appPrefs", Value::Object(all));
    }

    pub fn app_on(&self, id: &str) -> bool {
        self.get("apps").get(id).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if self.keep_file {
            return; // (keep_file: the file is still the user's real settings)
        }
        let text = serde_json::to_string_pretty(&Value::Object(self.data.clone())).unwrap_or_default();
        let tmp = self.file.with_extension("json.tmp");
        let result = std::fs::create_dir_all(self.file.parent().unwrap_or(Path::new(".")))
            .and_then(|_| std::fs::write(&tmp, text))
            .and_then(|_| std::fs::rename(&tmp, &self.file));
        if let Err(err) = result {
            log!("settings save failed: {err}");
            self.dirty = true; // tried again with the next change, and at quit
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::Settings;

    /// A scratch folder of the test's own under target/ (the system temp folder may be closed to
    /// new folders, as in sandboxed runs).
    fn test_dir(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!("settings-{name}-{}", std::process::id()))
    }

    #[test]
    fn the_old_three_second_hold_moves_to_half_a_second_once() {
        let dir = test_dir("hold");
        std::fs::create_dir_all(&dir).unwrap();
        let load = |saved: Value| {
            std::fs::write(dir.join("settings.json"), saved.to_string()).unwrap();
            let s = Settings::load(&dir);
            (s.f64("edgeHold"), s.dirty)
        };
        // a new install
        let _ = std::fs::remove_file(dir.join("settings.json"));
        assert_eq!(Settings::load(&dir).f64("edgeHold"), 0.5);
        // never changed on 1.4 - 1.7.4 (or a file without a version): 0.5 s, written back
        assert_eq!(load(json!({ "edgeHold": 3, "lastVersion": "1.7.4", "onboarded": true })), (0.5, true));
        assert_eq!(load(json!({ "edgeHold": 3.0, "lastVersion": "1.4.0" })), (0.5, true));
        assert_eq!(load(json!({ "edgeHold": 3 })), (0.5, true));
        // picked by the user: kept
        assert_eq!(load(json!({ "edgeHold": 1, "lastVersion": "1.7.4" })), (1.0, false));
        assert_eq!(load(json!({ "edgeHold": 0, "lastVersion": "1.7.4" })), (0.0, false));
        assert_eq!(load(json!({ "edgeHold": 3, "lastVersion": "1.7.5" })), (3.0, false));
        assert_eq!(load(json!({ "edgeHold": 3, "lastVersion": "1.8.0" })), (3.0, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_isnt_settings_is_kept_aside_not_lost() {
        let dir = test_dir("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        // saved by an editor with a BOM: still read
        std::fs::write(&file, "\u{feff}{\"hotkey\": \"Control+Alt+Z\", \"onboarded\": true}").unwrap();
        let s = Settings::load(&dir);
        assert_eq!((s.str("hotkey"), s.bool("onboarded")), ("Control+Alt+Z", true));
        // cut short: the defaults, and the old file kept beside them
        std::fs::write(&file, "{\"hotkey\": \"Control+Alt+Z\", \"onbo").unwrap();
        let s = Settings::load(&dir);
        assert_eq!((s.str("hotkey"), s.bool("onboarded")), ("Control+Alt+C", false));
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.bad")).unwrap(), "{\"hotkey\": \"Control+Alt+Z\", \"onbo");
        // locked by another program the whole time: the defaults for now, and the file never written over
        let good = "{\"hotkey\": \"Control+Alt+Z\", \"onboarded\": true}";
        std::fs::write(&file, good).unwrap();
        let lock = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).share_mode(0).open(&file).unwrap()
        };
        let mut s = Settings::load(&dir);
        assert_eq!(s.str("hotkey"), "Control+Alt+C");
        s.set("hotkey", serde_json::json!("Control+Alt+X"));
        drop(lock);
        s.flush();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), good);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn important_people_match_however_the_name_is_written() {
        let vips = json!([{ "app": "facebook", "name": "แม่" }, { "app": "instagram", "name": "Nok Jaidee" }]);
        assert!(super::vip_match(&vips, "facebook", "แม่"));
        assert!(super::vip_match(&vips, "facebook", " แม่ "));
        assert!(super::vip_match(&vips, "instagram", "nok  jaidee"));
        assert!(!super::vip_match(&vips, "facebook", "Nok Jaidee")); // another app
        assert!(!super::vip_match(&vips, "instagram", "Nok"));
        assert!(!super::vip_match(&vips, "facebook", ""));
        // Thai with the tone mark and SARA AM typed in the other order is the same name
        assert!(super::vip_match(&json!([{ "app": "x", "name": "น\u{0E49}\u{0E33}" }]), "x", "น\u{0E33}\u{0E49}"));
        // SARA AM typed as NIKHAHIT + SARA AA (with the tone mark between, or not)
        assert!(super::vip_match(&json!([{ "app": "x", "name": "น\u{0E49}\u{0E33}" }]), "x", "น\u{0E4D}\u{0E49}\u{0E32}"));
        assert!(super::vip_match(&json!([{ "app": "x", "name": "ก\u{0E33}" }]), "x", "ก\u{0E4D}\u{0E32}"));
        // SARA E twice for SARA AE
        assert!(super::vip_match(&json!([{ "app": "x", "name": "\u{0E41}ม่" }]), "x", "\u{0E40}\u{0E40}ม่"));
        // invisible characters, and Latin letters written as a letter plus its mark
        assert!(super::vip_match(&json!([{ "app": "x", "name": "Nok" }]), "x", "N\u{200B}ok\u{200D}"));
        assert!(super::vip_match(&json!([{ "app": "x", "name": "Jos\u{00E9}" }]), "x", "Jose\u{0301}"));
    }

    #[test]
    fn the_new_settings_have_defaults() {
        let dir = test_dir("defaults18");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("settings.json"));
        let s = Settings::load(&dir);
        assert_eq!(s.get("quickReplies").as_array().map(|a| a.len()), Some(0)); // empty = the language's three
        assert_eq!((s.str("replyKey"), s.bool("pttOn"), s.str("pttKey")), ("", false, ""));
        assert_eq!(s.get("vips").as_array().map(|a| a.len()), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
