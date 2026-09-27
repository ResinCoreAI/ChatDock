//! "Start with Windows": one value under HKCU\...\Run, the same one the Electron builds wrote
//! ("com.chatdock.app"), so the setting survives the move. The command starts ChatDock hidden.

use windows::{
    core::HSTRING,
    Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
    },
};

use crate::log;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const APP_NAME: &str = "com.chatdock.app";
const TEST_NAME: &str = "com.chatdock.updtest";

fn name() -> &'static str {
    if crate::core::test_product() {
        TEST_NAME
    } else {
        APP_NAME
    }
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
}

fn read_run() -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN),
            &HSTRING::from(name()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    if !status.is_ok() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
}

/// Task Manager's "Startup apps" can switch the entry off without deleting it.
fn approved() -> bool {
    let mut data = [0u8; 12];
    let mut size = data.len() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(APPROVED),
            &HSTRING::from(name()),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr() as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    !status.is_ok() || data[0] & 1 == 0 // 02 / 06 = on, 03 / 07 = switched off
}

pub fn get() -> bool {
    read_run().is_some() && approved()
}

fn write_run() {
    let cmd = format!("\"{}\" --hidden", exe());
    let wide: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN),
            &HSTRING::from(name()),
            REG_SZ.0,
            Some(wide.as_ptr() as *const std::ffi::c_void),
            (wide.len() * 2) as u32,
        );
    }
}

pub fn set(on: bool) {
    unsafe {
        if on {
            write_run();
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN), &HSTRING::from(name()));
        }
        // a fresh choice: whatever Task Manager had switched off no longer applies
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED), &HSTRING::from(name()));
    }
    log!("start with Windows {}", if on { "on" } else { "off" });
}

/// "Start with Windows" set up by an older copy of ChatDock (the Electron install, a portable
/// folder) points at that copy's ChatDock.exe. Point it at this one instead, so it keeps working.
pub fn migrate() {
    let Some(cmd) = read_run() else { return };
    let old = cmd.trim().trim_start_matches('"').split('"').next().unwrap_or("").to_string();
    let me = exe();
    if old.is_empty() || old.eq_ignore_ascii_case(&me) {
        return;
    }
    let same_name = std::path::Path::new(&old).file_name().map(|f| f.to_ascii_lowercase())
        == std::path::Path::new(&me).file_name().map(|f| f.to_ascii_lowercase());
    if !same_name {
        return;
    }
    write_run(); // Task Manager's on/off stays as it was
    log!("start with Windows moved from {old} to {me}");
}
