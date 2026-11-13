//! The intro card when ChatDock is started by hand: the logo, the build and a bar that fills as the
//! chats really load. When they have (or after MAX_MS at most), the card fades away and the tab
//! comes out at the edge for a moment, so you see where the chats are. It never takes the focus,
//! a click on it skips it, and it isn't shown when Windows starts ChatDock, on the first start (the
//! welcome guide shows instead), right after an update (What's new does), during a game, or with
//! Settings → General → intro off.

use serde_json::json;

use crate::{
    core::{timer, Core, PanelState, Win},
    log, rt,
    updater::own_window,
    win32::{self, Rect},
};

/// Shown at least this long (a card that blinks away reads as a glitch), and at most this long
/// (a chat that hangs or a slow network doesn't hold the tab back).
const MIN_MS: i64 = 1200;
const MAX_MS: i64 = 10_000;
/// The page's fade-out before the window goes.
const FADE_MS: u64 = 320;
/// How long the tab stays out afterwards (unless the cursor comes to it).
const TAB_MS: i64 = 3000;
const WIDTH: f64 = 280.0;
const HEIGHT: f64 = 250.0;

#[derive(Default)]
pub struct Intro {
    win: Option<Win>,
    started_at: i64,
    finishing: bool,
    tick: u64,
}

/// How far along the loading is: done / all, the chats that aren't asleep. (load states from
/// chats.rs: "loading", "ready", "error"; none yet = not started)
pub fn progress(states: &[Option<&str>]) -> (usize, usize) {
    let done = states.iter().filter(|s| matches!(s, Some("ready") | Some("error"))).count();
    (done, states.len())
}

/// Time to go? (ms since the card showed, chats done, chats in all)
pub fn finished(shown_ms: i64, done: usize, all: usize) -> bool {
    shown_ms >= MAX_MS || (shown_ms >= MIN_MS && done >= all)
}

impl Core {
    /// At the end of the start-up: show the card if this start wants one.
    pub fn maybe_intro(&mut self) {
        let wanted = !self.args.hidden
            && !self.args.selftest
            && self.settings.bool("introOnStart")
            && self.settings.bool("onboarded")
            && !win32::game_in_front()
            && !win32::is_exclusive_fullscreen();
        if !wanted {
            return;
        }
        own_window("intro".into(), "intro.html", (WIDTH, HEIGHT), false, self.args.clone(), |c, made| match made {
            Ok(w) => c.intro.win = Some(w),
            Err(err) => log!("intro: {err}"),
        });
    }

    /// The card's page is up: its texts, then it shows (centred on the monitor with the mouse).
    pub fn intro_ready(&mut self) {
        let Some(w) = self.intro.win.as_ref() else { return };
        let hwnd = w.hwnd;
        self.emit(
            "intro",
            "intro:show",
            json!([{
                "build": self.build_name(&rt::version()),
                "skip": self.t("intro.skip"),
                "loading": self.t("intro.loading"),
            }]),
        );
        let (x, y) = win32::cursor_pos();
        let d = self.display_at(x, y).unwrap_or_else(|| self.target_display());
        let (w_px, h_px) = ((WIDTH * d.ui).round() as i32, (HEIGHT * d.ui).round() as i32);
        win32::set_bounds(hwnd, Rect { x: d.work.x + (d.work.w - w_px) / 2, y: d.work.y + (d.work.h - h_px) / 2, w: w_px, h: h_px });
        win32::show_inactive(hwnd);
        win32::raise(hwnd);
        self.intro.started_at = rt::epoch_ms();
        log!("intro shown on {}", d.id);
        self.intro_tick();
    }

    fn intro_tick(&mut self) {
        if self.intro.win.is_none() || self.intro.finishing {
            return;
        }
        let states: Vec<Option<&str>> = self
            .enabled_apps()
            .into_iter()
            .filter(|id| !*self.asleep.get(*id).unwrap_or(&false))
            .map(|id| self.load_state.get(id).copied())
            .collect();
        let (done, all) = progress(&states);
        self.emit("intro", "intro:progress", json!([if all == 0 { 1.0 } else { done as f64 / all as f64 }]));
        let shown = rt::epoch_ms() - self.intro.started_at;
        if finished(shown, done, all) {
            log!("intro: {done}/{all} chats loaded after {shown} ms");
            self.intro_finish();
        } else {
            self.intro.tick = timer(150, |c| c.intro_tick());
        }
    }

    /// Loaded, timed out or clicked: the card fades, then the tab comes out.
    pub fn intro_finish(&mut self) {
        if self.intro.win.is_none() || self.intro.finishing {
            return;
        }
        self.intro.finishing = true;
        rt::cancel(self.intro.tick);
        self.emit("intro", "intro:done", json!([]));
        timer(FADE_MS, |c| {
            if let Some(w) = c.intro.win.take() {
                let _ = w.w.destroy();
            }
            c.intro.finishing = false;
            // (not if the chat was opened meanwhile, or the edge tab is switched off)
            if c.panel_state == PanelState::Hidden && c.settings.str("edgeMode") != "off" && !win32::game_in_front() {
                c.show_tab_for(TAB_MS);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{finished, progress, MAX_MS, MIN_MS};

    #[test]
    fn loading_counts_ready_and_failed_chats() {
        assert_eq!(progress(&[Some("ready"), Some("loading"), None, Some("error")]), (2, 4));
        assert_eq!(progress(&[]), (0, 0));
    }

    #[test]
    fn the_card_waits_for_the_chats_but_not_forever() {
        assert!(!finished(MIN_MS - 1, 3, 3), "never blinks away");
        assert!(finished(MIN_MS, 3, 3));
        assert!(!finished(5000, 2, 3), "still loading");
        assert!(finished(MAX_MS, 2, 3), "a hanging chat doesn't hold it");
        assert!(finished(MIN_MS, 0, 0), "nothing to load");
    }
}
