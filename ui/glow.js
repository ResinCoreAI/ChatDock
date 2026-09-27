'use strict';

const bar = document.getElementById('bar');

function restart(cls) {
  bar.classList.remove('pulse', 'in');
  void bar.offsetWidth;
  bar.classList.add(cls);
}

// colors: gradient stops of every app that has unread chats (e.g. IG pink + Discord blurple)
chatdock.on('glow:state', ({ colors, pulse, side, appear }) => {
  document.body.classList.toggle('left', side === 'left');
  const stops = colors && colors.length ? colors : ['#0866ff'];
  bar.style.setProperty('--c', stops.length > 1 ? `linear-gradient(${stops.join(', ')})` : stops[0]);
  bar.style.setProperty('--g', `color-mix(in srgb, ${stops[Math.floor(stops.length / 2)]} 80%, transparent)`);
  bar.classList.remove('out');
  if (pulse) restart('pulse'); // a new message: grows in with a flash
  else if (appear) restart('in'); // back after being hidden: fades in
});

// Nothing unread any more: fade out (the main process hides the window afterwards).
chatdock.on('glow:hide', () => bar.classList.add('out'));

chatdock.send('ui:ready');
