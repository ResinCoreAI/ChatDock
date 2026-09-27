'use strict';

// Bridge for ChatDock's own UI pages (panel header + settings, edge tab, glow, pop-ups, update window). The chat
// sites never get this bridge (they only get preload-site.js).

const { contextBridge, ipcRenderer } = require('electron');

const SEND = new Set([
  'ui:ready', 'app:select', 'panel:reload', 'panel:retry', 'panel:pin', 'panel:hide', 'panel:settings',
  'panel:update', 'panel:resize', 'panel:zoom-reset', 'onboarding:done', 'banner:dismiss', 'tab:open', 'tab:log',
  'settings:set', 'settings:app', 'settings:app-pref', 'settings:action',
  'toast:size', 'toast:click', 'toast:dismiss', 'toast:more', 'toast:hover',
]);
const RECEIVE = new Set(['state', 'settings:goto', 'tab:show', 'tab:hide', 'glow:state', 'toasts', 'update:show']);

contextBridge.exposeInMainWorld('chatdock', {
  send(channel, ...args) {
    if (SEND.has(channel)) ipcRenderer.send(channel, ...args);
  },
  on(channel, fn) {
    if (RECEIVE.has(channel)) ipcRenderer.on(channel, (_event, ...args) => fn(...args));
  },
});
