// The live demo: a game picture with ChatDock's white edge tab and chat panel on top, drawn into a canvas.
// It draws at the screen's real resolution, moves the tab and panel on springs, and sleeps off screen.
import { ICONS } from './icons.js';
import { SCREEN } from './i18n.js';
import { mmss } from './lib.js';

const W = 1280;
const H = 800;
const PW = 430;
const APPS = ['discord', 'facebook', 'instagram', 'telegram', 'whatsapp', 'x', 'spotify'];
// made-up tracks for the Spotify view (Spotify plays music; it is not a chat)
const TRACKS = [
  { t: 'Night Drive', a: 'Lofi Lab', d: 184, c: ['#1db954', '#0b3d2e'] },
  { t: 'Rain on Glass', a: 'Hikari', d: 201, c: ['#8ecae6', '#3a4f8f'] },
  { t: 'Boss Fight (Chill)', a: '8-bit Café', d: 167, c: ['#ffb36b', '#b23a48'] },
];

const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

// a damped spring: smooth start, a hint of overshoot, settles by itself
function spring(k = 230, c = 24) {
  return {
    x: 0, v: 0, to: 0,
    step(dt) {
      const a = k * (this.to - this.x) - c * this.v;
      this.v += a * dt;
      this.x += this.v * dt;
      if (Math.abs(this.to - this.x) < 0.0005 && Math.abs(this.v) < 0.0005) { this.x = this.to; this.v = 0; }
      return this.x;
    },
  };
}

const iconCache = {};
function iconImage(id, color) {
  const key = id + color;
  if (!iconCache[key]) {
    const svg = ICONS[id].replace('<svg ', '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" ').replace(/currentColor/g, color);
    const img = new Image();
    img.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
    iconCache[key] = img;
  }
  return iconCache[key];
}

function rr(ctx, x, y, w, h, r) {
  const q = typeof r === 'number' ? [r, r, r, r] : r;
  ctx.beginPath();
  ctx.moveTo(x + q[0], y);
  ctx.lineTo(x + w - q[1], y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + q[1]);
  ctx.lineTo(x + w, y + h - q[2]);
  ctx.quadraticCurveTo(x + w, y + h, x + w - q[2], y + h);
  ctx.lineTo(x + q[3], y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - q[3]);
  ctx.lineTo(x, y + q[0]);
  ctx.quadraticCurveTo(x, y, x + q[0], y);
  ctx.closePath();
}

