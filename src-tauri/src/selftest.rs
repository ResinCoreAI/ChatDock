//! Self-test: `ChatDock.exe --selftest --profile=<test dir> [--shots=<dir>] [--keep] [--selftest-only=edge]`
//! It drives the running app from a thread of its own (every step reaches the app state through
//! later()) and logs what it saw, followed by "(expect …)". Run it on a test profile only: it
//! switches settings around. `--selftest-only=edge` never takes focus, so it can run during a game.

use std::{path::PathBuf, sync::mpsc, time::Duration};

use serde_json::json;
use tauri::Manager;
use webview2_com::{CapturePreviewCompletedHandler, ExecuteScriptCompletedHandler, Microsoft::Web::WebView2::Win32::*};
use windows::{
    core::HSTRING,
    Win32::{
        Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
        },
        System::Com::{STGM_CREATE, STGM_WRITE},
        UI::Shell::SHCreateStreamOnFileEx,
    },
};

use crate::{
    core::{later, Core, PanelState},
    frames, i18n, log, rt, win32,
};

pub fn start() {
    let _ = std::thread::Builder::new().name("chatdock-selftest".into()).spawn(|| {
        let only = on(|c| c.args.selftest_only.clone());
        log!("selftest: hotkey ok = {} | apps {:?}", on(|c| c.hotkey_ok), on(|c| c.enabled_apps()));
        if only.as_deref() == Some("edge") {
            wait(3000);
            edge_test(false);
        } else if only.as_deref() == Some("view") {
            wait(9000); // the chats load
            for id in on(|c| c.enabled_apps()) {
                on(move |c| c.open_panel(Some(id), "selftest"));
                wait(1500);
                view_check(&format!("40-view-{id}"));
            }
            click_check();
            close_panel();
        } else {
            full_test();
        }
        log!("selftest done");
        if !std::env::args().any(|a| a == "--keep") {
            on(|c| c.quit());
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------
fn wait(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Run f with the app state on the main thread and hand back what it returns.
fn on<R: Send + 'static>(f: impl FnOnce(&mut Core) -> R + Send + 'static) -> R {
    let (tx, rx) = mpsc::channel();
    later(move |c| {
        let _ = tx.send(f(c));
    });
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(r) => r,
        Err(_) => {
            log!("selftest: the app stopped answering (main thread stuck?)");
            std::process::exit(3);
        }
    }
}

/// A script in one of ChatDock's own pages; its result as JSON text.
fn page_js(label: &str, js: &str) -> String {
    let Some(w) = rt::app().get_webview_window(label) else { return "err:no such window".into() };
    let (tx, rx) = mpsc::channel::<String>();
    let js = js.to_string();
    let sent = w.with_webview(move |pw| unsafe {
        match pw.controller().CoreWebView2() {
            Ok(wv) => {
                let done = tx.clone();
                let handler = ExecuteScriptCompletedHandler::create(Box::new(move |_, result| {
                    let _ = done.send(result);
                    Ok(())
                }));
                if wv.ExecuteScript(&HSTRING::from(js), &handler).is_err() {
                    let _ = tx.send("err:ExecuteScript".into());
                }
            }
            Err(e) => {
                let _ = tx.send(format!("err:{e}"));
            }
        }
    });
    if sent.is_err() {
        return "err:with_webview".into();
    }
    rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| "err:timeout".into())
}

/// A script in a chat site's page; its result as JSON text.
fn view_js(id: &str, js: &str) -> String {
    let (tx, rx) = mpsc::channel::<String>();
    let (id, js) = (id.to_string(), js.to_string());
    later(move |c| {
        c.chats.execute(&id, &js, move |r| {
            let _ = tx.send(r);
        })
    });
    rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| "err:no answer".into())
}

/// Same, for an expression that gives a promise (ExecuteScript doesn't wait for promises).
fn view_js_async(id: &str, expr: &str) -> String {
    view_js(
        id,
        &format!("window.__cdTest = 'pending'; (async () => {{ try {{ window.__cdTest = JSON.stringify(await ({expr})); }} catch (e) {{ window.__cdTest = 'err:' + e.message; }} }})(); true"),
    );
    for _ in 0..20 {
        wait(150);
        let r = view_js(id, "window.__cdTest");
        if r != "\"pending\"" {
            return serde_json::from_str::<String>(&r).unwrap_or(r);
        }
    }
    "err:timeout".into()
}

fn js_click(label: &str, selector: &str) -> String {
    page_js(
        label,
        &format!("(() => {{ const el = document.querySelector({}); if (!el) return false; el.click(); return true; }})()", json!(selector)),
    )
}

unsafe fn capture(wv: &ICoreWebView2, path: PathBuf, tx: mpsc::Sender<String>) {
    let stream = match SHCreateStreamOnFileEx(&HSTRING::from(path.as_os_str()), STGM_CREATE.0 | STGM_WRITE.0, 0x80, true, None) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(format!("err:{e}"));
            return;
        }
    };
    let keep = stream.clone();
    let done = tx.clone();
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |r| {
        drop(keep); // closes the file
        let _ = done.send(if r.is_ok() { "ok".into() } else { "err:capture".into() });
        Ok(())
    }));
    if wv.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler).is_err() {
        let _ = tx.send("err:CapturePreview".into());
    }
}

