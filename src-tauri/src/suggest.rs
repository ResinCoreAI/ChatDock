//! One small HTTPS GET for the Browser's suggestions: WinHTTP on a worker thread, no cookies, a
//! short timeout, a capped answer. Only ever to the chosen engine's suggestion address.

use core::ffi::c_void;

use windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::Networking::WinHttp::*,
};

const BODY_MAX: usize = 64 * 1024;

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = WinHttpCloseHandle(self.0);
        }
    }
}

pub fn get(url: &str, timeout_ms: u32) -> Option<String> {
    let u = tauri::Url::parse(url).ok()?;
    if u.scheme() != "https" {
        return None;
    }
    let host = HSTRING::from(u.host_str()?);
    let path = HSTRING::from(match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().to_string(),
    });
    let t = timeout_ms as i32;
    unsafe {
        let session = Handle(WinHttpOpen(w!("ChatDock"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return None;
        }
        WinHttpSetTimeouts(session.0, t, t, t, t).ok()?;
        let conn = Handle(WinHttpConnect(session.0, &host, INTERNET_DEFAULT_HTTPS_PORT, 0));
        if conn.0.is_null() {
            return None;
        }
        let req =
            Handle(WinHttpOpenRequest(conn.0, w!("GET"), &path, PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if req.0.is_null() {
            return None;
        }
        let off: u32 = WINHTTP_DISABLE_COOKIES;
        let _ = WinHttpSetOption(Some(req.0), WINHTTP_OPTION_DISABLE_FEATURE, Some(&off.to_ne_bytes()));
        WinHttpSendRequest(req.0, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).ok()?;
        let mut status = 0u32;
        let mut size = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let mut read = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read).ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            if body.len() > BODY_MAX {
                return None;
            }
        }
        String::from_utf8(body).ok()
    }
}

#[cfg(test)]
mod tests {
    /// Needs the network: `cargo test suggest:: -- --ignored`
    #[test]
    #[ignore]
    fn a_real_engine_answers() {
        for engine in ["google", "bing"] {
            let body = super::get(&crate::search::suggest_url(engine, "elden", "en"), 4000).expect(engine);
            let items = crate::search::parse_suggestions(&body);
            assert!(!items.is_empty(), "{engine}: {body}");
        }
        // Thai comes back as Thai (Google answers in the page's encoding unless asked for UTF-8)
        let body = super::get(&crate::search::suggest_url("google", "ไทย", "th"), 4000).expect("thai");
        let items = crate::search::parse_suggestions(&body);
        assert!(items.iter().any(|s| s.contains("ไทย")), "{body}");
        assert_eq!(super::get("http://example.com/", 2000), None); // https only
    }
}
