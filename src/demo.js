'use strict';

// Dev-only: records the README animation. Every frame captures ChatDock's own windows one by one
// (panel, chat page, edge tab, pop-ups) with their screen positions, plus a scripted cursor.
// scripts/make_demo_gif.py then paints them over a game scene. No real mouse input is used, so it
// also works with the screen locked.
//   electron . --demo-frames=<dir> --profile=<fresh scratch profile> --no-occlusion
// <dir>/avatar-1.png and avatar-2.png (optional) are used as the senders' pictures.

const fs = require('node:fs');
const path = require('node:path');

const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);

async function run(ctx) {
  const { dir, log } = ctx;
  fs.mkdirSync(dir, { recursive: true });
  const wa = ctx.display().workArea;
  const avatar = (n) => {
    const f = path.join(dir, `avatar-${n}.png`);
    return fs.existsSync(f) ? `data:image/png;base64,${fs.readFileSync(f).toString('base64')}` : '';
  };

  // ---- scripted cursor: straight eased moves; the recorder asks where it is at any moment
  let from = { x: wa.x + wa.width * 0.5, y: wa.y + wa.height * 0.55 };
  let to = from;
  let t0 = 0;
  let span = 1;
  const clicks = [];
  const cursorAt = (t) => {
    const p = span > 0 ? Math.min(1, Math.max(0, (t - t0) / span)) : 1;
    const k = ease(p);
    return { x: Math.round(from.x + (to.x - from.x) * k), y: Math.round(from.y + (to.y - from.y) * k) };
  };
  const move = async (x, y, ms) => {
    from = cursorAt(Date.now());
    to = { x: Math.round(x), y: Math.round(y) };
    t0 = Date.now();
    span = ms;
    await wait(ms);
  };
  const press = () => clicks.push(Date.now());
  const captions = []; // English captions for the README animation
  const caption = (text) => captions.push({ t: Date.now(), text });

  // ---- recorder
  const frames = [];
  const lastPng = {};
  let files = 0;
  let recording = true;
  const capture = async () => {
    const t = Date.now();
    const layers = await ctx.layers();
    const entry = { t, cursor: cursorAt(t), layers: [] };
    for (const L of layers) {
      const png = L.img.toPNG();
      let file = lastPng[L.name] && lastPng[L.name].png.equals(png) ? lastPng[L.name].file : null;
      if (!file) { // only write a layer again when it changed
        file = `${L.name}-${String(files += 1).padStart(4, '0')}.png`;
        await fs.promises.writeFile(path.join(dir, file), png);
        lastPng[L.name] = { png, file };
      }
      entry.layers.push({ name: L.name, file, x: L.x, y: L.y, w: L.w, h: L.h, opacity: L.opacity });
    }
    frames.push(entry);
  };
  const loop = (async () => {
    while (recording) {
      await capture();
      await wait(10);
    }
  })();

  const clickOn = async (target, selector, ms = 750) => {
    const p = await ctx.point(target, selector);
    if (!p) throw new Error(`demo: nothing at ${target} ${selector}`);
    await move(p.screen.x, p.screen.y, ms);
    await wait(120);
    press();
    ctx.click(target, p.local);
  };

  // ---- the story: two messages pop up over the game, open one, look around, move the dock left
  const started = Date.now();
  caption('Playing a game… and someone messages you');
  await wait(1300);
  await ctx.notify('discord', 'สมชาย ใจดี', 'ว่างไหม เย็นนี้ลงแรงค์ด้วยกัน 5 ตาพอ 🎮', avatar(2));
  await wait(300);
  caption('ChatDock pops up who wrote and what, right over the game');
  await wait(700);
  await ctx.notify('instagram', 'mint.ch', 'ส่งรูปให้ดูแล้วนะ 555 📸', avatar(1));
  await wait(1300);
  caption('Click a pop-up and that chat slides in');
  await clickOn('toast', '.card', 900);
  await wait(1800);

  caption('Settings: apps, pop-ups, privacy, updates');
  await clickOn('panel', '#settings-btn', 800);
  await wait(1000);
  await clickOn('panel', '.set-nav [data-goto="look"]', 600);
  await wait(700);
  caption('Dock it on the left or the right edge');
  await clickOn('panel', '.mini-screen.left', 700);
  await wait(1600);

  caption('Click the game: the chat tucks away, the game gets focus back');
  await move(wa.x + wa.width * 0.62, wa.y + wa.height * 0.5, 800);
  await wait(100);
  press();
  ctx.clickElsewhere();
  await wait(900);

  caption('Rest the cursor on the edge for the white tab');
  await move(wa.x, wa.y + wa.height * 0.46, 900);
  ctx.showTab(wa.y + wa.height * 0.46);
  await wait(1700);

  recording = false;
  await loop;
  ctx.setPref('side', 'right');
  const screen = { x: ctx.display().bounds.x, y: ctx.display().bounds.y, width: ctx.display().bounds.width, height: ctx.display().bounds.height };
  fs.writeFileSync(path.join(dir, 'frames.json'), JSON.stringify({ screen, workArea: wa, clicks, captions, frames }, null, 1));
  log('demo recorded', { frames: frames.length, files, seconds: (Date.now() - started) / 1000, captureErrors: ctx.captureErrors() });
}

module.exports = { run };
