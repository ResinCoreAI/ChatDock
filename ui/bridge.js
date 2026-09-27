'use strict';

// Bridge between ChatDock's own pages and the app (Rust side), loaded first on every page. Pages
// talk to it the same way everywhere: chatdock.send(channel, ...args) and chatdock.on(channel, fn).
// Messages arrive only for this page's own window; each one carries its arguments as an array.
// Registering a listener takes a moment, so sending waits until every listener so far is in place
// (otherwise the answer to 'ui:ready' could arrive before anyone listens for it).
(function () {
  const tauri = window.__TAURI__;
  const me = tauri.webviewWindow.getCurrentWebviewWindow();
  const listening = [];
  window.chatdock = {
    send(channel, ...args) {
      Promise.all(listening)
        .then(() => tauri.core.invoke('ui_send', { channel, args }))
        .catch(() => {});
    },
    on(channel, fn) {
      listening.push(me.listen(channel, (e) => fn(...(Array.isArray(e.payload) ? e.payload : [e.payload]))));
    },
  };
})();