/// Each visible ChatDock page (and the chat on screen) saved as <shots>/<name>-<page>.png.
fn shot(name: &str) {
    let Some(dir) = on(|c| c.args.shots.clone()) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let (pages, view) = on(|c| {
        let mut pages = Vec::new();
        for (label, hwnd) in
            [("tab", c.tab.hwnd), ("panel", c.panel.hwnd), ("glow", c.glow.hwnd), ("edge", c.edgewin.hwnd), ("toasts", c.toastwin.hwnd)]
        {
            if win32::is_visible(hwnd) {
                pages.push(label);
            }
        }
        if c.update_win.as_ref().is_some_and(|w| win32::is_visible(w.hwnd)) {
            pages.push("update");
        }
        let active = c.active();
        let view = (win32::is_visible(c.panel.hwnd) && c.chats.is_visible(&active)).then_some(active);
        (pages, view)
    });
    let mut out = Vec::new();
    for label in pages {
        let Some(w) = rt::app().get_webview_window(label) else { continue };
        let (tx, rx) = mpsc::channel::<String>();
        let path = dir.join(format!("{name}-{label}.png"));
        let _ = w.with_webview(move |pw| unsafe {
            if let Ok(wv) = pw.controller().CoreWebView2() {
                capture(&wv, path, tx);
            }
        });
        out.push(format!("{label}:{}", rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| "timeout".into())));
    }
    if let Some(id) = view {
        let (tx, rx) = mpsc::channel::<String>();
        let path = dir.join(format!("{name}-view.png"));
        later(move |c| {
            if let Some(wv) = c.chats.webview(&id) {
                unsafe { capture(&wv, path, tx) };
            }
        });
        out.push(format!("view:{}", rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| "timeout".into())));
    }
    log!("shot {name} {}", out.join(" "));
}

/// What is really on screen where the chat should be: which window a click in the middle of it
/// would hit, and how much of it is still the empty panel background (a real screen capture, not
/// the page's own rendering; black while the PC is locked).
fn view_check(name: &str) {
    let (id, panel, page, center, area, visible) = on(|c| {
        let id = c.active();
        let p = win32::window_rect(c.panel.hwnd);
        let b = c.chats.bounds(&id).unwrap_or_default();
        let area = win32::Rect { x: p.x + b.x, y: p.y + b.y, w: b.w, h: b.h };
        (id.clone(), c.panel.hwnd, c.panel_page_hwnds.clone(), (area.x + area.w / 2, area.y + area.h / 2), area, c.chats.is_visible(&id))
    });
    let top = win32::child_on_top_at(panel, center.0, center.1);
    let hit = if top == 0 {
        "nothing".to_string()
    } else if page.contains(&top) {
        "the panel page".to_string()
    } else {
        format!("a chat window ({})", win32::class_name(top))
    };
    let bg = on(|c| {
        let theme = c.settings.str("theme").to_string();
        if theme == "light" || (theme != "dark" && !win32::system_dark()) {
            (255, 255, 255)
        } else {
            (24, 25, 29)
        }
    });
    let (blank, dark) = screen_grab(name, area, bg);
    log!(
        "view {id}: panel {:?} | visible {visible} | on top at its centre: {hit} | empty panel background {blank}% of it | black {dark}% (expect Open, true, a chat window, low %; black 100% only while locked)",
        panel_state()
    );
}

/// A real click in the middle of the chat (the pointer goes straight back): the click must reach
/// the page, and the panel must stay open and in front (it hides when you click anything else).
fn click_check() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
    let (id, center, before) = on(|c| {
        let id = c.active();
        let p = win32::window_rect(c.panel.hwnd);
        let b = c.chats.bounds(&id).unwrap_or_default();
        (id, (p.x + b.x + b.w / 2, p.y + b.y + b.h / 3), c.panel_state)
    });
    if before != PanelState::Open {
        log!("click into {id}: skipped, the panel is {before:?} (something else took the focus)");
        return;
    }
    view_js(&id, "window.__cdFocus = 0; window.addEventListener('focus', () => { window.__cdFocus++; }); document.addEventListener('mousedown', () => { window.__cdDown = (window.__cdDown || 0) + 1; }, true); true");
    let was = win32::cursor_pos();
    let click = |flags| INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } } };
    unsafe {
        let _ = SetCursorPos(center.0, center.1);
        SendInput(&[click(MOUSEEVENTF_LEFTDOWN), click(MOUSEEVENTF_LEFTUP)], std::mem::size_of::<INPUT>() as i32);
    }
    wait(60);
    unsafe {
        let _ = SetCursorPos(was.0, was.1);
    }
    wait(500);
    let (state, front) = on(|c| (c.panel_state, win32::foreground_window() == c.panel.hwnd));
    log!(
        "click into {id}: page got the mouse {} | page has focus {} | panel {state:?} | panel in front {front} (expect 1, true, Open, true)",
        view_js(&id, "window.__cdDown || 0"),
        view_js(&id, "document.hasFocus()")
    );
}

