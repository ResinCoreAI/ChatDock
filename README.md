<div align="center">

<img src="assets/icon.png" width="96" alt="ChatDock icon">

# ChatDock

**Your chats, docked on the edge of the screen — on top of your game.**

Instagram · Facebook · X · Discord (+ Telegram, WhatsApp) in one panel that hides on the right or left
edge of the screen, slides out when you need it, and pops up who messaged you — even over a
borderless game.

[![Latest release](https://img.shields.io/github/v/release/ResinCoreAI/ChatDock?label=download&color=8b5cf6&display_name=release)](https://github.com/ResinCoreAI/ChatDock/releases/latest)
![Windows 10/11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078d4)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

<img src="docs/demo.gif" width="880" alt="Messages pop up over a game, a click opens the chat, the dock moves to the left edge">

</div>

> [!NOTE]
> ChatDock speaks **English, ไทย, 简体中文, 日本語 and Deutsch** (it follows Windows, or pick one in Settings).
> The chat sites inside it use whatever language your accounts are set to.

## Why

Reading a message while you play usually means Alt+Tab, a browser, and a game that loses focus
(or a stray click that fires a gun). ChatDock keeps the real chat sites right at the edge of the screen:

- **Hidden until you want it** – nothing on screen until you hold the cursor on the screen edge (a line
  grows along it, so a quick flick of the mouse to the side never opens anything).
- **Always on top** – the panel and the pop-ups float over borderless / windowed-fullscreen games.
- **Never steals the game** – the edge tab and pop-ups don't take focus; closing the panel hands
  focus straight back to the game, no extra click.
- **Know who wrote without looking away** – a pop-up shows the sender's picture, name and message.

## Features

| | |
|---|---|
| 💬 **Chat apps** | Instagram, Facebook, X, Discord on by default; Telegram and WhatsApp one switch away. Each app keeps its own login. |
| ↔️ **Left or right** | Dock on either screen edge (and on any monitor). The panel, edge tab and unread glow all follow. |
| 🖱️ **Open it your way** | Hold the cursor on the edge (a line grows to the top and bottom; 3 s by default, adjustable) → white tab → click (or click an app icon on the tab). Or the global hotkey <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>C</kbd>, the tray icon, or a pop-up. |
| 🔔 **Pop-ups over games** | Sender picture + name + message, stacked, click to open that exact conversation. |
| ⚙️ **Fully adjustable pop-ups** | Any screen corner · how long they stay (or until closed) · how many at once · show/hide message text · show/hide pictures · chime · do-not-disturb timer · stay quiet while a fullscreen game or video is in front. |
| 🎚️ **Per-app notifications** | For each app on its own: pop-ups, message text, chime, unread count & glow, and the site’s own sounds. |
| 🧠 **Light on RAM** | See how much RAM each app uses; let apps you rarely open **sleep** when unused (they wake up when you open them). ChatDock’s own screens share one process. |
| 🌐 **Five languages** | English, ไทย, 简体中文, 日本語, Deutsch — automatic from Windows or picked in Settings. |
| 🔒 **Security** | Encrypted cookies, sandboxed pages, isolated logins, locked-down app files. See [Privacy & security](#privacy--security). |
| ⬆️ **Updates from GitHub** | Checks for new releases, downloads them in the background, and installs only when you press **Update**. |
| ✨ **Smooth** | The panel and the edge tab move with every screen refresh — up to 300 fps on a 300 Hz screen. Pop-ups slide in and away, the others glide into place. |
| 🎨 **Look** | Dark / light / follow Windows · panel opacity · per-app width and zoom · unread glow on the screen edge. |
| 🎥 **Streaming-safe** | Optionally hide the panel and pop-ups from screenshots, OBS and Discord screen share. |

<p align="center">
  <img src="docs/popups.png" width="430" alt="Two message pop-ups stacked in the corner of the screen">
  &nbsp;
  <img src="docs/left-dock.png" width="430" alt="The chat panel docked on the left edge">
</p>

<p align="center">
  <img src="docs/settings.png" width="880" alt="Settings: dock side, pop-up options, privacy and security">
</p>

## Install

1. Download **`ChatDock-Setup-x.y.z.exe`** from the [latest release](https://github.com/ResinCoreAI/ChatDock/releases/latest).
2. Run it. It installs for your Windows user only (no admin rights needed), adds Start-menu and
   desktop shortcuts, and starts ChatDock.
3. The welcome screen slides out (pick your language at its top right): press **Get started** and log in
   to each chat app once, with its normal login page. ChatDock remembers the logins.

> [!IMPORTANT]
> The installer isn't code-signed, so Windows SmartScreen may say *"Windows protected your PC"*.
> Click **More info → Run anyway**. The SHA-512 of every installer is listed in the release's `latest.yml`.

To uninstall: *Settings → Apps → Installed apps → ChatDock → Uninstall*. Your logins stay in
`%APPDATA%\ChatDock` unless you clear them first (Settings → Security → Clear all data).

## Using it

| To… | Do this |
|---|---|
| **Open the chat** | Hold the cursor on the screen edge (middle part) until the line reaches the top and bottom; a white tab slides out → click it. The hold time is under *Settings → Position & look* (or *Right away*) |
| Open a specific app | Click that app's icon on the white tab |
| **Reply to a pop-up** | Click the pop-up → the panel opens on that conversation |
| Open / hide from anywhere | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>C</kbd> (works in games; changeable) |
| **Hide the chat** | Click anywhere else · the hotkey again · <kbd>Esc</kbd> twice · <kbd>Ctrl</kbd>+<kbd>W</kbd> · the arrow button |
| Keep it open | Pin 📌 in the header |
| Switch apps | <kbd>Ctrl</kbd>+<kbd>1</kbd> … <kbd>Ctrl</kbd>+<kbd>9</kbd> or <kbd>Ctrl</kbd>+<kbd>Tab</kbd>; click the open app's tab again to go back to its inbox |
| Resize | Drag the panel's inner edge (remembered per app) |
| Zoom | <kbd>Ctrl</kbd>+mouse wheel or <kbd>Ctrl</kbd>+<kbd>+</kbd> / <kbd>-</kbd> / <kbd>0</kbd> (per app) |
| Reload / back | <kbd>Ctrl</kbd>+<kbd>R</kbd> or <kbd>F5</kbd> · <kbd>Alt</kbd>+<kbd>←</kbd> |
| Settings | The ⚙️ button in the header, or right-click the tray icon → Settings |

The tray menu (right-click the ChatDock icon) has the quick switches: pop-ups on/off, do not disturb
(30 min … until turned off), pin, settings, quit.

### Knowing about new messages

- **Pop-ups** in the corner you choose, above the game. They don't take focus, pause while the cursor
  is on them, stack up (the rest become "+ N more"), and vanish when you read the chat elsewhere.
- A thin **glow** on the dock edge in the app's colours while something is unread (click-through).
- Unread counts on the white tab, the header, and a red dot on the tray icon.

Pop-ups come from the notifications the chat sites raise themselves. If a site doesn't say who wrote,
ChatDock still tells you something arrived ("new message · 2 unread"). Discord needs
*User Settings → Notifications → Enable Desktop Notifications* turned on once.

## Gaming tips

- Set the game to **Borderless** or **Windowed fullscreen**. Then the panel and pop-ups float over it.
- Games that take over the mouse (FPS aiming, camera turning) never pull the white tab out: while a
  game hides the pointer or holds it inside its window, ChatDock ignores the screen edge.
- **Exclusive fullscreen** games can't be drawn over by anything. Opening the chat minimizes the game
  for a moment (a yellow bar explains this), and hiding the chat brings it back.
- Games where the mouse lives on the screen edge (MOBA / RTS camera scrolling): set
  *Settings → Position → Cursor on the screen edge* to “not during fullscreen games” (or raise the hold time) and use
  the hotkey instead. Or move the dock to the other edge.
- ChatDock never touches the game's files or memory. It is just a window that stays on top.

## Privacy & security

ChatDock shows the chat services' **real websites**, and you log in to them directly. It has no
server, sends nothing anywhere, and never sees your passwords.

| | |
|---|---|
| **Encrypted cookies** | Logins and cookies are stored encrypted with your Windows account's key (DPAPI, via Electron's cookie-encryption fuse). Copied to another PC or user, they are useless. Cookies saved by the old portable 1.0 build are re-encrypted once. |
| **Isolated logins** | Every app has its own storage partition; one site can't read another's cookies. |
| **Sandboxed pages** | All pages run in Chromium's sandbox with context isolation and no Node.js. The chat sites get nothing from ChatDock except the notification hand-off and the passkey guard below. |
| **No local access** | Chat pages can't reach programs on your PC (`localhost` is blocked) and can't open ChatDock's own pages. |
| **Private IP stays private** | WebRTC is limited to the default route and never lists your local addresses. |
| **Only what's needed** | Permissions (notifications, mic/camera for calls, …) are granted only to the app's own domains. Links that leave the app open in your normal browser. |
| **Locked app files** | Release builds enable Electron fuses: the `app.asar` integrity is checked at start, and `RunAsNode`, `NODE_OPTIONS` and `--inspect` are disabled. A tampered ChatDock won't start. |
| **No surprise passkey dialogs** | Login pages that ask for passkeys on their own (Meta, Discord) would pop Windows' passkey dialog over your game. ChatDock turns passkeys off, so use a password or QR code to log in. |
| **Safe updates** | Updates come over HTTPS from this repository's releases. Each download is checked against the SHA-512 in `latest.yml` before it runs, and it installs only when you press the button. |
| **Minimal log** | `chatdock.log` records events (opened, closed, pop-up from *which app*). It never records names or message text. |

Settings → Security also lets you hide ChatDock from screen capture and log out of every app
(wipes cookies, storage and cache). Found a security problem? Please open an issue.

## Updates

ChatDock checks this repository's [releases](https://github.com/ResinCoreAI/ChatDock/releases)
shortly after it starts and every 6 hours. It downloads a new version in the background (usually just
the changed parts, a few MB), then shows a pop-up and an **Update** button in the panel header.
Nothing is installed until you press it. Then an *Updating ChatDock* window shows the progress, the
installer's progress window follows, and ChatDock starts again by itself with a “now on Beta Build 1.4 ✓”
pop-up and a *What's new* list. Both automatic checking and background downloading can be turned off
under Settings → Updates.

Releases are called **Beta Build 1.1, 1.2, 1.3, …**: the version number without its trailing `.0`
(Beta Build 1.4 is version `1.4.0`).

## Troubleshooting

| Problem | Fix |
|---|---|
| The white tab doesn't appear | Check *Cursor on the screen edge* isn't off and the right monitor/edge is chosen. The top and bottom corners are left alone on purpose (close buttons, clock), so aim for the middle, and keep the cursor there until the line reaches the top and bottom (or set *Hold time at the edge* to *Right away*). Holding a mouse button down also suppresses it. |
| The hotkey does nothing | Another program took it. Pick another one in Settings → General. |
| No pop-ups | Check pop-ups are on and do-not-disturb is off (tray menu), then *Test pop-up*. Check that app’s own switches under Settings → Notifications → Per app. Chats you're currently looking at don't pop up. For Discord, enable its desktop notifications. |
| A pop-up only says "new message" | That site didn't include the details in its notification; ChatDock only knows the count. |
| "Continue with Google" on X fails | Google blocks sign-in inside embedded browsers. Use X's own username + password. |
| Windows Firewall asks about ChatDock | Not needed for chatting. *Cancel* is fine. |
| Anything else | Send `%APPDATA%\ChatDock\chatdock.log`: it has no chat content. |

## Build from source

Requires Windows 10/11, [Node.js](https://nodejs.org/) 22+ and Git.

```bash
git clone https://github.com/ResinCoreAI/ChatDock.git
cd ChatDock
npm install          # if npm blocks install scripts: node node_modules/electron/install.js
npm start            # run from source (updates are disabled in this mode)
npm run dist         # build the installer: dist/ChatDock-Setup-<version>.exe (+ .blockmap, latest.yml)
```

Publishing a release (the installed apps pick it up automatically):

```bash
# Beta Build 1.N = version 1.N.0: raise the middle number of "version" in package.json
# (and add "whatsnew.1.N" to src/ui/i18n.js for the in-app What's new), then
npm run dist
gh release create v1.5.0 dist/ChatDock-Setup-1.5.0.exe dist/ChatDock-Setup-1.5.0.exe.blockmap dist/latest.yml --title "ChatDock Beta Build 1.5" --notes "What changed"
```

Developer switches: `--profile=<dir>` (separate data folder) · `--debug` · `--selftest [--shots=<dir>]`
(automatic end-to-end check; add `--selftest-only=edge` for just the frame clock, edge tab, hold line
and pop-up checks, which never take focus from a game) · `--no-occlusion` (test with the screen locked) ·
`--stress[=tab|keys|popup] [--side=left]` (real mouse/keyboard; needs a window titled `FAKE GAME`
covering the screen) · `--demo-frames=<dir>` (records the README animation; see
`scripts/make_demo_gif.py`). The stress and demo modules are not shipped in the installer.

<details>
<summary>Project layout</summary>

```
src/main.js          windows, edge detection, animations, hotkey, tray, settings, unread counts
src/updater.js       GitHub release updates (electron-updater)
src/toasts.js        message pop-ups (topmost, never take focus)
src/frames.js        frame clock: moves windows once per screen refresh (vertical blank) while animating
src/apps.js          the chat services (URLs, domains, colours, widths): add new ones here
src/settings.js      settings store (%APPDATA%\ChatDock\settings.json)
src/win32.js         Windows calls Electron lacks (focus hand-back, fullscreen detection) via koffi
src/preload.js       bridge for ChatDock's own pages
src/preload-site.js  passkey guard + notification hand-off for the chat sites
src/ui/              panel + settings, edge tab, glow, hold line, pop-ups (served from chatdock://ui/)
src/ui/i18n.js       every UI text in English, Thai, Chinese, Japanese and German
src/stress.js        real-input stress test (dev only)
src/demo.js          README animation recorder (dev only)
build/installer.nsh  uninstall step: remove "start with Windows"
scripts/             icon and README-media generators
```

</details>

## License

[MIT](LICENSE) © 2026 ResinCore

ChatDock is a ResinCore project and isn't affiliated with Meta, X, Discord, Telegram or WhatsApp.
Their names and logos belong to their teams.
