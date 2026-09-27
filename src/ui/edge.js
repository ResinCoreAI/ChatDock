'use strict';

// The line that grows along the dock edge while the cursor is held against it. It starts at the
// cursor and reaches the top and the bottom of the screen together, at the moment the tab comes
// out. Drawn on every screen refresh (requestAnimationFrame follows the screen: 300 frames a
// second on a 300 Hz screen); the main process only reports where the cursor is.

const canvas = document.getElementById('line');
const ctx = canvas.getContext('2d');

const FADE_IN_MS = 90;
const DONE_MS = 340; // lights up, then fades while the tab slides out
const CANCEL_MS = 230; // shrinks back into the cursor
const FOLLOW_MS = 45; // the line's centre catches up with the cursor in about this time

let W = 0;
let H = 0;
let grad = null;
let mode = 'idle'; // hold | done | cancel | idle
let side = 'right';
let startAt = 0; // when the hold began (this page's clock)
let total = 3000;
let shownAt = 0;
let endAt = 0; // when "done" / "cancel" began
let endP = 0; // how far the line had grown then
let target = 0; // cursor height
let y = 0; // drawn centre, follows the cursor
let lastT = 0;
let raf = 0;

function fit() {
  const dpr = window.devicePixelRatio || 1;
  W = window.innerWidth;
  H = window.innerHeight;
  canvas.width = Math.round(W * dpr);
  canvas.height = Math.round(H * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  grad = ctx.createLinearGradient(0, 0, 0, H);
  grad.addColorStop(0, '#c084fc');
  grad.addColorStop(0.4, '#8b5cf6');
  grad.addColorStop(0.75, '#3b82f6');
  grad.addColorStop(1, '#0866ff');
}

function stroke(x, from, to) {
  ctx.beginPath();
  ctx.moveTo(x, from);
  ctx.lineTo(x, to);
  ctx.stroke();
}

function dot(x, cy, r) {
  ctx.beginPath();
  ctx.arc(x, cy, r, 0, Math.PI * 2);
  ctx.fill();
}

// p: how far it has grown (0 = a dot at the cursor, 1 = top to bottom), alpha, flash: 0..1
function paint(p, alpha, flash) {
  const x = side === 'left' ? 2 : W - 2;
  const cy = Math.max(0, Math.min(H, y));
  const top = cy * (1 - p);
  const bottom = cy + (H - cy) * p;
  ctx.lineCap = 'round';

  // faint track: where the line is heading
  ctx.globalAlpha = alpha * 0.5;
  ctx.shadowBlur = 0;
  ctx.lineWidth = 2;
  ctx.strokeStyle = 'rgba(167, 139, 250, .35)';
  stroke(x, 0, H);

  // the line, glowing more as it grows
  ctx.globalAlpha = alpha;
  ctx.lineWidth = 3.5 + flash * 1.5;
  ctx.strokeStyle = grad;
  ctx.shadowColor = 'rgba(139, 92, 246, .9)';
  ctx.shadowBlur = 7 + 7 * p + 12 * flash;
  stroke(x, top, bottom);
  if (flash > 0) {
    ctx.strokeStyle = `rgba(255, 255, 255, ${0.75 * flash})`;
    ctx.shadowColor = 'rgba(255, 255, 255, .9)';
    stroke(x, top, bottom);
  }

  // bright heads at both ends
  ctx.fillStyle = '#f5f3ff';
  ctx.shadowColor = '#a78bfa';
  ctx.shadowBlur = 10 + 10 * flash;
  dot(x, top, 2.4 + flash);
  dot(x, bottom, 2.4 + flash);

  ctx.shadowBlur = 0;
  ctx.globalAlpha = 1;
}

function frame(t) {
  raf = 0;
  if (window.innerWidth !== W || window.innerHeight !== H) fit(); // moved to another screen / resized
  const dt = lastT ? Math.min(50, t - lastT) : 1000 / 60;
  lastT = t;
  y += (target - y) * (1 - Math.exp(-dt / FOLLOW_MS));
  let p = 0;
  let alpha = 0;
  let flash = 0;
  if (mode === 'hold') {
    p = Math.min(1, (t - startAt) / total);
    alpha = Math.min(1, (t - shownAt) / FADE_IN_MS);
  } else if (mode === 'done') {
    const k = Math.min(1, (t - endAt) / DONE_MS);
    p = 1;
    flash = 1 - k;
    alpha = 1 - k * k;
    if (k >= 1) mode = 'idle';
  } else if (mode === 'cancel') {
    const k = Math.min(1, (t - endAt) / CANCEL_MS);
    const e = 1 - (1 - k) ** 3;
    p = endP * (1 - e);
    alpha = (1 - e) * Math.min(1, (endAt - shownAt) / FADE_IN_MS);
    if (k >= 1) mode = 'idle';
  }
  ctx.clearRect(0, 0, W, H);
  if (mode === 'idle') { // last frame is empty, so the next show never flashes an old line
    lastT = 0;
    return;
  }
  paint(p, alpha, flash);
  raf = requestAnimationFrame(frame);
}

function run() {
  if (!raf) raf = requestAnimationFrame(frame);
}

chatdock.on('edge:start', ({ y: cy, held, total: ms, side: s }) => {
  const now = performance.now();
  side = s === 'left' ? 'left' : 'right';
  total = Math.max(1, Number(ms) || 3000);
  startAt = now - Math.max(0, Number(held) || 0);
  shownAt = now;
  target = Number(cy) || 0;
  y = target; // a fresh line starts right at the cursor
  mode = 'hold';
  run();
});

chatdock.on('edge:move', (cy) => {
  target = Number(cy) || 0;
});

chatdock.on('edge:done', () => {
  if (mode !== 'hold') return;
  mode = 'done';
  endAt = performance.now();
  run();
});

chatdock.on('edge:cancel', () => {
  if (mode !== 'hold') return;
  endAt = performance.now();
  endP = Math.min(1, (endAt - startAt) / total);
  mode = 'cancel';
  run();
});

window.addEventListener('resize', fit);
fit();
