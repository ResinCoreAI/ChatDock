'use strict';

// The intro card (intro.rs): ChatDock sends the texts, then how far the chats have loaded (0..1),
// then "done" when it is time to go. A click on the card skips the wait.
const card = document.getElementById('card');
const fill = document.getElementById('fill');
let leaving = false;

chatdock.on('intro:show', ({ build, skip, loading }) => {
  document.getElementById('build').textContent = build || '';
  document.getElementById('foot').textContent = `© 2026 ResinCore · ${skip || ''}`;
  card.setAttribute('aria-label', loading || '');
  card.classList.add('in');
});

chatdock.on('intro:progress', (part) => {
  fill.style.width = `${Math.round(Math.max(0, Math.min(1, Number(part) || 0)) * 100)}%`;
});

chatdock.on('intro:done', () => {
  if (leaving) return;
  leaving = true;
  fill.style.width = '100%';
  card.classList.add('out'); // (keeps "in": the logo and texts stay where they ended up)
});

document.addEventListener('mousedown', (e) => {
  e.preventDefault(); // (no focus to the card)
  if (!leaving) chatdock.send('intro:skip');
});

chatdock.send('ui:ready');
