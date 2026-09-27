'use strict';

// Dev-only: drives ChatDock with REAL mouse/keyboard input to reproduce open/close bugs.
//   ChatDock.exe --stress --profile=<scratch dir>
// Needs a window titled "FAKE GAME" covering the screen (clicks "elsewhere" land on it), and the
// user must leave the mouse alone while it runs. Every click target is verified before clicking.

const koffi = require('koffi');
const { screen } = require('electron');

const user32 = koffi.load('user32.dll');
const SetCursorPos = user32.func('int __stdcall SetCursorPos(int x, int y)');
const mouseEvent = user32.func('void __stdcall mouse_event(uint32_t flags, uint32_t dx, uint32_t dy, uint32_t data, uintptr_t extra)');
const keybdEvent = user32.func('void __stdcall keybd_event(uint8_t vk, uint8_t scan, uint32_t flags, uintptr_t extra)');
const POINT = koffi.struct('STRESS_POINT', { x: 'int32_t', y: 'int32_t' });
const WindowFromPoint = user32.func('intptr_t __stdcall WindowFromPoint(STRESS_POINT pt)');
const GetAncestor = user32.func('intptr_t __stdcall GetAncestor(intptr_t hWnd, uint32_t flags)');
const GetWindowTextW = user32.func('int __stdcall GetWindowTextW(intptr_t hWnd, void *buf, int max)');
const MapVirtualKeyW = user32.func('uint32_t __stdcall MapVirtualKeyW(uint32_t code, uint32_t mapType)');
const GetClassNameW = user32.func('int __stdcall GetClassNameW(intptr_t hWnd, void *buf, int max)');
const GetWindowThreadProcessId = user32.func('uint32_t __stdcall GetWindowThreadProcessId(intptr_t hWnd, _Out_ uint32_t *pid)');

function whoIsAt(x, y) {
  const p = px(x, y);
  const h = WindowFromPoint({ x: p.x, y: p.y });
  const root = GetAncestor(h, 2);
  const cls = Buffer.alloc(512);
  const n = GetClassNameW(h, cls, 256);
  const pid = [0];
  GetWindowThreadProcessId(h, pid);
  return { hwnd: h, root, cls: n > 0 ? cls.toString('utf16le', 0, n * 2) : '', pid: pid[0], title: windowTitleAt(x, y).title };
}

const LEFTDOWN = 0x02;
const LEFTUP = 0x04;
const KEYUP = 0x02;
const VK = { CONTROL: 0x11, MENU: 0x12, C: 0x43, ESCAPE: 0x1b };
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
let realScanCodes = true; // real keyboards send scan codes; some remote/virtual keyboards don't
const key = (vk, up) => keybdEvent(vk, realScanCodes ? MapVirtualKeyW(vk, 0) : 0, up ? KEYUP : 0, 0);

function px(x, y) {
  const p = screen.dipToScreenPoint({ x, y });
  return { x: Math.round(p.x), y: Math.round(p.y) };
}

function windowTitleAt(x, y) {
  const p = px(x, y);
  const h = GetAncestor(WindowFromPoint({ x: p.x, y: p.y }), 2 /* GA_ROOT */);
  const buf = Buffer.alloc(512);
  const n = GetWindowTextW(h, buf, 256);
  return { hwnd: h, title: n > 0 ? buf.toString('utf16le', 0, n * 2) : '' };
}

function moveTo(x, y) {
  const p = px(x, y);
  SetCursorPos(p.x, p.y);
}

async function click(x, y) {
  moveTo(x, y);
  await wait(40);
  mouseEvent(LEFTDOWN, 0, 0, 0, 0);
  await wait(70);
  mouseEvent(LEFTUP, 0, 0, 0, 0);
}

async function hotkey() {
  key(VK.CONTROL, false);
  key(VK.MENU, false);
  key(VK.C, false);
  await wait(30);
  key(VK.C, true);
  key(VK.MENU, true);
  key(VK.CONTROL, true);
}

async function escEsc() {
  for (let i = 0; i < 2; i += 1) {
    key(VK.ESCAPE, false);
    await wait(20);
    key(VK.ESCAPE, true);
    await wait(130);
  }
}

