//! ChatDock's own text for the Rust side (tray menu, pop-ups, update window), from the same file
//! the pages use: ui/i18n-data.js ("window.CHATDOCK_STRINGS = {...};"). A key missing in a
//! language falls back to English; {name} placeholders are filled in by t().

use std::sync::OnceLock;

use serde_json::{Map, Value};
use windows::{
    core::PWSTR,
    Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME},
};

pub const LANGS: &[(&str, &str)] = &[("en", "English"), ("th", "ไทย"), ("zh", "简体中文"), ("ja", "日本語"), ("de", "Deutsch")];

static STRINGS: OnceLock<Map<String, Value>> = OnceLock::new();

fn strings() -> &'static Map<String, Value> {
    STRINGS.get_or_init(|| {
        let src = include_str!("../../ui/i18n-data.js");
        let json = src.trim().trim_start_matches("window.CHATDOCK_STRINGS = ").trim_end_matches(';');
        serde_json::from_str(json).unwrap_or_default()
    })
}

pub fn is_lang(id: &str) -> bool {
    LANGS.iter().any(|(l, _)| *l == id)
}

pub fn t(lang: &str, key: &str, vars: &[(&str, String)]) -> String {
    let get = |l: &str| strings().get(l).and_then(|m| m.get(key)).and_then(Value::as_str);
    let mut s = get(lang).or_else(|| get("en")).unwrap_or(key).to_string();
    for (name, value) in vars {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

/// The first of the user's Windows display languages that ChatDock speaks, else English.
pub fn system_lang() -> String {
    let mut count = 0u32;
    let mut len = 0u32;
    unsafe {
        let _ = GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len);
        let mut buf = vec![0u16; len.max(1) as usize];
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, Some(PWSTR(buf.as_mut_ptr())), &mut len).is_ok() {
            for tag in String::from_utf16_lossy(&buf).split('\0').filter(|s| !s.is_empty()) {
                let base = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
                if is_lang(&base) {
                    return base;
                }
            }
        }
    }
    "en".into()
}

/// Releases are called "Beta Build 1.4" for version 1.4.0: the version without its trailing ".0".
pub fn build(version: &str) -> Option<String> {
    let mut parts = version.split('.');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    let c = c.split(['-', '+']).next().unwrap_or(c);
    if [a, b, c].iter().any(|p| p.is_empty() || !p.chars().all(|ch| ch.is_ascii_digit())) {
        return None;
    }
    Some(if c == "0" { format!("{a}.{b}") } else { format!("{a}.{b}.{c}") })
}

pub fn build_name(lang: &str, version: &str) -> String {
    match build(version) {
        Some(n) => t(lang, "build.name", &[("n", n)]),
        None => version.to_string(),
    }
}

/// The page-side locale for a language (dates, numbers).
pub fn locale(lang: &str) -> &'static str {
    match lang {
        "th" => "th-TH",
        "zh" => "zh-CN",
        "ja" => "ja-JP",
        "de" => "de-DE",
        _ => "en-US",
    }
}
