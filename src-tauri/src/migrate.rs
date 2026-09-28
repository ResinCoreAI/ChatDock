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
/// Electron data, or when WebView2 has already started in this folder once.
pub fn electron_logins(data_dir: &Path) {
    let old = data_dir.join("Partitions");
    let ebw: PathBuf = data_dir.join("WebView2").join("EBWebView");
    if !old.is_dir() || ebw.exists() {
        return;
    }
    let Some(key) = electron_cookie_key(data_dir) else {
        log!("electron data found but no cookie key: logins not moved");
        return;
    };
    if let Err(err) = fs::create_dir_all(&ebw)
        .and_then(|_| fs::write(ebw.join("Local State"), json!({ "os_crypt": { "encrypted_key": key } }).to_string()))
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
        let to = ebw.join(format!("WV2Profile_{id}"));
        let mut bytes = 0;
        for part in KEEP {
            let src = from.join(part);
            if !src.exists() {
                continue;
            }
            match copy_tree(&src, &to.join(part)) {
                Ok(n) => bytes += n,
                Err(err) => log!("moving {id} {part}: {err}"),
            }
        }
        moved.push(format!("{id} {} KB", bytes / 1024));
    }
    log!("logins moved over from the Electron build: {}", moved.join(", "));
}

/// The Electron builds' updater kept its downloaded installers in %LOCALAPPDATA%\chatdock-updater;
/// nothing uses them any more. (Installed copies only: tests never touch it.)
pub fn electron_leftovers() {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return };
    let dir = local.join(if crate::core::test_product() { "chatdock-updtest-updater" } else { "chatdock-updater" });
    if dir.is_dir() {
        match fs::remove_dir_all(&dir) {
            Ok(()) => log!("removed the Electron updater's old downloads"),
            Err(err) => log!("old Electron downloads not removed yet: {err}"),
        }
    }
}
