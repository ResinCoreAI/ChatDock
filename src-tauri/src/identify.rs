//! "Show numbers on the screens" (Settings → Monitors): a card in the middle of every monitor with
//! the number the monitor map gives it, its name and size, for a few seconds; and while the mouse
//! rests on a monitor in the map (or the list), on that monitor only. The cards never take the
//! keyboard and let clicks through.

use std::collections::HashMap;

use serde_json::json;

use crate::{
    core::{timer, Core, Win},
    log, rt,
    win32::{self, Display, Rect},
};

/// the card's window, CSS px (the card itself is smaller: the rest is room for its shadow)
const CARD: (f64, f64) = (320.0, 250.0);
const ALL_MS: u64 = 2800;

#[derive(Default)]
pub struct Identify {
    /// one window per monitor, by its label ("ident-display1")
    wins: HashMap<String, Win>,
    building: Vec<String>,
    /// every card, until this timer runs out
    all: u64,
    /// the monitor the mouse is on in the map (its key), "" = none
    hover: String,
}

/// A window label for a monitor: "\\.\DISPLAY1" -> "ident-display1".
fn label_for(d: &Display) -> String {
    let name: String = d.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("ident-{}", name.to_ascii_lowercase())
}

impl Core {
    /// Every monitor's card, for a moment.
    pub fn identify_all(&mut self) {
        rt::cancel(self.identify.all);
        self.identify.all = timer(ALL_MS, |c| {
            c.identify.all = 0;
            c.identify_update();
        });
        self.identify_update();
    }

    /// The mouse is on this monitor in the map ("" = on none of them).
    pub fn identify_hover(&mut self, key: &str) {
        if self.identify.hover == key {
            return;
        }
        self.identify.hover = key.to_string();
        self.identify_update();
    }

    /// Show the cards wanted right now and hide the others; make the windows that don't exist yet.
    fn identify_update(&mut self) {
        let all = self.identify.all != 0;
        for (n, d) in self.numbered_displays() {
            let wanted = all || (!self.identify.hover.is_empty() && self.identify.hover == d.key);
            let label = label_for(&d);
            if let Some(w) = self.identify.wins.get(&label).cloned() {
                if wanted {
                    self.identify_show(&w, &label, n, &d);
                } else {
                    win32::hide(w.hwnd);
                }
            } else if wanted && !self.identify.building.contains(&label) {
                self.identify.building.push(label.clone());
                let name = label.clone();
                crate::updater::own_window(label, "identify.html", CARD, false, self.args.clone(), move |c, made| {
                    c.identify.building.retain(|l| *l != name);
                    match made {
                        Ok(w) if !c.settings_mode => {
                            let _ = w.w.destroy(); // Settings closed while it was being made
                        }
                        Ok(w) => {
                            let _ = w.w.set_ignore_cursor_events(true); // clicks go to what is under it
                            c.identify.wins.insert(name.clone(), w);
                            if c.ready.contains(&name) {
                                c.identify_update(); // its page was quicker
                            }
                        }
                        Err(err) => log!("{err}"),
                    }
                });
            }
        }
    }

    /// Its page is ready: show it if it is still wanted.
    pub fn identify_ready(&mut self, _label: &str) {
        self.identify_update();
    }

    fn identify_show(&mut self, w: &Win, label: &str, n: u32, d: &Display) {
        if !self.ready.contains(label) {
            return; // shown once its page says it is ready
        }
        let chat = self.target_display().id == d.id;
        self.emit(
            label,
            "identify:show",
            json!([{
                "n": n,
                "name": self.monitor_label(d),
                "detail": self.tv("mon.res", &[("w", d.bounds.w.to_string()), ("h", d.bounds.h.to_string()), ("hz", d.hz.to_string())]),
                "chat": chat,
                "chatText": self.t("mon.chatHere"),
                "main": d.primary,
                "mainText": self.t("mon.main"),
            }]),
        );
        let (width, height) = ((CARD.0 * d.ui).round() as i32, (CARD.1 * d.ui).round() as i32);
        let b = d.bounds;
        win32::set_bounds(w.hwnd, Rect { x: b.x + (b.w - width) / 2, y: b.y + (b.h - height) / 2, w: width, h: height });
        if !win32::is_visible(w.hwnd) {
            win32::show_inactive(w.hwnd);
        }
        win32::raise(w.hwnd);
    }

    /// The monitors changed (or the chat moved to another one): cards of monitors that are gone go,
    /// the others show again as they are now (number, place, whether the chat is there).
    pub fn identify_refresh(&mut self) {
        let now = self.numbered_displays();
        let alive: Vec<String> = now.iter().map(|(_, d)| label_for(d)).collect();
        let gone: Vec<String> = self.identify.wins.keys().filter(|l| !alive.contains(l)).cloned().collect();
        for label in gone {
            if let Some(w) = self.identify.wins.remove(&label) {
                self.ready.remove(&label);
                let _ = w.w.destroy();
            }
        }
        if !now.iter().any(|(_, d)| d.key == self.identify.hover) {
            self.identify.hover.clear();
        }
        self.identify_update();
    }

    /// Settings closed: no card stays, and the windows go.
    pub fn identify_close(&mut self) {
        rt::cancel(self.identify.all);
        self.identify.all = 0;
        self.identify.hover.clear();
        for (label, w) in self.identify.wins.drain() {
            self.ready.remove(&label);
            let _ = w.w.destroy();
        }
    }
}