/// Screen pixels of `r` saved as <shots>/<name>-screen.bmp; returns how much of it is the empty
/// panel background (dark or light theme) and how much is black, in %.
fn screen_grab(name: &str, r: win32::Rect, bg: (u8, u8, u8)) -> (u32, u32) {
    if r.w <= 0 || r.h <= 0 {
        return (0, 0);
    }
    let mut px = vec![0u8; (r.w * r.h * 4) as usize];
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, r.w, r.h);
        let old = SelectObject(mem, bmp.into());
        let _ = BitBlt(mem, 0, 0, r.w, r.h, Some(screen), r.x, r.y, SRCCOPY | CAPTUREBLT);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r.w,
                biHeight: -r.h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        GetDIBits(mem, bmp, 0, r.h as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
    let n = (r.w * r.h) as usize;
    let bg_color = bg;
    let (mut bg, mut black) = (0usize, 0usize);
    for p in px.chunks_exact(4) {
        let (b, g, rr) = (p[0], p[1], p[2]);
        if (rr, g, b) == bg_color {
            bg += 1;
        }
        if (rr, g, b) == (0, 0, 0) {
            black += 1;
        }
    }
    if let Some(dir) = on(|c| c.args.shots.clone()) {
        let _ = std::fs::create_dir_all(&dir);
        let mut file = Vec::with_capacity(54 + px.len());
        let size = 54 + px.len() as u32;
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&size.to_le_bytes());
        file.extend_from_slice(&[0, 0, 0, 0]);
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&r.w.to_le_bytes());
        file.extend_from_slice(&(-r.h).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&[0u8; 24]);
        file.extend_from_slice(&px);
        let _ = std::fs::write(dir.join(format!("{name}-screen.bmp")), file);
    }
    ((bg * 100 / n) as u32, (black * 100 / n) as u32)
}

fn toasts_snapshot() -> String {
    on(|c| json!({ "visible": c.toasts_visible(), "count": c.toasts_count() }).to_string())
}

fn panel_state() -> PanelState {
    on(|c| c.panel_state)
}

fn close_panel() {
    on(|c| {
        if c.panel_state != PanelState::Hidden {
            c.close_panel(true, "selftest");
        }
    });
    wait(600);
}

/// Frames the frame clock delivers in half a second (the screen's refresh rate with vertical blanks).
fn frame_clock() -> (u32, u32, &'static str) {
    fn step(n: u32, t0: f64, tx: mpsc::Sender<u32>) {
        frames::request(move |now| {
            if now - t0 < 500.0 {
                step(n + 1, t0, tx);
            } else {
                let _ = tx.send(((n + 1) as f64 / ((now - t0) / 1000.0)).round() as u32);
            }
        });
    }
    let (tx, rx) = mpsc::channel();
    later(move |_| step(0, rt::now_ms(), tx));
    let fps = rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0);
    let (hz, mode) = frames::info();
    (fps, hz, mode)
}

