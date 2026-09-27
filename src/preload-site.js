'use strict';

// Preload for the chat sites (Instagram, Facebook, X, Discord, ...). It exposes nothing to the
// page and never reads the page. It does two things:
//
// 1. Passkey guard. Login pages (Meta, and the "Continue with Google" frame on X) ask for a
//    passkey on their own. Inside ChatDock that pops a "Windows Security" dialog out of nowhere
//    (even mid-game or with the PC locked) and can keep the app from quitting. Passkeys are
//    refused in every frame and the page falls back to password / QR login. (main.js also sends
//    a Permissions-Policy header that switches passkeys off for every frame.)
//
// 2. Notifications. The sites announce new messages with the web Notification API. Windows'
//    own toasts are muted while gaming, so the sender / text / picture of each notification the
//    site creates is handed to ChatDock, which shows its own always-on-top pop-up instead.
//    Clicking that pop-up runs the site's own click handler, so the site opens the conversation.

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.executeInMainWorld({
  func: (report) => {
    const creds = navigator.credentials;
    if (!creds) return;
    const proto = Object.getPrototypeOf(creds);
    const guard = (original, kind) => function guarded(options) {
      if (options && options.publicKey) {
        try {
          report({ kind, origin: location.origin, mediation: String(options.mediation || '') });
        } catch {
          // ignore
        }
        return Promise.reject(new DOMException('The operation is not allowed.', 'NotAllowedError'));
      }
      return original.call(this, options);
    };
    proto.get = guard(proto.get, 'get');
    proto.create = guard(proto.create, 'create');
    if (window.PublicKeyCredential) {
      PublicKeyCredential.isConditionalMediationAvailable = () => Promise.resolve(false);
      PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable = () => Promise.resolve(false);
    }
  },
  args: [(info) => ipcRenderer.send('site:passkey-blocked', info)],
});

let bridge = null;
try {
  bridge = contextBridge.executeInMainWorld({
    func: (send) => {
      let seq = 0;
      const live = new Map(); // id -> notification the page still holds
      const ids = new WeakMap();
      const abs = (u) => {
        try {
          return u ? new URL(String(u), location.href).href : '';
        } catch {
          return '';
        }
      };
      const post = (payload) => {
        try {
          send(payload);
        } catch {
          // ChatDock unavailable: nothing to do
        }
      };
      const fire = (n, type) => {
        const event = new Event(type);
        const handler = n[`on${type}`];
        if (typeof handler === 'function') {
          try {
            handler.call(n, event);
          } catch (err) {
            setTimeout(() => { throw err; });
          }
        }
        n.dispatchEvent(event);
      };

      class Notification extends EventTarget {
        constructor(title, options) {
          super();
          const o = options || {};
          const id = ++seq;
          ids.set(this, id);
          const ro = (name, value) => Object.defineProperty(this, name, { value, enumerable: true });
          ro('title', String(title ?? ''));
          ro('body', String(o.body ?? ''));
          ro('icon', abs(o.icon));
          ro('image', abs(o.image));
          ro('badge', abs(o.badge));
          ro('tag', String(o.tag ?? ''));
          ro('data', o.data ?? null);
          ro('dir', o.dir || 'auto');
          ro('lang', o.lang || '');
          ro('silent', !!o.silent);
          ro('requireInteraction', !!o.requireInteraction);
          ro('renotify', !!o.renotify);
          ro('timestamp', typeof o.timestamp === 'number' ? o.timestamp : Date.now());
          ro('actions', []);
          ro('vibrate', []);
          this.onclick = null;
          this.onshow = null;
          this.onclose = null;
          this.onerror = null;
          live.set(id, this);
          post({ id, title: this.title, body: this.body, icon: this.icon || this.image, tag: this.tag });
          setTimeout(() => fire(this, 'show'), 0);
        }

        close() {
          const id = ids.get(this);
          if (!live.delete(id)) return;
          post({ id, closed: true });
          fire(this, 'close');
        }

        static get permission() {
          return 'granted';
        }

        static requestPermission(callback) {
          if (typeof callback === 'function') callback('granted');
          return Promise.resolve('granted');
        }

        static get maxActions() {
          return 2;
        }
      }
      Object.defineProperty(window, 'Notification', { value: Notification, writable: true, configurable: true });

      // Some sites (WhatsApp) show notifications through their service worker registration.
      if (window.ServiceWorkerRegistration) {
        const swProto = ServiceWorkerRegistration.prototype;
        swProto.showNotification = function showNotification(title, options) {
          const o = options || {};
          post({ id: ++seq, title: String(title ?? ''), body: String(o.body ?? ''), icon: abs(o.icon || o.image), tag: String(o.tag ?? ''), sw: true });
          return Promise.resolve();
        };
        swProto.getNotifications = function getNotifications() {
          return Promise.resolve([]);
        };
      }

      return {
        click(id) {
          const n = live.get(id);
          if (!n) return false;
          try {
            window.focus();
          } catch {
            // ignore
          }
          fire(n, 'click');
          return true;
        },
      };
    },
    args: [(payload) => ipcRenderer.send('site:notification', payload)],
  });
} catch {
  bridge = null; // older Electron without executeInMainWorld args: sites fall back to unread counts
}

ipcRenderer.on('site:notification-click', (_event, id) => {
  try {
    if (bridge) bridge.click(id);
  } catch {
    // the page may have navigated away
  }
});
