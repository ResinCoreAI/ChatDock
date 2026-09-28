//! Moving over from the Electron builds (Beta Build 1.4 and older), once, before WebView2 first
//! starts. They kept each app's login in <data>\Partitions\<app>; WebView2 keeps it in
//! <data>\WebView2\EBWebView\WV2Profile_<app>. Both are Chromium profiles, so what keeps you logged
//! in (cookies, local storage, IndexedDB) is copied over, together with the key the cookies are
//! encrypted with (Windows-protected, for this Windows user only), and nobody has to log in again.
//! The Electron files are only read; they stay where they are.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};

use crate::{apps, log};

/// What each app's profile needs to stay logged in (paths inside a Chromium profile folder).
const KEEP: [&str; 5] = ["Network/Cookies", "Network/Cookies-journal", "Local Storage", "IndexedDB", "WebStorage"];

fn copy_tree(from: &Path, to: &Path) -> io::Result<u64> {
    if from.is_file() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        return fs::copy(from, to);
    }
    let mut bytes = 0;
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "LOCK" {
            continue; // leveldb's lock file belongs to the browser that has it open
        }
        bytes += copy_tree(&entry.path(), &to.join(name))?;
    }
    Ok(bytes)
}

fn electron_cookie_key(data_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(data_dir.join("Local State")).ok()?;
    let state: Value = serde_json::from_str(&text).ok()?;
    state.get("os_crypt")?.get("encrypted_key")?.as_str().map(str::to_string)
}

/// Copy the Electron build's logins into WebView2's profiles. Does nothing when there is no
/// Electron data, or when WebView2 has already started in this folder once. Everything is built in
/// a side folder first and renamed into place at the end, so an interrupted copy (full disk, PC
/// switched off) simply runs again next time instead of leaving half-copied profiles.
pub fn electron_logins(data_dir: &Path) {
    let old = data_dir.join("Partitions");
    let ebw: PathBuf = data_dir.join("WebView2").join("EBWebView");
    let tmp = data_dir.join("WebView2").join("EBWebView.migrating");
    if ebw.exists() {
        let _ = fs::remove_dir_all(&tmp); // left over from an interrupted run: never keep copies of cookies around
        return;
    }
    if !old.is_dir() {
        return;
    }
    let Some(key) = electron_cookie_key(data_dir) else {
        log!("electron data found but no cookie key: logins not moved");
        return;
    };
    let _ = fs::remove_dir_all(&tmp);
    if let Err(err) = fs::create_dir_all(&tmp)
        .and_then(|_| fs::write(tmp.join("Local State"), json!({ "os_crypt": { "encrypted_key": key } }).to_string()))
    {
        log!("moving logins failed: {err}");
        return;
    }
    let mut moved = Vec::new();
    for id in apps::ids() {
        let from = old.join(id);
        if !from.is_dir() {
            continue;
        }
        let to = tmp.join(format!("WV2Profile_{id}"));
        let mut bytes = 0;
        let mut failed = None;
        for part in KEEP {
            let src = from.join(part);
            if !src.exists() {
                continue;
            }
            match copy_tree(&src, &to.join(part)) {
                Ok(n) => bytes += n,
                Err(err) => {
                    failed = Some(format!("{part}: {err}"));
                    break;
                }
            }
        }
        match failed {
            // half a database is worse than none: that app starts clean (logged out)
            Some(why) => {
                let _ = fs::remove_dir_all(&to);
                log!("moving {id} failed ({why}): it starts logged out");
            }
            None => moved.push(format!("{id} {} KB", bytes / 1024)),
        }
    }
    // An antivirus scan of the new files can hold the folder for a moment: try again, then copy.
    let mut last = None;
    for attempt in 0..15u64 {
        match fs::rename(&tmp, &ebw) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(err) => {
                last = Some(err);
                std::thread::sleep(std::time::Duration::from_millis(100 + attempt * 50));
            }
        }
    }
    match last {
        None => log!("logins moved over from the Electron build: {}", moved.join(", ")),
        Some(err) => {
            match copy_tree(&tmp, &ebw) {
                Ok(_) => log!("logins moved over (copied; the rename failed: {err}): {}", moved.join(", ")),
                Err(e2) => {
                    let _ = fs::remove_dir_all(&ebw);
                    log!("moving logins failed at the end: {err} / {e2}");
                }
            }
            let _ = fs::remove_dir_all(&tmp);
        }
    }
}

/// What only made the Electron build faster (caches: Chromium makes them again). Its logins stay
/// for now, in case someone goes back to it.
const ELECTRON_CACHES: [&str; 10] = [
    "Cache",
    "Code Cache",
    "GPUCache",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "GrShaderCache",
    "GPUPersistentCache",
    "ShaderCache",
    "Service Worker/CacheStorage",
    "Service Worker/ScriptCache",
];

/// Free the space the Electron build's caches take (hundreds of MB), once its logins are in WebView2.
pub fn electron_caches(data_dir: &Path) {
    let old = data_dir.join("Partitions");
    if !old.is_dir() || !data_dir.join("WebView2").join("EBWebView").is_dir() {
        return;
    }
    let mut roots = vec![data_dir.to_path_buf()];
    if let Ok(list) = fs::read_dir(&old) {
        roots.extend(list.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    let mut removed = 0;
    for root in roots {
        for cache in ELECTRON_CACHES {
            let dir = root.join(cache);
            if dir.is_dir() && fs::remove_dir_all(&dir).is_ok() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        log!("removed {removed} cache folders of the Electron build");
    }
}

/// Downloaded installers nobody needs any more: ChatDock's own (in the temp folder) and the ones the
/// Electron builds' updater kept in %LOCALAPPDATA%\chatdock-updater. (Installed copies only.)
pub fn electron_leftovers() {
    // installers ChatDock's own updater ran (the one that just ran may still be closing: next time)
    if let Ok(list) = fs::read_dir(std::env::temp_dir()) {
        for e in list.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("ChatDock-") && name.ends_with("-update") {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return };
    let dir = local.join(if crate::core::test_product() { "chatdock-updtest-updater" } else { "chatdock-updater" });
    if dir.is_dir() {
        match fs::remove_dir_all(&dir) {
            Ok(()) => log!("removed the Electron updater's old downloads"),
            Err(err) => log!("old Electron downloads not removed yet: {err}"),
        }
    }
}