// ---------------------------------------------------------------------------------------------
// The edge: frame clock, the tab following the cursor, holding on the edge, pop-ups sliding away.
// Only the panel slide (with_panel) takes focus.
// ---------------------------------------------------------------------------------------------
fn edge_test(with_panel: bool) {
    let (fps, hz, mode) = frame_clock();
    log!("frame clock: fps {fps} | hz {hz} | mode {mode} (expect fps close to hz, mode vblank)");
    if with_panel {
        on(|c| {
            c.anim_frames = 0;
            c.open_panel(None, "selftest");
        });
        wait(260 + 150);
        let frames = on(|c| c.anim_frames);
        log!("panel slide-in: {frames} frames in 260 ms = {} fps (expect close to {hz})", (frames as f64 / 0.26).round());
        close_panel();
    }

    // A pretend cursor slides 300 px down the edge in 0.3 s: the tab must follow in small steps.
    let (edge_x, y0, height) = on(|c| {
        let b = c.target_display().bounds;
        let x = if c.on_left() { b.x } else { b.right() - 1 };
        (x, b.y + b.h / 2 - 150, b.h)
    });
    on(move |c| {
        c.edge.test_cursor = Some((edge_x, y0, false));
        c.show_tab(Some(y0));
    });
    wait(250);
    let (tx, rx) = mpsc::channel::<(u32, i32)>();
    fn follow(t0: f64, prev: i32, moves: u32, biggest: i32, edge_x: i32, y0: i32, tx: mpsc::Sender<(u32, i32)>) {
        frames::request(move |now| {
            let y = crate::core::with(|c| {
                c.edge.test_cursor = Some((edge_x, y0 + (300.0 * ((now - t0) / 300.0).min(1.0)).round() as i32, false));
                c.tab_bounds().y
            })
            .unwrap_or(prev);
            let (moves, biggest) = if y != prev { (moves + 1, biggest.max((y - prev).abs())) } else { (moves, biggest) };
            if now - t0 < 650.0 {
                follow(t0, y, moves, biggest, edge_x, y0, tx);
            } else {
                let _ = tx.send((moves, biggest));
            }
        });
    }
    later(move |c| {
        let prev = c.tab_bounds().y;
        follow(rt::now_ms(), prev, 0, 0, edge_x, y0, tx);
    });
    let (moves, biggest) = rx.recv_timeout(Duration::from_secs(5)).unwrap_or((0, 0));
    let inside = on(move |c| {
        let t = c.tab_bounds();
        let s = c.target_display().scale;
        let y = y0 + 300;
        y >= t.y + (16.0 * s) as i32 && y <= t.bottom() - (16.0 * s) as i32
    });
    log!(
        "tab follows the cursor: {moves} moves | biggest step {biggest} px | cursor inside the pill at the end {inside} (expect many moves, <= {} px, true)",
        (1000.0 / hz.max(1) as f64).ceil() as i32 * 2 + 4
    );
    on(|c| {
        c.hide_tab(true);
        c.edge.test_cursor = None;
    });
    wait(300);

    // Holding the cursor on the edge: the line grows, and the tab only comes out when the time is up.
    let hold_was = on(|c| c.settings.get("edgeHold").clone());
    let hy = on(move |c| {
        let b = c.target_display().bounds;
        c.settings.set("edgeHold", json!(1));
        b.y + (height as f64 * 0.4).round() as i32
    });
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false)));
    wait(550);
    let line_state = page_js("edge", "JSON.stringify({ mode, h: H })");
    let (line_mid, tab_mid) = on(|c| (win32::is_visible(c.edgewin.hwnd), c.edge.tab_shown));
    shot("33-hold-line");
    wait(700);
    let tab_late = on(|c| c.edge.tab_shown);
    log!("hold 1 s: at 0.55 s line shown {line_mid} {line_state} | tab {tab_mid} | at 1.25 s tab {tab_late} (expect true {{\"mode\":\"hold\",...}}, false, true)");
    on(move |c| {
        c.hide_tab(true);
        c.edge.test_cursor = Some((edge_x - 300, hy, false));
    });
    wait(500);
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, false))); // held half way, then let go
    wait(500);
    on(move |c| c.edge.test_cursor = Some((edge_x - 300, hy, false)));
    wait(60);
    let (tab, line) = on(|c| (c.edge.tab_shown, c.edge.hold_line));
    log!("hold given up half way: tab {tab} | line holding {line} (expect false, false)");
    wait(400);
    let gone = on(|c| !win32::is_visible(c.edgewin.hwnd));
    log!("line window gone after shrinking back: {gone} (expect true)");
    on(move |c| c.edge.test_cursor = Some((edge_x, hy, true))); // a game holds the mouse against the edge
    wait(1300);
    let (line, tab) = on(|c| (c.edge.hold_line, c.edge.tab_shown));
    log!("mouse held by a game: line {line} | tab {tab} (expect false, false)");
    on(move |c| {
        c.edge.test_cursor = None;
        c.settings.set("edgeHold", hold_was);
    });
    log!(
        "mouse taken by a game right now: {} | pointer hidden {} | confined {}",
        win32::mouse_captured(),
        win32::cursor_hidden(),
        win32::cursor_confined()
    );

    // A pop-up that closes slides away first; the window goes once it is gone.
    on(|c| c.test_popup());
    wait(900);
    log!("pop-up page: {}", page_js("toasts", "JSON.stringify([document.visibilityState, document.querySelectorAll('.card').length])"));
    on(|c| c.toasts_dismiss_all());
    wait(60);
    let leaving = page_js("toasts", "document.querySelectorAll('.card.leaving').length");
    let up_meanwhile = on(|c| c.toasts_visible());
    wait(500);
    let gone = on(|c| !c.toasts_visible());
    wait(600);
    let gone_late = on(|c| !c.toasts_visible());
    log!(
        "pop-up closing: sliding away {leaving} | window still up meanwhile {up_meanwhile} | window gone after {gone} | gone 1.1 s later {gone_late} (expect 1, true, true, true; with the screen locked pages don't animate, so only the last)"
    );
}

