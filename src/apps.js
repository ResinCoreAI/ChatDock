'use strict';

// The chat services ChatDock can host. Each one gets its own login session (partition).
//   domains      pages that stay inside the panel (anything else opens in the normal browser)
//   authDomains  sign-in pop-ups that may open as a small window (e.g. "Sign in with Google" on X)
//   plainTitle   what the site's normal tab title looks like, so other titles can be read as
//                "someone messaged you" flashes
//   colors       gradient used for the unread glow on the screen edge
//   width        starting panel width for sites that need more room (Discord's sidebars)
// LINE has no web version, so it can't be added here.

const CATALOG = [
  {
    id: 'instagram',
    name: 'Instagram',
    icon: 'instagram',
    home: 'https://www.instagram.com/direct/inbox/',
    homePath: '/direct/',
    colors: ['#feda75', '#fa7e1e', '#d62976', '#962fbf'],
    domains: ['instagram.com', 'cdninstagram.com', 'facebook.com', 'fb.com', 'fbcdn.net', 'facebook.net', 'fbsbx.com', 'meta.com'],
    plainTitle: /instagram/i,
    enabledByDefault: true,
  },
  {
    id: 'facebook',
    name: 'Facebook',
    icon: 'messenger',
    // messenger.com was retired in April 2026; Facebook chats now live here.
    home: 'https://www.facebook.com/messages/',
    homePath: '/messages',
    colors: ['#4d9bff', '#0866ff'],
    domains: ['facebook.com', 'messenger.com', 'fb.com', 'fbcdn.net', 'facebook.net', 'fbsbx.com', 'meta.com'],
    plainTitle: /facebook|messenger/i,
    enabledByDefault: true,
  },
  {
    id: 'x',
    name: 'X',
    icon: 'x',
    home: 'https://x.com/messages',
    homePath: '/messages',
    colors: ['#ffffff', '#a1a1aa'],
    domains: ['x.com', 'twitter.com', 'twimg.com'],
    authDomains: ['accounts.google.com', 'appleid.apple.com'],
    plainTitle: /(^|[\s/])X$|twitter/i,
    enabledByDefault: true,
  },
  {
    id: 'discord',
    name: 'Discord',
    icon: 'discord',
    home: 'https://discord.com/channels/@me',
    homePath: '/channels',
    width: 800, // server list + channel list + chat don't fit in less
    colors: ['#8b93ff', '#5865f2'],
    domains: ['discord.com', 'discordapp.com', 'discordapp.net', 'discord.gg', 'discord.media'],
    plainTitle: /discord/i,
    enabledByDefault: true,
  },
  {
    id: 'telegram',
    name: 'Telegram',
    icon: 'telegram',
    home: 'https://web.telegram.org/k/',
    homePath: '/k/',
    colors: ['#6fcbf7', '#2aabee'],
    domains: ['web.telegram.org', 'telegram.org', 't.me'],
    plainTitle: /telegram/i,
    enabledByDefault: false,
  },
  {
    id: 'whatsapp',
    name: 'WhatsApp',
    icon: 'whatsapp',
    home: 'https://web.whatsapp.com/',
    homePath: '/',
    width: 700, // chat list + conversation side by side
    colors: ['#6ee89b', '#25d366'],
    domains: ['web.whatsapp.com', 'whatsapp.com', 'whatsapp.net'],
    plainTitle: /whatsapp/i,
    enabledByDefault: false,
  },
];

const BY_ID = Object.fromEntries(CATALOG.map((a) => [a.id, { ...a, partition: `persist:${a.id}` }]));
const ALL_IDS = CATALOG.map((a) => a.id);

function get(id) {
  return BY_ID[id] || null;
}

function defaultsEnabled() {
  return Object.fromEntries(CATALOG.map((a) => [a.id, a.enabledByDefault]));
}

function hostMatches(hostname, list) {
  return (list || []).some((d) => hostname === d || hostname.endsWith(`.${d}`));
}

// Does this https URL belong to the app (so it may stay inside the panel)?
function owns(id, url) {
  const a = BY_ID[id];
  if (!a) return false;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' && hostMatches(u.hostname, a.domains);
  } catch {
    return false;
  }
}

function isAuthPopup(id, url) {
  const a = BY_ID[id];
  if (!a) return false;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' && hostMatches(u.hostname, a.authDomains);
  } catch {
    return false;
  }
}

module.exports = { CATALOG, ALL_IDS, get, defaultsEnabled, owns, isAuthPopup };