async function run(ctx) {
  const { log } = ctx;
  const results = [];
  const d = ctx.display();
  const left = ctx.side() === 'left';
  const right = d.bounds.x + d.bounds.width;
  const edgeX = left ? d.bounds.x : right - 1; // the screen edge the dock lives on
  const midY = Math.round(d.bounds.y + d.bounds.height / 2);
  const GAME = { x: Math.round(d.bounds.x + d.bounds.width / 2), y: midY }; // clear of the panel on either side

  const expect = async (label, want, settleMs = 750) => {
    await wait(settleMs);
    const s = ctx.state();
    const g = ctx.geometry();
    const okOpen = s.st === 'open' && s.vis && s.x === g.x;
    const okHidden = s.st === 'hidden' && !s.vis;
    const ok = want === 'open' ? okOpen : want === 'hidden' ? okHidden : (okOpen || okHidden);
    results.push({ label, want, ok });
    log(`STRESS ${ok ? 'PASS' : 'FAIL'} ${label} (want ${want})`, s);
    return ok;
  };

  const gameIsUnderCursorTarget = () => windowTitleAt(GAME.x, GAME.y).title === 'FAKE GAME';

  const clickGame = async () => {
    if (!gameIsUnderCursorTarget()) {
      log('STRESS skip game click: window at target is', windowTitleAt(GAME.x, GAME.y).title);
      return false;
    }
    await click(GAME.x, GAME.y);
    return true;
  };

  const openViaTab = async (app = 'instagram') => {
    moveTo(edgeX, midY);
    let shown = false;
    for (let i = 0; i < 25 && !shown; i += 1) {
      await wait(40);
      shown = ctx.state().tab;
    }
    if (!shown) {
      log('STRESS tab did not appear', ctx.state());
      return false;
    }
    await wait(260); // let it slide in (clicks in the first 180 ms are ignored on purpose)
    const target = await ctx.tabIconPoint(app);
    if (!target) {
      log('STRESS no icon on the tab for', app);
      return false;
    }
    const who = whoIsAt(target.x, target.y);
    log('STRESS window under tab click point:', { ...who, isTab: who.root === ctx.hwnds()[1], isPanel: who.root === ctx.hwnds()[0], ourPid: process.pid });
    await click(target.x, target.y);
    return true;
  };

  const clickHideButton = async () => {
    const b = ctx.state();
    if (b.st !== 'open') return false;
    const p = await ctx.panelPoint('#hide');
    if (!p) return false;
    await click(p.x, p.y);
    return true;
  };

  log('STRESS start', ctx.mode, { side: left ? 'left' : 'right', edgeX, midY, game: windowTitleAt(GAME.x, GAME.y).title });
  if (!gameIsUnderCursorTarget()) {
    log('STRESS abort: no FAKE GAME window under', GAME);
    return results;
  }
  await clickGame(); // give the fake game the foreground, like a real game
  await wait(500);

  if (ctx.mode === 'demo') { // real screenshots of the new features over the game backdrop
    moveTo(GAME.x, GAME.y);
    await wait(500);
    const apps = ctx.enabledApps();
    ctx.setCount(apps[0], 2);
    ctx.setCount(apps[apps.length - 1], 5);
    await ctx.notify(apps[apps.length - 1], 'สมชาย ใจดี', 'ว่างไหม เย็นนี้ลงแรงค์ด้วยกัน 5 ตาพอ 🎮');
    await wait(700);
    await ctx.notify(apps[0], 'mint.ch', 'ส่งรูปให้ดูแล้วนะ 555');
    await wait(1200);
    await ctx.capture('demo-1-popups');
    moveTo(edgeX, midY);
    for (let i = 0; i < 25 && !ctx.state().tab; i += 1) await wait(40);
    await wait(400);
    const icon = await ctx.tabIconPoint(apps[1]);
    if (icon) moveTo(icon.x, icon.y); // hover a middle icon
    await wait(300);
    await ctx.capture('demo-2-tab');
    const p = await ctx.toastCardPoint('card');
    if (p) await click(p.x, p.y); // open the newest message's chat
    await wait(1400);
    await ctx.capture('demo-3-open');
    await hotkey();
    await wait(800);
    moveTo(GAME.x, GAME.y);
    log('STRESS done: demo captured');
    return results;
  }

  if (ctx.mode === 'popup') { // message pop-ups over a "game" with real clicks
    const apps = ctx.enabledApps();
    const pick = (i) => apps[i % apps.length];
    const popupShown = async () => {
      for (let i = 0; i < 20 && !ctx.toast().visible; i += 1) await wait(50);
      return ctx.toast().visible;
    };
    const check = (label, ok, extra) => {
      results.push({ label, ok });
      log(`STRESS ${ok ? 'PASS' : 'FAIL'} ${label}`, extra || ctx.state());
    };

    for (let i = 0; i < 4; i += 1) {
      const app = pick(i);
      const clicksBefore = await ctx.pageClicks(app);
      await ctx.notify(app, `คนทดสอบ ${i + 1}`, 'ข้อความทดสอบจาก ChatDock');
      const shown = await popupShown();
      await wait(400);
      const s = ctx.state();
      check(`pop-up #${i + 1} (${app}) appears without taking focus from the game`, shown && s.fg !== 'PANEL' && /WindowsForms/.test(s.fg), { ...s, toast: ctx.toast() });
      const p = await ctx.toastCardPoint('card');
      if (!p) continue;
      await click(p.x, p.y);
      await wait(900);
      const after = ctx.state();
      const pageRan = (await ctx.pageClicks(app)) > clicksBefore;
      check(`clicking pop-up #${i + 1} opens ${app} and its conversation`, after.st === 'open' && pageRan && !ctx.toast().visible, { ...after, pageRan });
      await hotkey();
      await wait(800);
      check(`closing after pop-up #${i + 1} returns focus to the game`, ctx.state().st === 'hidden' && /WindowsForms/.test(ctx.state().fg));
    }

    // Dismiss with the X: the pop-up goes away and the game keeps / gets back focus.
    await ctx.notify(pick(0), 'คนทดสอบ ปิด', 'กดกากบาทเพื่อปิด');
    await popupShown();
    await wait(500);
    const x = await ctx.toastCardPoint('close');
    if (x) {
      moveTo(x.x - 40, x.y); // hover first, like a person
      await wait(250);
      await click(x.x, x.y);
      await wait(700);
      check('X closes the pop-up and focus stays in the game', !ctx.toast().visible && ctx.state().st === 'hidden' && /WindowsForms/.test(ctx.state().fg));
    }

    // Burst: several messages at once stack up (max 3 + "more") and never steal focus.
    for (let i = 0; i < 5; i += 1) await ctx.notify(pick(i), `คนที่ ${i + 1}`, `ข้อความที่ ${i + 1}`);
    await wait(900);
    check('a burst of 5 messages stacks without taking focus', ctx.toast().visible && ctx.toast().count === 5 && /WindowsForms/.test(ctx.state().fg), { toast: ctx.toast(), fg: ctx.state().fg });
    moveTo(GAME.x, GAME.y);
    await wait(9000); // they fade out on their own (8 s)
    check('pop-ups go away on their own', !ctx.toast().visible, { toast: ctx.toast() });

    const failed = results.filter((r) => !r.ok);
    log(`STRESS done: ${results.length - failed.length}/${results.length} passed`, failed.map((f) => f.label));
    return results;
  }

  if (ctx.mode === 'keys' || ctx.mode === 'keys-noscan') { // Esc Esc right after opening, via each open path
    realScanCodes = ctx.mode === 'keys';
    for (const via of ['hotkey', 'tab', 'hotkey', 'tab']) {
      if (via === 'hotkey') await hotkey(); else await openViaTab('instagram');
      await wait(800);
      log('STRESS keys: opened?', ctx.state());
      await escEsc();
      await expect(`Esc Esc after opening via ${via}`, 'hidden');
      if (ctx.state().st === 'open') { await hotkey(); await wait(700); }
      moveTo(GAME.x, GAME.y);
      await wait(1100);
    }
    const failed = results.filter((r) => !r.ok);
    log(`STRESS done: ${results.length - failed.length}/${results.length} passed`, failed.map((f) => f.label));
    return results;
  }

  if (ctx.mode === 'tab') { // just the white-tab path, over and over
    for (let i = 0; i < 12; i += 1) {
      moveTo(GAME.x, GAME.y);
      await wait(1100); // let the tab tuck away
      const app = i % 2 ? 'facebook' : 'instagram';
      if (await openViaTab(app)) await expect(`open via tab #${i + 1} (${app})`, 'open');
      if (ctx.state().st === 'open') { await hotkey(); await wait(700); }
    }
    const failed = results.filter((r) => !r.ok);
    log(`STRESS done: ${results.length - failed.length}/${results.length} passed`, failed.map((f) => f.label));
    moveTo(GAME.x, GAME.y);
    return results;
  }

  // 1) each open / close path on its own
  if (await openViaTab('instagram')) await expect('open via tab (IG icon)', 'open');
  if (await clickGame()) await expect('close by clicking the game', 'hidden');
  await hotkey(); await expect('open via hotkey', 'open');
  await hotkey(); await expect('close via hotkey', 'hidden');
  if (await openViaTab('facebook')) await expect('open via tab (FB icon)', 'open');
  await escEsc(); await expect('close via Esc Esc', 'hidden');
  await hotkey(); await expect('open via hotkey (2)', 'open');
  if (await clickHideButton()) await expect('close via hide button', 'hidden');

  // 2) the "click back and forth quickly" cases
  for (let i = 0; i < 6; i += 1) {
    await hotkey(); await wait(90 + i * 40); await hotkey();
    await expect(`hotkey twice fast #${i + 1}`, 'hidden', 900);
  }
  for (let i = 0; i < 4; i += 1) {
    await hotkey(); await wait(60 + i * 50);
    await clickGame();
    await expect(`open then click the game during the slide #${i + 1}`, 'hidden', 900);
    if (ctx.state().st === 'open') { await hotkey(); await wait(700); }
  }
  for (let i = 0; i < 4; i += 1) {
    if (await openViaTab(i % 2 ? 'facebook' : 'instagram')) {
      await wait(40 + i * 60);
      await clickGame();
      await expect(`tab open then click the game fast #${i + 1}`, 'hidden', 900);
      if (ctx.state().st === 'open') { await hotkey(); await wait(700); }
    }
  }
  for (let i = 0; i < 10; i += 1) {
    const r = Math.random();
    if (r < 0.35) await hotkey();
    else if (r < 0.6) await clickGame();
    else if (r < 0.8 && ctx.state().st === 'hidden') await openViaTab(Math.random() < 0.5 ? 'instagram' : 'facebook');
    else if (ctx.state().st === 'open') await escEsc();
    await wait(40 + Math.floor(Math.random() * 400));
  }
  await expect('after random clicking, state is consistent', 'any', 1000);

  // A click that lands the instant the tab pops up is ignored on purpose; the game must keep focus.
  if (ctx.state().st === 'open') { await hotkey(); await wait(800); }
  moveTo(GAME.x, GAME.y);
  await wait(1100);
  moveTo(edgeX, midY);
  for (let i = 0; i < 25 && !ctx.state().tab; i += 1) await wait(20);
  if (ctx.state().tab) {
    const p = await ctx.tabIconPoint(ctx.enabledApps()[0]);
    await click(p.x, p.y); // ~20-60 ms after it appeared
    await wait(700);
    const s = ctx.state();
    const ok = s.st === 'hidden' && s.fg !== 'Chrome_WidgetWin_1' && s.fg !== 'PANEL';
    results.push({ label: 'too-early tab click leaves focus in the game', want: 'hidden', ok });
    log(`STRESS ${ok ? 'PASS' : 'FAIL'} too-early tab click leaves focus in the game`, s);
  }

  // 3) does it still open afterwards? (the reported bug)
  if (ctx.state().st === 'open') { await hotkey(); await wait(800); }
  if (await openViaTab('instagram')) await expect('still opens via tab afterwards', 'open');
  if (await clickGame()) await expect('still closes by clicking the game', 'hidden');
  await hotkey(); await expect('still opens via hotkey afterwards', 'open');
  await hotkey(); await expect('still closes via hotkey afterwards', 'hidden');

  const failed = results.filter((r) => !r.ok);
  log(`STRESS done: ${results.length - failed.length}/${results.length} passed`, failed.map((f) => f.label));
  moveTo(GAME.x, GAME.y);
  return results;
}

module.exports = { run };
