'use strict';

// Small set of Win32 calls (via koffi) that Electron does not expose:
//  - remember / give back the foreground window (so closing the chat returns you to your game)
//  - check whether a mouse button is held (so dragging a scrollbar to the edge doesn't pop the tab)
//  - detect fullscreen apps / exclusive-fullscreen games
//  - tell when a game has taken the mouse (pointer hidden or held inside part of the screen)
// Every function degrades to a harmless default if koffi can't load.

let api = null;
let loadError = null;
let MONITORINFO_SIZE = 40;
let CURSORINFO_SIZE = 24;

try {
  const koffi = require('koffi');
  const user32 = koffi.load('user32.dll');
  const kernel32 = koffi.load('kernel32.dll');
  const shell32 = koffi.load('shell32.dll');
  const RECT = koffi.struct('RECT', { left: 'int32_t', top: 'int32_t', right: 'int32_t', bottom: 'int32_t' });
  const MONITORINFO = koffi.struct('MONITORINFO', { cbSize: 'uint32_t', rcMonitor: RECT, rcWork: RECT, dwFlags: 'uint32_t' });
  MONITORINFO_SIZE = koffi.sizeof(MONITORINFO);
  const POINT = koffi.struct('CD_POINT', { x: 'int32_t', y: 'int32_t' });
  const CURSORINFO = koffi.struct('CURSORINFO', { cbSize: 'uint32_t', flags: 'uint32_t', hCursor: 'intptr_t', ptScreenPos: POINT });
  CURSORINFO_SIZE = koffi.sizeof(CURSORINFO);
  api = {
    GetWindowRect: user32.func('int __stdcall GetWindowRect(intptr_t hWnd, _Out_ RECT *rect)'),
    MonitorFromWindow: user32.func('intptr_t __stdcall MonitorFromWindow(intptr_t hWnd, uint32_t flags)'),
    GetMonitorInfoW: user32.func('int __stdcall GetMonitorInfoW(intptr_t hMonitor, _Inout_ MONITORINFO *info)'),
    IsWindowVisible: user32.func('int __stdcall IsWindowVisible(intptr_t hWnd)'),
    GetForegroundWindow: user32.func('intptr_t __stdcall GetForegroundWindow()'),
    SetForegroundWindow: user32.func('int __stdcall SetForegroundWindow(intptr_t hWnd)'),
    BringWindowToTop: user32.func('int __stdcall BringWindowToTop(intptr_t hWnd)'),
    IsWindow: user32.func('int __stdcall IsWindow(intptr_t hWnd)'),
    IsIconic: user32.func('int __stdcall IsIconic(intptr_t hWnd)'),
    ShowWindow: user32.func('int __stdcall ShowWindow(intptr_t hWnd, int nCmdShow)'),
    GetAncestor: user32.func('intptr_t __stdcall GetAncestor(intptr_t hWnd, uint32_t gaFlags)'),
    GetWindowThreadProcessId: user32.func('uint32_t __stdcall GetWindowThreadProcessId(intptr_t hWnd, void *pid)'),
    AttachThreadInput: user32.func('int __stdcall AttachThreadInput(uint32_t idAttach, uint32_t idAttachTo, int fAttach)'),
    GetAsyncKeyState: user32.func('int16_t __stdcall GetAsyncKeyState(int vKey)'),
    GetClassNameW: user32.func('int __stdcall GetClassNameW(intptr_t hWnd, void *buf, int maxCount)'),
    GetCursorInfo: user32.func('int __stdcall GetCursorInfo(_Inout_ CURSORINFO *info)'),
    GetClipCursor: user32.func('int __stdcall GetClipCursor(_Out_ RECT *rect)'),
    GetSystemMetrics: user32.func('int __stdcall GetSystemMetrics(int index)'),
    GetCurrentThreadId: kernel32.func('uint32_t __stdcall GetCurrentThreadId()'),
    SHQueryUserNotificationState: shell32.func('int32_t __stdcall SHQueryUserNotificationState(_Out_ int32_t *state)'),
  };
} catch (err) {
  api = null;
  loadError = err;
}

const GA_ROOTOWNER = 3;
const SW_RESTORE = 9;
const MONITOR_DEFAULTTONULL = 0;
// Shell windows that cover the whole screen but are not "an app running fullscreen"
const SHELL_CLASSES = new Set(['Progman', 'WorkerW', 'Shell_TrayWnd', 'Shell_SecondaryTrayWnd']);
const VK_LBUTTON = 0x01;
const VK_RBUTTON = 0x02;
const VK_MBUTTON = 0x04;
const CURSOR_SHOWING = 0x1;
const SM_XVIRTUALSCREEN = 76;
const SM_YVIRTUALSCREEN = 77;
const SM_CXVIRTUALSCREEN = 78;
const SM_CYVIRTUALSCREEN = 79;

// QUERY_USER_NOTIFICATION_STATE
const QUNS_RUNNING_D3D_FULL_SCREEN = 3; // exclusive-fullscreen Direct3D game
const QUNS_PRESENTATION_MODE = 4;

function available() {
  return api !== null;
}

