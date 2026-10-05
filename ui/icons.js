'use strict';

// Inline SVG icons shared by ChatDock's own pages. Fills every [data-icon] element.
window.ICONS = {
  instagram: `<svg viewBox="0 0 24 24" aria-hidden="true"><defs><linearGradient id="ig-g" x1="0" y1="1" x2="1" y2="0">
    <stop offset="0" stop-color="#feda75"/><stop offset=".3" stop-color="#fa7e1e"/><stop offset=".55" stop-color="#d62976"/>
    <stop offset=".8" stop-color="#962fbf"/><stop offset="1" stop-color="#4f5bd5"/></linearGradient></defs>
    <rect x="3" y="3" width="18" height="18" rx="5.5" fill="none" stroke="url(#ig-g)" stroke-width="2.1"/>
    <circle cx="12" cy="12" r="4.1" fill="none" stroke="url(#ig-g)" stroke-width="2.1"/>
    <circle cx="17.4" cy="6.6" r="1.25" fill="url(#ig-g)"/></svg>`,
  // the blue circle with a white "f" whose stem runs off the bottom (the white "f" is drawn under the
  // circle's cut-out, so it stays white on dark backgrounds too)
  facebook: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9.101 23.691v-7.98H6.627v-3.667h2.474v-1.58c0-4.085 1.848-5.978 5.858-5.978.401 0 .955.042 1.468.103a8.68 8.68 0 0 1 1.141.195v3.325a8.623 8.623 0 0 0-.653-.036 26.805 26.805 0 0 0-.733-.009c-.707 0-1.259.096-1.675.309a1.686 1.686 0 0 0-.679.622c-.258.42-.374.995-.374 1.752v1.297h3.919l-.386 2.103-.287 1.564h-3.246v8.245A12 12 0 0 1 9.101 23.691z" fill="#fff"/>
    <path d="M9.101 23.691v-7.98H6.627v-3.667h2.474v-1.58c0-4.085 1.848-5.978 5.858-5.978.401 0 .955.042 1.468.103a8.68 8.68 0 0 1 1.141.195v3.325a8.623 8.623 0 0 0-.653-.036 26.805 26.805 0 0 0-.733-.009c-.707 0-1.259.096-1.675.309a1.686 1.686 0 0 0-.679.622c-.258.42-.374.995-.374 1.752v1.297h3.919l-.386 2.103-.287 1.564h-3.246v8.245C19.396 23.238 24 18.179 24 12.044c0-6.627-5.373-12-12-12s-12 5.373-12 12c0 5.628 3.874 10.35 9.101 11.647z" fill="#0866ff"/></svg>`,
  x: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18.2 2.5h3.3l-7.2 8.2 8.5 10.8h-6.6l-5.2-6.7-5.9 6.7H1.8l7.7-8.8L1.4 2.5h6.8l4.7 6.2 5.3-6.2zm-1.2 17h1.8L7.1 4.3H5.2z" fill="currentColor"/></svg>`,
  discord: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20.3 4.4A19.6 19.6 0 0 0 15.4 3l-.6 1.3a18.3 18.3 0 0 0-5.6 0L8.6 3a19.5 19.5 0 0 0-4.9 1.5C.6 9.1-.2 13.6.2 18a19.7 19.7 0 0 0 6 3l1.3-2.1a12.8 12.8 0 0 1-2-1l.5-.4a14 14 0 0 0 12 0l.5.4c-.6.4-1.3.7-2 1l1.3 2.1a19.6 19.6 0 0 0 6-3c.5-5.1-.8-9.6-3.5-13.6zM8 15.3c-1.2 0-2.1-1.1-2.1-2.4S6.8 10.5 8 10.5s2.2 1.1 2.1 2.4c0 1.3-.9 2.4-2.1 2.4zm8 0c-1.2 0-2.1-1.1-2.1-2.4s.9-2.4 2.1-2.4 2.2 1.1 2.1 2.4c0 1.3-.9 2.4-2.1 2.4z" fill="#5865F2"/></svg>`,
  telegram: `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="11" fill="#2AABEE"/>
    <path d="M5.4 11.6l11.6-4.5c.5-.2 1 .1.8.9l-2 9.3c-.1.6-.5.8-1.1.5l-3-2.2-1.4 1.4c-.2.2-.3.3-.6.3l.2-3.1 5.6-5.1c.2-.2 0-.3-.4-.1l-6.9 4.4-3-.9c-.6-.2-.7-.6.2-1z" fill="#fff"/></svg>`,
  whatsapp: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 1.8A10.1 10.1 0 0 0 3.2 17L1.8 22.2l5.4-1.4A10.1 10.1 0 1 0 12 1.8z" fill="#25D366"/>
    <path d="M8.8 6.9c-.2-.5-.5-.5-.7-.5h-.6c-.2 0-.6.1-.9.4s-1.1 1.1-1.1 2.7 1.2 3.1 1.3 3.3c.2.2 2.3 3.6 5.6 4.9 2.8 1.1 3.3.9 3.9.8.6-.1 1.9-.8 2.2-1.5.3-.7.3-1.4.2-1.5-.1-.1-.3-.2-.6-.4l-2.1-1c-.3-.1-.5-.2-.7.1l-1 1.2c-.2.2-.4.2-.6.1-.3-.2-1.3-.5-2.5-1.5-.9-.8-1.5-1.8-1.7-2.2-.2-.3 0-.5.1-.6l.5-.5c.1-.2.2-.3.3-.5.1-.2 0-.4 0-.5l-1-2.3z" fill="#fff"/></svg>`,
  reload: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
    <path d="M19.5 4.2v4.6h-4.6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  pin: `<svg viewBox="0 0 24 24" aria-hidden="true"><path class="pin-head" d="M9.2 3.5h5.6l-.9 5.6 3.1 3.1v1.9H7v-1.9l3.1-3.1z" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linejoin="round"/>
    <path d="M12 14.1v6.4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"/></svg>`,
  dots: `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="5.5" cy="12" r="1.8" fill="currentColor"/><circle cx="12" cy="12" r="1.8" fill="currentColor"/><circle cx="18.5" cy="12" r="1.8" fill="currentColor"/></svg>`,
  chevronRight: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 5l7 7-7 7" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  chevronLeft: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  gear: `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3.2" fill="none" stroke="currentColor" stroke-width="1.9"/>
    <path d="M19.4 13.3a7.7 7.7 0 0 0 0-2.6l2-1.6-2-3.4-2.4 1a7.4 7.4 0 0 0-2.2-1.3L14.4 3h-4l-.4 2.4a7.4 7.4 0 0 0-2.2 1.3l-2.4-1-2 3.4 2 1.6a7.7 7.7 0 0 0 0 2.6l-2 1.6 2 3.4 2.4-1a7.4 7.4 0 0 0 2.2 1.3l.4 2.4h4l.4-2.4a7.4 7.4 0 0 0 2.2-1.3l2.4 1 2-3.4z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/></svg>`,
  close: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6.5 6.5l11 11M17.5 6.5l-11 11" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/></svg>`,
  arrowUp: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 18.5V6.5M6.8 11.2 12 6l5.2 5.2" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  shield: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2.8 4.6 5.6v5.6c0 4.6 3 8.5 7.4 9.9 4.4-1.4 7.4-5.3 7.4-9.9V5.6z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>
    <path d="m8.8 12.1 2.3 2.3 4.2-4.5" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
  warning: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.5L2.8 19.5h18.4z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/>
    <path d="M12 9.5v4.6" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"/><circle cx="12" cy="16.9" r="1.1" fill="currentColor"/></svg>`,
  discordMono: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20.3 4.4A19.6 19.6 0 0 0 15.4 3l-.6 1.3a18.3 18.3 0 0 0-5.6 0L8.6 3a19.5 19.5 0 0 0-4.9 1.5C.6 9.1-.2 13.6.2 18a19.7 19.7 0 0 0 6 3l1.3-2.1a12.8 12.8 0 0 1-2-1l.5-.4a14 14 0 0 0 12 0l.5.4c-.6.4-1.3.7-2 1l1.3 2.1a19.6 19.6 0 0 0 6-3c.5-5.1-.8-9.6-3.5-13.6zM8 15.3c-1.2 0-2.1-1.1-2.1-2.4S6.8 10.5 8 10.5s2.2 1.1 2.1 2.4c0 1.3-.9 2.4-2.1 2.4zm8 0c-1.2 0-2.1-1.1-2.1-2.4s.9-2.4 2.1-2.4 2.2 1.1 2.1 2.4c0 1.3-.9 2.4-2.1 2.4z" fill="currentColor"/></svg>`,
  // settings categories (drawn white on a coloured tile)
  sliders: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7.5h8.5M17.5 7.5H20M4 16.5h2.5M11.5 16.5H20" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
    <circle cx="15" cy="7.5" r="2.5" fill="none" stroke="currentColor" stroke-width="2"/><circle cx="9" cy="16.5" r="2.5" fill="none" stroke="currentColor" stroke-width="2"/></svg>`,
  monitor: `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2.8" y="4" width="18.4" height="12.6" rx="2.2" fill="none" stroke="currentColor" stroke-width="2"/>
    <path d="M8.5 20.3h7M12 16.6v3.7" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  spotify: `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="11" fill="#1ed760"/><path d="M6.3 9.2c3.9-1.2 8.4-.9 11.6 1.1M6.9 12.6c3.2-.9 6.9-.6 9.6 1M7.6 15.8c2.5-.7 5.3-.5 7.5.8" fill="none" stroke="#0b0b0b" stroke-width="1.9" stroke-linecap="round"/></svg>`,
  volume: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9.2h3.3L12 5v14l-4.7-4.2H4z" fill="currentColor" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M15.4 9a4.3 4.3 0 0 1 0 6M17.9 6.4a8 8 0 0 1 0 11.2" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  volumeLow: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 9.2h3.3L13 5v14l-4.7-4.2H5z" fill="currentColor" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M16.4 9a4.3 4.3 0 0 1 0 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  volumeOff: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3.5 9.2h3.3L11.5 5v14l-4.7-4.2H3.5z" fill="currentColor" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M15.5 9.5l5 5M20.5 9.5l-5 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  phone: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7.2 3.5c.5 0 .9.3 1.1.8l1.3 3.2c.2.5 0 1-.4 1.3L7.6 10a12.5 12.5 0 0 0 6.4 6.4l1.2-1.6c.3-.4.8-.6 1.3-.4l3.2 1.3c.5.2.8.6.8 1.1v2.6c0 .9-.7 1.6-1.6 1.6C10.4 21 3 13.6 3 5.1c0-.9.7-1.6 1.6-1.6z" fill="currentColor"/></svg>`,
  pointerEdge: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20.5 3.5v17" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>
    <path d="M5.2 4.6l9.9 6.6-4.4.9 2.4 4.9-2.1 1-2.3-4.9-3.3 3z" fill="currentColor" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/></svg>`,
  theme: `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="8.4" fill="none" stroke="currentColor" stroke-width="2"/>
    <path d="M12 3.6a8.4 8.4 0 0 1 0 16.8z" fill="currentColor"/></svg>`,
  grid: `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3.6" y="3.6" width="7" height="7" rx="2" fill="none" stroke="currentColor" stroke-width="2"/>
    <rect x="13.4" y="3.6" width="7" height="7" rx="2" fill="none" stroke="currentColor" stroke-width="2"/><rect x="3.6" y="13.4" width="7" height="7" rx="2" fill="none" stroke="currentColor" stroke-width="2"/>
    <rect x="13.4" y="13.4" width="7" height="7" rx="2" fill="none" stroke="currentColor" stroke-width="2"/></svg>`,
  bell: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6.2 16.4v-5.1a5.8 5.8 0 0 1 11.6 0v5.1l1.7 2.1H4.5z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/>
    <path d="M9.9 20.6a2.3 2.3 0 0 0 4.2 0" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  download: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.8v10.4M7.6 9.9l4.4 4.4 4.4-4.4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
    <path d="M4.4 15.8v2.4a1.9 1.9 0 0 0 1.9 1.9h11.4a1.9 1.9 0 0 0 1.9-1.9v-2.4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  info: `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="8.6" fill="none" stroke="currentColor" stroke-width="2"/>
    <path d="M12 10.8v5.6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/><circle cx="12" cy="7.6" r="1.3" fill="currentColor"/></svg>`,
  search: `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10.6" cy="10.6" r="6.4" fill="none" stroke="currentColor" stroke-width="2"/>
    <path d="M15.4 15.4l4.6 4.6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/></svg>`,
  memory: `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="2.4" fill="none" stroke="currentColor" stroke-width="1.9"/>
    <rect x="9.4" y="9.4" width="5.2" height="5.2" rx="1" fill="currentColor"/>
    <path d="M9.5 3.5V6M14.5 3.5V6M9.5 18v2.5M14.5 18v2.5M3.5 9.5H6M3.5 14.5H6M18 9.5h2.5M18 14.5h2.5" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"/></svg>`,
  logo: `<svg viewBox="0 0 64 64" aria-hidden="true"><defs>
    <linearGradient id="logo-bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2c2e3e"/><stop offset="1" stop-color="#0d0e13"/></linearGradient>
    <linearGradient id="logo-b" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#feda75"/><stop offset=".3" stop-color="#fa7e1e"/>
    <stop offset=".55" stop-color="#d62976"/><stop offset=".8" stop-color="#962fbf"/><stop offset="1" stop-color="#0866ff"/></linearGradient></defs>
    <rect x="1" y="1" width="62" height="62" rx="14.5" fill="url(#logo-bg)"/>
    <ellipse cx="26" cy="30.4" rx="18.2" ry="16.4" fill="url(#logo-b)"/>
    <path d="M13.3 38.6l-5.3 12.8 15.6-6.1z" fill="url(#logo-b)"/>
    <circle cx="19.3" cy="30.4" r="2.3" fill="#fff"/><circle cx="26" cy="30.4" r="2.3" fill="#fff"/><circle cx="32.7" cy="30.4" r="2.3" fill="#fff"/>
    <rect x="48.3" y="16" width="7" height="32" rx="3.5" fill="#fff"/></svg>`,
};

// Each copy of an icon gets its own gradient ids. With shared ids, every copy paints with the first
// copy's gradients, and those vanish while the first copy sits in a hidden section.
let iconSeq = 0;
window.iconHTML = (name) => {
  const svg = window.ICONS[name] || '';
  if (!svg.includes('id="')) return svg;
  const n = ++iconSeq;
  return svg.replace(/id="([\w-]+)"/g, `id="$1-${n}"`).replace(/url\(#([\w-]+)\)/g, `url(#$1-${n})`);
};

// Fills [data-icon] elements that are still empty (again after translated text brings new ones).
window.fillIcons = (scope) => {
  for (const el of (scope || document).querySelectorAll('[data-icon]')) {
    if (!el.firstChild) el.innerHTML = window.iconHTML(el.dataset.icon);
  }
};
window.fillIcons();
