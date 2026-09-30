//! Tiny JSON settings store: <data>/settings.json, the same file (and the same keys) the Electron
//! builds used, so an update keeps everything. Unknown keys are kept as they are.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::{apps, log};

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
        "edgeHold": 3,              // seconds the cursor is held on the edge before the tab comes out (0 = right away)
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
        "popupQuietFullscreen": false, // hold pop-ups while a fullscreen game / video is in front
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
        "onboarded": false
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

pub struct Settings {
    data: Map<String, Value>,
    file: PathBuf,
    pub dirty: bool,
}

impl Settings {
    pub fn load(dir: &Path) -> Self {
        let file = dir.join("settings.json");
        let saved: Map<String, Value> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
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
        Settings { data, file, dirty: false }
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
