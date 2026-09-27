'use strict';

const bar = document.getElementById('bar');

// colors: gradient stops of every app that has unread chats (e.g. IG pink + Discord blurple)
chatdock.on('glow:state', ({ colors, pulse, side }) => {
  document.body.classList.toggle('left', side === 'left');
  const stops = colors && colors.length ? colors : ['#0866ff'];
  bar.style.setProperty('--c', stops.length > 1 ? `linear-gradient(${stops.join(', ')})` : stops[0]);
  bar.style.setProperty('--g', `color-mix(in srgb, ${stops[Math.floor(stops.length / 2)]} 80%, transparent)`);
  if (pulse) {
    bar.classList.remove('pulse');
    void bar.offsetWidth;
    bar.classList.add('pulse');
  }
});

chatdock.send('ui:ready');
