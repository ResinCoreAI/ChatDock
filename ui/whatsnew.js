'use strict';

// "What's new" after an update: "ChatDock is now on Beta Build 1.5.3 ✓", then what changed since the
// version the user had, one section per release. ChatDock sends the texts (already translated),
// sizes the window to the card and shows it.
const card = document.querySelector('.card');
const notes = document.getElementById('notes');
let closing = false;

// The card's full height with every line showing, plus the room around it for the shadow (CSS px).
// Its layout height: the pop-in animation scales what getBoundingClientRect() would report.
function naturalHeight() {
  card.classList.add('measure');
  const h = card.offsetHeight;
  card.classList.remove('measure');
  return h + 33;
}

chatdock.on('whatsnew:show', ({ locale, title, route, sections, ok, github, close }) => {
  document.documentElement.lang = locale;
  document.getElementById('title').textContent = title;
  document.getElementById('route').textContent = route;
  document.getElementById('ok').textContent = ok;
  document.getElementById('github').textContent = github;
  const x = document.getElementById('x');
  x.title = close;
  x.setAttribute('aria-label', close);
  notes.replaceChildren();
  for (const s of sections) {
    if (s.title) {
      const h = document.createElement('h3');
      h.textContent = s.title;
      notes.append(h);
    }
    const ul = document.createElement('ul');
    for (const line of s.lines) {
      const li = document.createElement('li');
      li.textContent = line;
      ul.append(li);
    }
    notes.append(ul);
  }
  notes.scrollTop = 0;
  chatdock.send('whatsnew:size', naturalHeight());
  card.classList.remove('in');
  void card.offsetWidth; // restart the pop-in animation
  card.classList.add('in');
});

function close(action) {
  if (closing) return;
  closing = true;
  card.classList.remove('in');
  card.classList.add('out');
  setTimeout(() => chatdock.send(action), 150);
}

document.getElementById('ok').addEventListener('click', () => close('whatsnew:close'));
document.getElementById('x').addEventListener('click', () => close('whatsnew:close'));
document.getElementById('github').addEventListener('click', () => close('whatsnew:releases'));
document.addEventListener('keydown', (e) => {
  // Enter on a focused button is that button's click
  if (e.key === 'Escape' || (e.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement))) {
    close('whatsnew:close');
  }
});

chatdock.send('ui:ready');
