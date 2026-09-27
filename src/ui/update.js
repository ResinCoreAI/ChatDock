'use strict';

// "Updating ChatDock" window: from → to, a progress bar that fills while ChatDock gets ready, then
// the installing step (the installer's own window shows the rest).
const fill = document.getElementById('fill');

chatdock.on('update:show', ({ lang, from, to, ms }) => {
  i18n.set(lang);
  i18n.apply();
  document.getElementById('route').textContent = `${from} → ${to}`;
  const step = document.getElementById('step');
  step.textContent = i18n.t('updwin.preparing');
  fill.style.transitionDuration = `${ms}ms`;
  requestAnimationFrame(() => { fill.style.width = '100%'; });
  setTimeout(() => { step.textContent = i18n.t('updwin.installing'); }, Math.round(ms * 0.45));
});
