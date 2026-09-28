'use strict';

// "Show numbers on the screens": the monitor's number (the one Settings' map uses), its name and size,
// and whether the chat opens there. ChatDock places the window and shows it.
const card = document.getElementById('card');

chatdock.on('identify:show', ({ n, name, detail, chat, chatText, main, mainText }) => {
  document.getElementById('num').textContent = String(n);
  document.getElementById('name').textContent = name;
  document.getElementById('detail').textContent = detail;
  const m = document.getElementById('main');
  m.hidden = !main;
  m.textContent = mainText;
  document.getElementById('chat').hidden = !chat;
  document.getElementById('chat-text').textContent = chatText;
  card.classList.remove('in');
  void card.offsetWidth; // pop in again each time it is shown
  card.classList.add('in');
});

chatdock.send('ui:ready');
