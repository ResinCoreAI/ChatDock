# ChatDock official website

Agreed with the founder on 2026-11-14 through 15 clickable mock-ups (docs/site-mocks/). The founder picked mock 11 ("clay peach") and asked to build it for real with smoother animation.

## What it is

- A download site for ChatDock, hosted on GitHub Pages (resincoreai.github.io/ChatDock first; a custom domain can come later).
- Two pages: the home page and "What's new" (every release, read from GitHub Releases).
- Five languages, the same as the app: Thai, English, Chinese, Japanese, German. First visit follows the browser's language; the choice is remembered.
- No 3D. The founder moved from the react-three-fiber idea to the flat clay look.

## Home page

- Hero: version pill, headline, one paragraph, download button, a line with "free · MIT · size · Windows 10/11".
- A clay monitor holding a live ChatDock demo: a game picture, the white edge tab, the chat panel (switch apps, open a chat, send a quick reply), and Spotify as a music player (not a chat).
- No game HUD on the demo screen (the founder removed the minimap, HP bar, ammo count and crosshair).
- Clay bits around the hero that can be dragged and spring back; a sticker beside the monitor that reacts to what the visitor does.
- Six features with their own clay icons; the apps behind the tab (tapping one opens it on the demo screen); highlights of the latest release; four questions; a closing download block.

## Data

- The download button points straight at the latest `*-setup.exe` from the GitHub API; if that call fails it points at the releases page.
- Version and installer size come from the same call.
- Release notes are written in English on GitHub; the site shows them as they are.

## Feel

- Animation moves only transform and opacity, with spring easing; every section is readable at rest.
- `prefers-reduced-motion` stops all of it.
- The demo canvas draws at the screen's real resolution, and stops drawing when it is off screen or the tab is hidden.
- Phones: the white tab stays out and a tap near the right edge opens the panel.

## Code

- `site/`: static HTML, CSS and ES modules, no build step and no dependencies.
- Pure functions (language choice, release parsing, installer pick, sizes) live in `site/assets/lib.js` with `node --test` tests.
- Deploy: the `site/` folder goes to the `gh-pages` branch; this happens only when the founder says so.