// ---------------------------------------------------------------------------------------------
// Everything else
// ---------------------------------------------------------------------------------------------
fn full_test() {
    wait(11_000); // the chats load
    for id in on(|c| c.enabled_apps()) {
        let info = on(move |c| {
            format!(
                "page {id} | url {} | load {} | firstShown {}",
                c.chats.source(id),
                c.load_state.get(id).copied().unwrap_or("?"),
                c.first_shown.get(id).copied().unwrap_or(false)
            )
        });
        log!("{info} | title {}", view_js(id, "document.title"));
    }
    log!("UA {}", view_js(on(|c| c.enabled_apps()[0]), "navigator.userAgent"));
    let m = on(|c| c.memory_stats());
    log!("RAM after start: {} MB in {} processes | per app {:?}", m.total, m.processes, m.per_app);
    for id in on(|c| c.enabled_apps()) {
        let r = view_js_async(
            id,
            "Promise.all([CredentialsContainer.prototype.get.name, CredentialsContainer.prototype.create.name, \
             window.PublicKeyCredential ? PublicKeyCredential.isConditionalMediationAvailable() : 'n/a', \
             Notification.permission, navigator.permissions.query({ name: 'notifications' }).then((p) => p.state)])",
        );
        log!("site hooks {id} {r} (expect guarded, guarded, false, granted, granted)");
    }

    if !on(|c| c.settings.bool("onboarded")) {
        // first run: finish the welcome screen
        if panel_state() == PanelState::Hidden {
            on(|c| c.open_panel(None, "selftest"));
            wait(600);
        }
        shot("01-welcome");
        on(|c| c.finish_onboarding(None));
        for id in on(|c| c.enabled_apps()) {
            on(move |c| c.set_active(id, true));
            wait(1500);
            shot(&format!("02-app-{id}"));
        }
        let vis: Vec<String> =
            on(|c| c.enabled_apps()).into_iter().map(|id| format!("{id}={}", view_js(id, "document.visibilityState"))).collect();
        log!("visibility (panel open, last app active): {}", vis.join(" "));
        close_panel();
    }
    let (state, visible) = on(|c| (c.panel_state, win32::is_visible(c.panel.hwnd)));
    log!("panel after close: {state:?} | visible {visible}");
    let vis: Vec<String> =
        on(|c| c.enabled_apps()).into_iter().map(|id| format!("{id}={}", view_js(id, "document.visibilityState"))).collect();
    log!("visibility (panel hidden): {} (expect hidden)", vis.join(" "));
    let popups_were = on(|c| c.settings.get("popups").clone());
    on(|c| c.settings.set("popups", json!(false))); // the count checks below shouldn't pop anything up

    let first = on(|c| c.enabled_apps()[0]);
    let second = on(|c| c.enabled_apps().get(1).copied().unwrap_or(c.enabled_apps()[0]));
    on(move |c| c.on_title(first, "(3) Instagram"));
    log!("count after \"(3) …\" = {} (expect 3)", on(move |c| c.counts.get(first).copied().unwrap_or(0)));
    on(move |c| c.on_title(first, "Somchai sent you a message"));
    log!("count right after flash title = {} (expect 3)", on(move |c| c.counts.get(first).copied().unwrap_or(0)));
    wait(3400);
    log!("count 3.4s after a plain title = {} (expect 0)", on(move |c| c.counts.get(first).copied().unwrap_or(0)));
    on(move |c| c.on_title(second, "(2) Facebook"));
    wait(300);
    log!("glow visible = {} {:?} (expect true)", on(|c| win32::is_visible(c.glow.hwnd)), on(|c| win32::window_rect(c.glow.hwnd)));
    shot("04-glow");

    on(|c| {
        let b = c.target_display().bounds;
        c.show_tab(Some(b.y + b.h / 2));
    });
    wait(300);
    log!(
        "tab visible = {} {:?} | glow hidden = {}",
        on(|c| win32::is_visible(c.tab.hwnd)),
        on(|c| c.tab_bounds()),
        on(|c| !win32::is_visible(c.glow.hwnd))
    );
    log!(
        "tab renderer {}",
        page_js("tab", "JSON.stringify({ cls: document.getElementById('pill').className, transform: getComputedStyle(document.getElementById('pill')).transform, visibility: document.visibilityState })")
    );
    shot("05-tab");

    // Click the last app's icon on the tab
    let last = on(|c| *c.enabled_apps().last().unwrap());
    on(move |c| c.set_active(first, false));
    wait(200);
    let clicked = js_click("tab", &format!(".app[data-app=\"{last}\"]"));
    wait(700);
    let (state, active) = on(|c| (c.panel_state, c.active()));
    log!("after clicking the tab {last} icon (clicked {clicked}): panel {state:?} | active {active} (expect Open, {last})");
    if state == PanelState::Hidden {
        on(move |c| c.open_panel(Some(last), "selftest"));
    }
    wait(300);
    log!(
        "panel: {:?} | bounds {:?} | focused {} | tab hidden {}",
        panel_state(),
        on(|c| win32::window_rect(c.panel.hwnd)),
        on(|c| win32::foreground_window() == c.panel.hwnd),
        on(|c| !win32::is_visible(c.tab.hwnd))
    );
    shot("06-open-last-app");
    view_check("06-open-last-app");
    close_panel();
    on(move |c| {
        for id in c.enabled_apps() {
            c.set_count(id, 0);
        }
        c.settings.set("popups", popups_were);
    });

    // Pop-ups: the site raises a real web notification -> ChatDock pop-up -> click -> that chat opens
    let pop_app = on(|c| if c.is_enabled("discord") { "discord" } else { c.enabled_apps()[0] });
    let avatar = format!("data:image/png;base64,{}", crate::toasts::base64(include_bytes!("../../assets/icon.png")));
    let r = view_js(
        pop_app,
        &format!(
            "(() => {{ const n = new Notification('สมชาย ใจดี', {{ body: 'ว่างไหม เย็นนี้ลงแรงค์ด้วยกัน 5 ตาพอ 🎮', icon: {}, tag: 'dm-1' }}); \
             n.onclick = () => {{ window.__cdClicked = (window.__cdClicked || 0) + 1; }}; return Notification.permission; }})()",
            json!(avatar)
        ),
    );
    wait(900);
    log!("pop-up after a site notification ({r}): {} (expect visible, count 1)", toasts_snapshot());
    shot("08-popup");
    let clicked = js_click("toasts", ".card");
    wait(900);
    let (state, active) = on(|c| (c.panel_state, c.active()));
    log!(
        "after clicking the pop-up (clicked {clicked}): panel {state:?} | active {active} | site onclick ran {} | pop-ups left {} (expect Open, {pop_app}, 1, 0)",
        view_js(pop_app, "window.__cdClicked || 0"),
        on(|c| c.toasts_count())
    );
    close_panel();

    // A page that just (re)loaded shows its old unread count: that must not pop up as "new message"
    let quiet = on(|c| if c.is_enabled("x") { "x" } else { c.enabled_apps()[0] });
    on(move |c| {
        c.load_started_at.insert(quiet.to_string(), rt::epoch_ms());
        c.on_title(quiet, "(4) Messages / X");
    });
    wait(2900);
    log!("pop-up right after a page load: {} (expect not visible)", toasts_snapshot());
    on(move |c| c.set_count(quiet, 0));

    // A site that gives no notification text: the unread count + title flash still pop something up
    on(move |c| {
        c.load_started_at.insert(quiet.to_string(), 0);
        c.last_content_at.insert(quiet.to_string(), 0);
        c.on_title(quiet, "Somchai sent you a message");
        c.on_title(quiet, "(1) Messages / X");
    });
    wait(2900);
    log!("pop-up from the unread count: {} (expect visible, count 1)", toasts_snapshot());
    shot("09-popup-fallback");
    on(move |c| c.set_count(quiet, 0));
    wait(600);
    log!("pop-up gone once read elsewhere: {} (expect not visible)", toasts_snapshot());

    // Switching an app on and off at runtime
    on(|c| c.set_app_enabled("telegram", true));
    wait(1500);
    log!(
        "telegram on: view {} | tab height {} | apps {:?}",
        on(|c| c.chats.has("telegram")),
        on(|c| c.tab_bounds().h),
        on(|c| c.enabled_apps())
    );
    on(|c| c.set_app_enabled("telegram", false));
    wait(300);
    log!("telegram off: view {} | apps {:?}", on(|c| c.chats.has("telegram")), on(|c| c.enabled_apps()));
    on(move |c| c.open_panel(Some(first), "selftest"));
    wait(600);

    // Failed load -> error screen -> retry -> recovers
    on(move |c| {
        c.set_active(first, false);
        c.chats.navigate(first, "https://chatdock-selftest.invalid/");
    });
    wait(2500);
    let (load, visible) = on(move |c| (c.load_state.get(first).copied().unwrap_or("?"), c.chats.is_visible(first)));
    log!("failed load: load state = {load} | view visible = {visible} (expect error, false)");
    shot("07-error");
    on(move |c| c.reload_app(first, false)); // what the "Try again" button does
    wait(7000);
    let (load, visible, url) =
        on(move |c| (c.load_state.get(first).copied().unwrap_or("?"), c.chats.is_visible(first), c.chats.source(first)));
    log!("after retry: load state = {load} | view visible = {visible} | url {url} (expect ready, true)");
    close_panel();
    log!(
        "panel after close: {:?} | visible {} | glow visible {}",
        panel_state(),
        on(|c| win32::is_visible(c.panel.hwnd)),
        on(|c| win32::is_visible(c.glow.hwnd))
    );

    // Tray menu (the clicks it would get) and a real global hotkey
    let tray = on(|c| {
        let before = (c.settings.bool("popups"), c.settings.bool("pinned"));
        c.on_menu("popups");
        c.on_menu("pin");
        let changed = (c.settings.bool("popups"), c.settings.bool("pinned"));
        c.on_menu("popups");
        c.on_menu("pin");
        c.on_menu("dnd:30");
        let dnd = c.dnd_active();
        c.on_menu("dnd:0");
        (crate::tray::exists(), before, changed, dnd, c.dnd_active(), (c.settings.bool("popups"), c.settings.bool("pinned")) == before)
    });
    log!(
        "tray: icon {} | switches {:?} -> {:?} | dnd on {} then {} | back as before {} (expect true, flipped, true, false, true)",
        tray.0,
        tray.1,
        tray.2,
        tray.3,
        tray.4,
        tray.5
    );
    let hotkey = on(|c| {
        let ok = c.set_pref("hotkey", json!("Control+Alt+Z"));
        let registered = c.hotkey_ok;
        c.set_pref("hotkey", json!(""));
        (ok, registered, c.hotkey_ok)
    });
    log!(
        "hotkey Ctrl+Alt+Z: accepted {} | registered {} | after turning it off {} (expect true, true, false)",
        hotkey.0,
        hotkey.1,
        hotkey.2
    );

    // Settings screen, and values it must refuse
    on(|c| c.open_settings("", "selftest"));
    wait(700);
    let (mode, state, view_hidden) = on(|c| {
        let a = c.active();
        (c.settings_mode, c.panel_state, !c.chats.is_visible(&a))
    });
    log!(
        "settings screen: open {mode} | panel {state:?} | chat view hidden {view_hidden} | page shows it {} (expect true, Open, true, true)",
        page_js("panel", "!document.getElementById('settings').hidden")
    );
    shot("10-settings");
    let refused = on(|c| {
        [
            c.set_pref("side", json!("up")),
            c.set_pref("opacity", json!(5)),
            c.set_pref("nope", json!(1)),
            c.set_pref("__proto__", json!({})),
            c.set_pref("popupMax", json!("3")),
            c.set_pref("hotkey", json!("Alt+F4")),
        ]
    });
    log!("bad values refused: {refused:?} (expect all false)");
    on(|c| c.emit("panel", "settings:goto", json!(["popups"])));
    wait(900);
    log!(
        "jump to the pop-up section: its top is at {} px (expect about 100-160)",
        page_js("panel", "Math.round(document.querySelector('[data-section=\"popups\"]').getBoundingClientRect().top)")
    );
    shot("11-settings-popups");
    for section in ["look", "security", "updates"] {
        on(move |c| c.emit("panel", "settings:goto", json!([section])));
        wait(900);
        shot(&format!("12-settings-{section}"));
    }

    // Dock on the left edge: panel, views, tab and glow all move over
    on(|c| c.set_pref("side", json!("left")));
    wait(700);
    let (wa, lb, view_x) = on(|c| {
        let a = c.active();
        (c.target_display().work, win32::window_rect(c.panel.hwnd), c.chats.bounds(&a).map(|r| r.x))
    });
    log!(
        "left side: panel x {} (expect {}) | view x {view_x:?} (expect Some(0)) | body.left {}",
        lb.x,
        wa.x,
        page_js("panel", "document.body.classList.contains('left')")
    );
    shot("13-settings-left");
    on(|c| c.close_settings());
    wait(500);
    shot("14-left-chat");
    close_panel();
    log!("left side: closed panel parked at x {} (expect {})", on(|c| win32::window_rect(c.panel.hwnd).x), wa.x - lb.w);
    let db = on(|c| c.target_display().bounds);
    on(move |c| c.show_tab(Some(db.y + db.h / 2)));
    wait(400);
    log!(
        "left side: tab x {} (expect {}) | tab mirrored {}",
        on(|c| c.tab_bounds().x),
        db.x,
        page_js("tab", "document.body.classList.contains('left')")
    );
    shot("15-left-tab");
    on(|c| c.hide_tab(true));
    wait(300);
    on(move |c| c.set_count(first, 2));
    wait(400);
    log!(
        "left side: glow x {} (expect {}) | visible {}",
        on(|c| win32::window_rect(c.glow.hwnd).x),
        db.x,
        on(|c| win32::is_visible(c.glow.hwnd))
    );
    shot("16-left-glow");
    on(move |c| c.set_count(first, 0));

    // Pop-up settings: corner, do-not-disturb, per-app switch, duration
    on(|c| {
        c.set_pref("popupPosition", json!("bottom-left"));
        c.test_popup();
    });
    wait(900);
    let tb = on(|c| c.toasts_bounds());
    log!("pop-up bottom-left: {tb:?} (expect x near {} and bottom near {})", wa.x - 4, wa.y + wa.h + 4);
    shot("17-popup-bottom-left");
    on(|c| c.toasts_dismiss_all());
    let notify_from = move || view_js(pop_app, "(() => { new Notification('ทดสอบ', { body: 'ข้อความทดสอบ' }); return true; })()");
    on(|c| c.set_dnd(30));
    notify_from();
    wait(900);
    log!("do not disturb: pop-up visible {} (expect false)", on(|c| c.toasts_visible()));
    on(move |c| {
        c.set_dnd(0);
        c.set_app_pref(pop_app, "popups", false);
    });
    notify_from();
    wait(900);
    log!("{pop_app} pop-ups off: pop-up visible {} (expect false)", on(|c| c.toasts_visible()));
    on(move |c| {
        c.set_app_pref(pop_app, "popups", true);
        c.set_pref("popupDuration", json!(5));
    });
    notify_from();
    wait(900);
    log!("pop-ups back on: visible {} (expect true)", on(|c| c.toasts_visible()));
    wait(5600);
    log!("5 s duration: still visible {} (expect false)", on(|c| c.toasts_visible()));
    on(|c| c.set_pref("popupDuration", json!(0)));
    notify_from();
    wait(6500);
    log!("\"until closed\": still visible after 6.5 s {} (expect true)", on(|c| c.toasts_visible()));
    on(|c| {
        c.toasts_dismiss_all();
        c.set_pref("popupDuration", json!(8));
        c.set_pref("popupPosition", json!("top-right"));
    });
    wait(600);

    // Update pop-up -> click -> settings opens on the update section
    on(|c| c.announce_update("9.9.9", true));
    wait(900);
    shot("18-update-popup");
    let clicked = js_click("toasts", ".card");
    wait(1200);
    let (state, mode) = on(|c| (c.panel_state, c.settings_mode));
    log!("after clicking the update pop-up (clicked {clicked}): panel {state:?} | settings {mode} (expect Open, true)");
    on(|c| {
        c.update_announced.clear();
        c.set_pref("side", json!("right"));
    });
    wait(500);
    close_panel();
    log!(
        "back on the right: panel parked at x {} | side {}",
        on(|c| win32::window_rect(c.panel.hwnd).x),
        on(|c| c.settings.str("side").to_string())
    );

    // Languages: the settings screen, the tray and pop-ups follow the chosen language
    let lang_before = on(|c| c.settings.get("lang").clone());
    on(|c| c.open_settings("", "selftest"));
    wait(700);
    for lang in ["en", "th", "zh", "ja", "de"] {
        on(move |c| c.set_pref("lang", json!(lang)));
        wait(500);
        let seen = page_js(
            "panel",
            "JSON.stringify([document.querySelector('#settings h1').textContent, document.documentElement.lang, \
             document.querySelector('[data-goto=\"popups\"]').textContent, document.querySelector('[data-text=\"memory\"]').textContent])",
        );
        log!("language {lang}: {seen} | tray says {:?}", i18n::t(lang, "tray.open", &[]));
        shot(&format!("20-settings-{lang}"));
    }
    on(|c| c.emit("panel", "settings:goto", json!(["apps"])));
    wait(900);
    shot("21-settings-apps-de");
    on(move |c| {
        c.set_pref("lang", lang_before);
        c.close_settings();
    });
    wait(300);
    close_panel();

    // Per-app switches: message text off for one app, unread count off for another
    on(|c| c.toasts_dismiss_all());
    wait(600);
    on(move |c| c.set_app_pref(pop_app, "preview", false));
    notify_from();
    wait(900);
    let body = page_js("toasts", "document.querySelector('.card .body')?.textContent || ''");
    log!("{pop_app} message text off: pop-up body {body} (expect {:?})", on(|c| c.t("toast.sentYou")));
    on(move |c| {
        c.toasts_dismiss_all();
        c.set_app_pref(pop_app, "preview", true);
        c.set_app_pref(first, "badge", false);
        c.set_count(first, 3);
    });
    wait(300);
    let (counted, shown, glow) =
        on(move |c| (c.counts.get(first).copied().unwrap_or(0), c.ui_state()["counts"][first].clone(), win32::is_visible(c.glow.hwnd)));
    log!("{first} unread count off: counted {counted} | shown {shown} | glow {glow} (expect 3, 0, false)");
    on(move |c| {
        c.set_count(first, 0);
        c.set_app_pref(first, "badge", true);
    });

    // RAM saver: an app set to sleep unloads after being unused, and wakes up when opened
    let sleeper = on(|c| {
        let a = c.active();
        c.enabled_apps().into_iter().find(|id| *id != a).unwrap_or(c.enabled_apps()[0])
    });
    let before = on(|c| c.memory_stats());
    on(move |c| {
        c.set_app_pref(sleeper, "sleep", true);
        c.last_used.insert(sleeper.to_string(), 0);
        c.sleep_check();
    });
    wait(2500);
    let after = on(|c| c.memory_stats());
    let (asleep, loaded) = on(move |c| (c.asleep.get(sleeper).copied().unwrap_or(false), c.chats.has(sleeper)));
    log!(
        "{sleeper} sleep: asleep {asleep} | page loaded {loaded} | RAM {} MB -> {} MB ({} -> {} processes) (expect true, false, less)",
        before.total,
        after.total,
        before.processes,
        after.processes
    );
    on(move |c| c.open_panel(Some(sleeper), "selftest"));
    wait(1500);
    let (asleep, loaded) = on(move |c| (c.asleep.get(sleeper).copied().unwrap_or(false), c.chats.has(sleeper)));
    log!("{sleeper} opened: asleep {asleep} | page loaded {loaded} (expect false, true)");
    on(move |c| c.set_app_pref(sleeper, "sleep", false));
    close_panel();
    let m = on(|c| c.memory_stats());
    log!("RAM now: {} MB in {} processes", m.total, m.processes);

    edge_test(true);

    // Updating: the "Updating ChatDock" window, then the start after an update
    log!(
        "build names: {} | {} | {} (expect Beta Build 1.5.1, Beta Build 1.6, Beta Build 2.0.1)",
        i18n::build_name("en", &rt::version()),
        i18n::build_name("en", "1.6.0"),
        i18n::build_name("en", "2.0.1")
    );
    on(|c| c.show_update_window("1.6.0"));
    wait(1300);
    shot("30-update-window");
    log!(
        "update window: {} | visible {}",
        page_js("update", "JSON.stringify([document.querySelector('.top b').textContent, document.getElementById('route').textContent, document.getElementById('step').textContent])"),
        on(|c| c.update_win.as_ref().is_some_and(|w| win32::is_visible(w.hwnd)))
    );
    wait(1000);
    on(|c| {
        if let Some(w) = c.update_win.take() {
            let _ = w.w.destroy();
        }
    });
    on(|c| {
        c.settings.set("whatsNew", json!({ "version": rt::version(), "from": "1.4.0", "notes": "", "at": rt::epoch_ms() }));
        c.announce_updated();
    });
    wait(900);
    shot("31-updated-popup");
    log!(
        "updated pop-up: {}",
        page_js(
            "toasts",
            "JSON.stringify([document.querySelector('.card .title')?.textContent, document.querySelector('.card .body')?.textContent])"
        )
    );
    let clicked = js_click("toasts", ".card");
    wait(1500);
    log!(
        "after clicking it (clicked {clicked}): settings {} | notes box {} (expect true, [true, what's new…])",
        on(|c| c.settings_mode),
        page_js(
            "panel",
            "JSON.stringify([!document.querySelector('.uc-notes').hidden, document.querySelector('[data-text=notesTitle]').textContent])"
        )
    );
    shot("32-whats-new");
    close_panel();
}