function hwndOf(win) {
  if (!win || win.isDestroyed()) return 0;
  const buf = win.getNativeWindowHandle();
  return buf.length >= 8 ? Number(buf.readBigUInt64LE(0)) : buf.readUInt32LE(0);
}

function foregroundWindow() {
  return api ? api.GetForegroundWindow() : 0;
}

function rootOwner(hwnd) {
  return api && hwnd ? api.GetAncestor(hwnd, GA_ROOTOWNER) : 0;
}

function className(hwnd) {
  if (!api || !hwnd) return '';
  const buf = Buffer.alloc(512);
  const len = api.GetClassNameW(hwnd, buf, 256);
  return len > 0 ? buf.toString('utf16le', 0, len * 2) : '';
}

function isWindow(hwnd) {
  return !!(api && hwnd && api.IsWindow(hwnd));
}

// SetForegroundWindow, falling back to the AttachThreadInput trick when Windows' focus-stealing
// protection refuses the plain call. No synthetic key presses (those could leak into a game).
function forceForeground(hwnd) {
  if (!api || !hwnd) return false;
  if (api.SetForegroundWindow(hwnd)) return true;
  const fg = api.GetForegroundWindow();
  const fgThread = fg ? api.GetWindowThreadProcessId(fg, null) : 0;
  const me = api.GetCurrentThreadId();
  const attached = fgThread && fgThread !== me ? !!api.AttachThreadInput(me, fgThread, 1) : false;
  api.BringWindowToTop(hwnd);
  const ok = !!api.SetForegroundWindow(hwnd);
  if (attached) api.AttachThreadInput(me, fgThread, 0);
  return ok;
}

// Give focus back to a window we took it from (un-minimizing exclusive-fullscreen games first).
function restoreForeground(hwnd) {
  if (!isWindow(hwnd)) return false;
  if (api.IsIconic(hwnd)) api.ShowWindow(hwnd, SW_RESTORE);
  return forceForeground(hwnd);
}

function mouseButtonDown() {
  if (!api) return false;
  return ((api.GetAsyncKeyState(VK_LBUTTON) | api.GetAsyncKeyState(VK_RBUTTON) | api.GetAsyncKeyState(VK_MBUTTON)) & 0x8000) !== 0;
}

function notificationState() {
  if (!api) return 0;
  const out = [0];
  return api.SHQueryUserNotificationState(out) === 0 ? out[0] : 0;
}

// True when the foreground window covers its whole monitor (borderless / fullscreen game, F11 video, ...).
// SHQueryUserNotificationState alone is not enough: it can report "busy" while the desktop is focused.
function isFullscreenAppActive() {
  if (!api) return false;
  const s = notificationState();
  if (s === QUNS_RUNNING_D3D_FULL_SCREEN || s === QUNS_PRESENTATION_MODE) return true;
  const hwnd = api.GetForegroundWindow();
  if (!hwnd || !api.IsWindowVisible(hwnd) || SHELL_CLASSES.has(className(hwnd))) return false;
  const rect = {};
  if (!api.GetWindowRect(hwnd, rect)) return false;
  const mon = api.MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
  if (!mon) return false;
  const info = { cbSize: MONITORINFO_SIZE };
  if (!api.GetMonitorInfoW(mon, info)) return false;
  const m = info.rcMonitor;
  return rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom;
}

function isExclusiveFullscreen() {
  return notificationState() === QUNS_RUNNING_D3D_FULL_SCREEN;
}

// The mouse pointer is hidden: a game in mouse-look mode (FPS aiming), a fullscreen video, ...
function cursorHidden() {
  if (!api) return false;
  const info = { cbSize: CURSORINFO_SIZE };
  if (!api.GetCursorInfo(info)) return false;
  return (info.flags & CURSOR_SHOWING) === 0 || !info.hCursor; // hidden, or no pointer image at all
}

// Some program keeps the pointer inside part of the screen (ClipCursor), as games do while they
// own the mouse. Unclipped, the clip rectangle is the whole virtual screen.
function cursorConfined() {
  if (!api) return false;
  const r = {};
  if (!api.GetClipCursor(r)) return false;
  const x = api.GetSystemMetrics(SM_XVIRTUALSCREEN);
  const y = api.GetSystemMetrics(SM_YVIRTUALSCREEN);
  const w = api.GetSystemMetrics(SM_CXVIRTUALSCREEN);
  const h = api.GetSystemMetrics(SM_CYVIRTUALSCREEN);
  const slack = 2;
  return r.left > x + slack || r.top > y + slack || r.right < x + w - slack || r.bottom < y + h - slack;
}

// A game (or anything else) has taken the mouse: the pointer reaching the screen edge is then
// just aiming or camera movement, not the user reaching for ChatDock.
function mouseCaptured() {
  return cursorHidden() || cursorConfined();
}

module.exports = {
  available,
  loadError: () => loadError,
  hwndOf,
  foregroundWindow,
  rootOwner,
  className,
  isWindow,
  forceForeground,
  restoreForeground,
  mouseButtonDown,
  notificationState,
  isFullscreenAppActive,
  isExclusiveFullscreen,
  cursorHidden,
  cursorConfined,
  mouseCaptured,
};