export function mountScreen(host, { palette = {}, font = 'system-ui, sans-serif', still = false } = {}) {
  const P = Object.assign({
    sky: ['#9fd8ff', '#ffe3c9'], sun: '#ffd166', hills: ['#7ed3b2', '#5fbf98', '#3fa47c'],
    panel: '#fffaf3', text: '#3a2a22', muted: '#9b8579', line: '#f0e2d3', accent: '#ff6b4a',
    bubble: '#f3e6d8', mine: '#ff6b4a', mineText: '#ffffff', tab: '#ffffff', badge: '#ff4a3d',
  }, palette);
  const touch = matchMedia('(hover: none)').matches;
  const canvas = document.createElement('canvas');
  canvas.setAttribute('role', 'img');
  canvas.style.cssText = 'display:block;width:100%;height:auto;aspect-ratio:16/10;touch-action:pan-y;';
  host.append(canvas);
  const ctx = canvas.getContext('2d');

  const TAB = touch ? { w: 52, h: 170, y: H * 0.3 } : { w: 26, h: 104, y: H * 0.36 };
  const s = {
    lang: 'en', t: 0, mouse: { x: 0, y: 0, on: false }, cursor: { x: W * 0.5, y: H * 0.6 },
    open: false, app: 'discord', chat: -1, sent: [], seen: {}, linger: 0, hold: 0,
    playing: true, track: 0, pos: 42, demo: null, idleUntil: 0, hits: [],
  };
  const tab = spring(260, 26);
  const panel = spring(210, 23);
  const L = () => SCREEN[s.lang] || SCREEN.en;
  const unread = (app) => (s.seen[app] || app === 'spotify' ? 0 : L().chats[app].reduce((a, c) => a + c.u, 0));
  const total = () => APPS.reduce((a, id) => a + unread(id), 0);
  const listeners = new Set();
  const changed = () => listeners.forEach((f) => f(s));

  // ---- size: draw at the real pixel size of the canvas on screen ----
  let scale = 1;
  function fit() {
    const w = canvas.clientWidth || host.clientWidth;
    if (!w) return;
    const k = Math.min(window.devicePixelRatio || 1, 2);
    scale = (w * k) / W;
    canvas.width = Math.round(W * scale);
    canvas.height = Math.round(H * scale);
  }
  new ResizeObserver(fit).observe(host);
  fit();

  // ---- drawing ----
  function hit(tag, x, y, w, h, fn) { s.hits.push({ tag, x, y, w, h, fn }); }

  function game() {
    const g = ctx.createLinearGradient(0, 0, 0, H);
    g.addColorStop(0, P.sky[0]);
    g.addColorStop(1, P.sky[1]);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, W, H);
    ctx.fillStyle = P.sun;
    ctx.beginPath();
    ctx.arc(W * 0.66, H * 0.42 + Math.sin(s.t * 0.4) * 4, 74, 0, Math.PI * 2);
    ctx.fill();
    P.hills.forEach((c, i) => {
      const base = H * (0.55 + i * 0.12);
      const speed = 14 + i * 22;
      ctx.fillStyle = c;
      ctx.beginPath();
      ctx.moveTo(0, H);
      for (let x = 0; x <= W; x += 16) {
        const k = x + s.t * speed;
        ctx.lineTo(x, base - Math.sin(k / (170 - i * 30)) * (42 - i * 8) - Math.sin(k / 61 + i) * 12);
      }
      ctx.lineTo(W, H);
      ctx.fill();
    });
  }

  function avatar(x, y, r, name) {
    let h = 0;
    for (const ch of name) h = (h * 31 + ch.codePointAt(0)) % 360;
    ctx.fillStyle = `hsl(${h} 55% 52%)`;
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = '#fff';
    ctx.font = `700 ${Math.round(r * 0.9)}px ${font}`;
    ctx.textAlign = 'center';
    ctx.fillText([...name.replace(/^[@(]/, '')][0].toUpperCase(), x, y + r * 0.32);
    ctx.textAlign = 'left';
  }

  function clip(text, max) {
    if (ctx.measureText(text).width <= max) return text;
    let t = text;
    while (t.length > 1 && ctx.measureText(t + '…').width > max) t = t.slice(0, -1);
    return t + '…';
  }

  function player(x0) {
    const tr = TRACKS[s.track];
    const cx = x0 + PW / 2;
    ctx.textAlign = 'center';
    ctx.fillStyle = P.muted;
    ctx.font = `500 17px ${font}`;
    ctx.fillText(clip(L().music.now, PW - 40), cx, 116);
    const size = 230;
    const ax = cx - size / 2;
    const ay = 136;
    const g = ctx.createLinearGradient(ax, ay, ax + size, ay + size);
    g.addColorStop(0, tr.c[0]);
    g.addColorStop(1, tr.c[1]);
    ctx.fillStyle = g;
    rr(ctx, ax, ay, size, size, 22);
    ctx.fill();
    ctx.save();
    ctx.translate(cx, ay + size / 2);
    ctx.rotate(s.t * 0.8 * (s.playing ? 1 : 0) + s.track);
    ctx.strokeStyle = 'rgba(255,255,255,.3)';
    ctx.lineWidth = 2;
    [86, 66, 46].forEach((r) => { ctx.beginPath(); ctx.arc(0, 0, r, 0.3, Math.PI * 1.7); ctx.stroke(); });
    ctx.fillStyle = 'rgba(255,255,255,.85)';
    ctx.beginPath(); ctx.arc(0, 0, 14, 0, Math.PI * 2); ctx.fill();
    ctx.restore();
    ctx.fillStyle = P.text;
    ctx.font = `700 24px ${font}`;
    ctx.fillText(tr.t, cx, ay + size + 44);
    ctx.fillStyle = P.muted;
    ctx.font = `400 18px ${font}`;
    ctx.fillText(tr.a, cx, ay + size + 72);
    const by = ay + size + 98;
    const bx = x0 + 40;
    const bw = PW - 80;
    ctx.fillStyle = P.line;
    rr(ctx, bx, by, bw, 8, 4);
    ctx.fill();
    ctx.fillStyle = '#1db954';
    rr(ctx, bx, by, Math.max(8, bw * (s.pos / tr.d)), 8, 4);
    ctx.fill();
    ctx.fillStyle = P.muted;
    ctx.font = `400 15px ${font}`;
    ctx.textAlign = 'left';
    ctx.fillText(mmss(s.pos), bx, by + 30);
    ctx.textAlign = 'right';
    ctx.fillText(mmss(tr.d), bx + bw, by + 30);
    hit('seek', bx, by - 14, bw, 36, null);
    const yb = by + 78;
    const go = (d) => { s.track = (s.track + d + TRACKS.length) % TRACKS.length; s.pos = 0; s.playing = true; };
    ctx.fillStyle = P.text;
    ctx.beginPath(); ctx.moveTo(cx - 98, yb); ctx.lineTo(cx - 78, yb - 13); ctx.lineTo(cx - 78, yb + 13); ctx.closePath(); ctx.fill();
    ctx.fillRect(cx - 104, yb - 13, 4, 26);
    hit('prev', cx - 124, yb - 28, 56, 56, () => go(-1));
    ctx.fillStyle = '#1db954';
    ctx.beginPath(); ctx.arc(cx, yb, 34, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = '#0b0b0b';
    if (s.playing) { ctx.fillRect(cx - 11, yb - 13, 7, 26); ctx.fillRect(cx + 4, yb - 13, 7, 26); } else {
      ctx.beginPath(); ctx.moveTo(cx - 8, yb - 15); ctx.lineTo(cx + 14, yb); ctx.lineTo(cx - 8, yb + 15); ctx.closePath(); ctx.fill();
    }
    hit('play', cx - 38, yb - 38, 76, 76, () => { s.playing = !s.playing; });
    ctx.fillStyle = P.text;
    ctx.beginPath(); ctx.moveTo(cx + 98, yb); ctx.lineTo(cx + 78, yb - 13); ctx.lineTo(cx + 78, yb + 13); ctx.closePath(); ctx.fill();
    ctx.fillRect(cx + 100, yb - 13, 4, 26);
    hit('next', cx + 68, yb - 28, 56, 56, () => go(1));
    ctx.textAlign = 'left';
    ctx.fillStyle = P.muted;
    ctx.font = `500 15px ${font}`;
    const qy = yb + 64;
    ctx.fillText(L().music.next, x0 + 26, qy);
    [1, 2].forEach((k, i) => {
      const q = TRACKS[(s.track + k) % TRACKS.length];
      const y = qy + 16 + i * 52;
      const gq = ctx.createLinearGradient(x0 + 26, y, x0 + 66, y + 40);
      gq.addColorStop(0, q.c[0]);
      gq.addColorStop(1, q.c[1]);
      ctx.fillStyle = gq;
      rr(ctx, x0 + 26, y, 40, 40, 8);
      ctx.fill();
      ctx.fillStyle = P.text;
      ctx.font = `600 17px ${font}`;
      ctx.fillText(q.t, x0 + 80, y + 18);
      ctx.fillStyle = P.muted;
      ctx.font = `400 14px ${font}`;
      ctx.fillText(q.a, x0 + 80, y + 37);
      hit('queue' + i, x0, y - 4, PW, 48, () => go(k));
    });
  }

  function chatList(x0) {
    L().chats[s.app].forEach((c, i) => {
      const y = 96 + i * 86;
      avatar(x0 + 46, y + 36, 24, c.n);
      ctx.fillStyle = P.text;
      ctx.font = `700 21px ${font}`;
      ctx.fillText(clip(c.n, PW - 180), x0 + 84, y + 30);
      ctx.fillStyle = P.muted;
      ctx.font = `400 18px ${font}`;
      ctx.fillText(clip(c.m, PW - 150), x0 + 84, y + 58);
      ctx.textAlign = 'right';
      ctx.fillText(c.t, x0 + PW - 22, y + 30);
      ctx.textAlign = 'left';
      if (c.u && !s.seen[s.app]) {
        ctx.fillStyle = P.accent;
        rr(ctx, x0 + PW - 50, y + 42, 28, 22, 11);
        ctx.fill();
        ctx.fillStyle = '#fff';
        ctx.font = `700 15px ${font}`;
        ctx.textAlign = 'center';
        ctx.fillText(String(c.u), x0 + PW - 36, y + 58);
        ctx.textAlign = 'left';
      }
      hit('row' + i, x0, y, PW, 80, () => { s.chat = i; s.sent = []; s.seen[s.app] = true; });
    });
  }

  function thread(x0) {
    const c = L().chats[s.app][s.chat];
    ctx.fillStyle = P.text;
    ctx.font = `700 30px ${font}`;
    ctx.fillText('‹', x0 + 22, 124);
    hit('back', x0 + 8, 92, 52, 52, () => { s.chat = -1; s.sent = []; });
    avatar(x0 + 80, 114, 18, c.n);
    ctx.fillStyle = P.text;
    ctx.font = `700 21px ${font}`;
    ctx.fillText(clip(c.n, PW - 150), x0 + 108, 122);
    const bubbles = [{ me: false, s: c.m }];
    if (c.more) bubbles.push({ me: false, s: c.more });
    bubbles.push(...s.sent);
    let y = 168;
    ctx.font = `400 19px ${font}`;
    bubbles.forEach((b) => {
      const w = Math.min(PW - 110, ctx.measureText(b.s).width + 32);
      const bx = b.me ? x0 + PW - 22 - w : x0 + 22;
      ctx.fillStyle = b.me ? P.mine : P.bubble;
      rr(ctx, bx, y, w, 44, 18);
      ctx.fill();
      ctx.fillStyle = b.me ? P.mineText : P.text;
      ctx.fillText(clip(b.s, w - 30), bx + 16, y + 29);
      y += 56;
    });
    let qx = x0 + 18;
    let qy = H - 150;
    ctx.font = `500 17px ${font}`;
    L().quick.forEach((q, i) => {
      const w = ctx.measureText(q).width + 28;
      if (qx + w > x0 + PW - 10) { qx = x0 + 18; qy += 46; }
      ctx.strokeStyle = P.accent;
      ctx.lineWidth = 2;
      rr(ctx, qx, qy, w, 36, 18);
      ctx.stroke();
      ctx.fillStyle = P.text;
      ctx.fillText(q, qx + 14, qy + 24);
      hit('chip' + i, qx, qy, w, 36, () => {
        s.sent.push({ me: true, s: q });
        const at = s.chat;
        setTimeout(() => { if (s.chat === at && s.sent.length < 5) { s.sent.push({ me: false, s: L().reply }); changed(); } }, 1300);
      });
      qx += w + 10;
    });
    ctx.fillStyle = P.line;
    rr(ctx, x0 + 18, H - 62, PW - 36, 44, 22);
    ctx.fill();
    ctx.fillStyle = P.muted;
    ctx.font = `400 18px ${font}`;
    ctx.fillText(L().type, x0 + 38, H - 33);
  }

  function drawPanel() {
    const e = Math.max(0, panel.x);
    if (e <= 0.002) return;
    const x0 = W - PW * e;
    hit('panel', x0, 0, PW, H, null);
    ctx.save();
    ctx.shadowColor = 'rgba(0,0,0,0.28)';
    ctx.shadowBlur = 40;
    ctx.fillStyle = P.panel;
    rr(ctx, x0, 0, PW + 40, H, [22, 0, 0, 22]);
    ctx.fill();
    ctx.restore();
    ctx.save();
    rr(ctx, x0, 0, PW + 40, H, [22, 0, 0, 22]);
    ctx.clip();
    APPS.forEach((id, i) => {
      const ix = x0 + 18 + i * 57;
      const iy = 16;
      if (id === s.app) {
        ctx.fillStyle = P.line;
        rr(ctx, ix - 6, iy - 6, 52, 52, 14);
        ctx.fill();
      }
      const img = iconImage(id, P.text);
      if (img.complete) ctx.drawImage(img, ix + 2, iy + 2, 36, 36);
      const u = unread(id);
      if (u) {
        ctx.fillStyle = P.badge;
        ctx.beginPath();
        ctx.arc(ix + 38, iy + 2, 11, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = '#fff';
        ctx.font = `700 15px ${font}`;
        ctx.textAlign = 'center';
        ctx.fillText(String(u), ix + 38, iy + 7);
        ctx.textAlign = 'left';
      }
      hit('app' + i, ix - 6, iy - 6, 52, 52, () => { s.app = id; s.chat = -1; s.sent = []; });
    });
    ctx.fillStyle = P.line;
    ctx.fillRect(x0, 78, PW, 2);
    if (s.app === 'spotify') player(x0);
    else if (s.chat < 0) chatList(x0);
    else thread(x0);
    ctx.restore();
  }

  function drawTab() {
    const e = Math.max(0, tab.x) * (1 - Math.min(1, Math.max(0, panel.x)));
    if (e <= 0.002) return;
    const x = W - TAB.w * e;
    ctx.save();
    ctx.shadowColor = 'rgba(0,0,0,0.3)';
    ctx.shadowBlur = 18;
    ctx.fillStyle = P.tab;
    rr(ctx, x, TAB.y, TAB.w + 20, TAB.h, [14, 0, 0, 14]);
    ctx.fill();
    ctx.restore();
    ctx.fillStyle = 'rgba(0,0,0,0.18)';
    rr(ctx, x + TAB.w / 2 - 2, TAB.y + TAB.h / 2 - 16, 4, 32, 2);
    ctx.fill();
    const u = total();
    if (u) {
      const r = 15 * Math.min(1, e);
      ctx.fillStyle = P.badge;
      ctx.beginPath();
      ctx.arc(x - 2, TAB.y + 4, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = '#fff';
      ctx.font = `700 16px ${font}`;
      ctx.textAlign = 'center';
      ctx.fillText(String(u), x - 2, TAB.y + 10);
      ctx.textAlign = 'left';
    }
    const pad = touch ? 150 : 40;
    hit('tab', x - pad, TAB.y - 30, TAB.w + pad + 20, TAB.h + 60, () => open(true));
  }

  function drawCursor() {
    if (!s.demo) return; // a real mouse keeps its own cursor
    const { x, y } = s.cursor;
    ctx.save();
    ctx.translate(x, y);
    ctx.scale(1.5, 1.5);
    ctx.fillStyle = '#fff';
    ctx.strokeStyle = '#000';
    ctx.lineWidth = 1.4;
    ctx.beginPath();
    ctx.moveTo(0, 0); ctx.lineTo(0, 22); ctx.lineTo(6, 17); ctx.lineTo(10, 26);
    ctx.lineTo(14, 24); ctx.lineTo(10, 15.5); ctx.lineTo(17, 15.5);
    ctx.closePath();
    ctx.fill();
    ctx.stroke();
    ctx.restore();
  }

  function open(v) {
    if (s.open === v) return;
    s.open = v;
    s.hold = 0;
    if (!v) s.linger = 0.6;
    changed();
  }

  function click(x, y) {
    for (let i = s.hits.length - 1; i >= 0; i--) {
      const h = s.hits[i];
      if (x >= h.x && x <= h.x + h.w && y >= h.y && y <= h.y + h.h) {
        if (h.tag === 'seek') { s.pos = TRACKS[s.track].d * Math.min(1, Math.max(0, (x - h.x) / h.w)); return; }
        if (h.fn) { h.fn(); changed(); }
        if (h.fn || h.tag === 'panel') return;
      }
    }
    if (s.open) open(false);
    else if (touch && x > W * 0.78) open(true); // on a phone, a tap near the right edge is enough
  }

  const clickable = (x, y) => s.hits.some((h) => (h.fn || h.tag === 'seek') && x >= h.x && x <= h.x + h.w && y >= h.y && y <= h.y + h.h);

  // ---- the demo plays itself while nobody touches the screen ----
  const find = (tag) => {
    const h = s.hits.find((k) => k.tag === tag);
    return h ? [h.x + h.w / 2, h.y + h.h / 2] : [W * 0.5, H * 0.5];
  };
  function startDemo() {
    s.open = false; s.app = 'discord'; s.chat = -1; s.sent = []; s.seen = {};
    s.demo = {
      i: -1, phase: 'next', t0: 0, steps: [
        { to: () => [W * 0.42, H * 0.58], d: 650, wait: 300 },
        { to: () => [W - 3, H * 0.74], d: 1000, wait: 700 },
        { to: () => [W - 8, TAB.y + TAB.h / 2], d: 550, wait: 1300 },
        { to: () => find('row0'), d: 750, wait: 450, click: true },
        { to: () => find('chip0'), d: 800, wait: 2200, click: true },
        { to: () => find('app6'), d: 800, wait: 2400, click: true },
        { to: () => [W * 0.32, H * 0.5], d: 900, wait: 400, click: true },
        { to: () => [W * 0.5, H * 0.62], d: 700, wait: 2600 },
      ],
    };
  }
  function runDemo(now) {
    const d = s.demo;
    if (!d) return;
    if (d.phase === 'next') {
      d.i += 1;
      if (d.i >= d.steps.length) { s.demo = null; s.idleUntil = now + 2500; return; }
      d.from = [s.cursor.x, s.cursor.y];
      d.to = d.steps[d.i].to();
      d.t0 = now;
      d.phase = 'move';
    }
    const st = d.steps[d.i];
    if (d.phase === 'move') {
      const k = Math.min(1, (now - d.t0) / st.d);
      const e = ease(k);
      s.cursor.x = d.from[0] + (d.to[0] - d.from[0]) * e;
      s.cursor.y = d.from[1] + (d.to[1] - d.from[1]) * e;
      if (k >= 1) {
        if (st.click) click(s.cursor.x, s.cursor.y);
        d.phase = 'wait';
        d.t0 = now;
      }
    } else if (now - d.t0 >= st.wait) d.phase = 'next';
  }
  const stopDemo = (ms) => { s.demo = null; s.idleUntil = performance.now() + ms; };

  // ---- the loop ----
  let visible = true;
  let last = performance.now();
  new IntersectionObserver((en) => { visible = en[0].isIntersecting; }).observe(canvas);
  function frame(now) {
    requestAnimationFrame(frame);
    if (visible && !document.hidden) draw(now);
    else last = now;
  }
  function draw(now) {
    const dt = Math.min(0.034, Math.max(0, (now - last) / 1000));
    last = now;
    s.t += still ? 0 : dt;
    if (!still && !s.demo && !s.mouse.on && now > s.idleUntil) startDemo();
    runDemo(now);
    const pointer = s.demo ? s.cursor : s.mouse;
    const live = s.demo || s.mouse.on;
    const atEdge = live && pointer.x > W - 34;
    const onTab = atEdge && pointer.y > TAB.y - 20 && pointer.y < TAB.y + TAB.h + 20;
    s.linger = atEdge || touch ? 1.2 : Math.max(0, s.linger - dt);
    // holding the cursor on the tab for half a second opens it, like ChatDock's edge hold
    if (onTab && !s.open && tab.x > 0.85) {
      s.hold += dt;
      if (s.hold > 0.5) open(true);
    } else s.hold = 0;
    if (s.playing) {
      s.pos += dt;
      if (s.pos >= TRACKS[s.track].d) { s.track = (s.track + 1) % TRACKS.length; s.pos = 0; }
    }
    tab.to = s.linger > 0 || s.open ? 1 : 0;
    panel.to = s.open ? 1 : 0;
    if (still) { tab.x = tab.to; panel.x = panel.to; } else { tab.step(dt); panel.step(dt); }

    ctx.setTransform(scale, 0, 0, scale, 0, 0);
    s.hits = [];
    game();
    drawPanel();
    drawTab();
    if (s.hold > 0) {
      ctx.strokeStyle = P.badge;
      ctx.lineWidth = 4;
      ctx.beginPath();
      ctx.arc(pointer.x + 26, pointer.y + 30, 16, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * (s.hold / 0.5));
      ctx.stroke();
    }
    drawCursor();
  }
  requestAnimationFrame(frame);

  // ---- input ----
  const at = (e) => {
    const r = canvas.getBoundingClientRect();
    return [((e.clientX - r.left) / r.width) * W, ((e.clientY - r.top) / r.height) * H];
  };
  canvas.addEventListener('pointermove', (e) => {
    if (e.pointerType === 'touch') return;
    const [x, y] = at(e);
    s.mouse = { x, y, on: true };
    stopDemo(4000);
    canvas.style.cursor = clickable(x, y) ? 'pointer' : 'default';
  });
  canvas.addEventListener('pointerleave', () => { s.mouse.on = false; stopDemo(4000); });
  canvas.addEventListener('click', (e) => {
    const [x, y] = at(e);
    stopDemo(touch ? 9000 : 4000);
    click(x, y);
  });

  return {
    state: s,
    draw, // one frame by hand (for checks while the page is in the background)
    onChange: (f) => listeners.add(f),
    setLang(l) { s.lang = SCREEN[l] ? l : 'en'; canvas.setAttribute('aria-label', 'ChatDock'); },
    openApp(id) {
      if (!APPS.includes(id)) return;
      stopDemo(12000);
      s.app = id; s.chat = -1; s.sent = [];
      open(true);
      changed();
    },
  };
}
