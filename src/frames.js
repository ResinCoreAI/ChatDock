'use strict';

// Frame clock for windows that the main process moves (the chat panel sliding in, the edge tab
// following the cursor). A moving window only looks smooth when it moves once per screen refresh,
// and Windows' normal timers fire just every 15.6 ms (64 times a second): on a 144-300 Hz screen a
// window moved on a timer visibly jumps. Here each frame is timed by the screen itself: the wait
// for the next vertical blank (D3DKMTWaitForVerticalBlankEvent, the same wait Chromium uses for
// its own vsync) runs on a worker thread through koffi's async calls, and the frame callbacks run
// on the main thread right after it. Without it: 1 ms timer resolution while something animates,
// and timeouts at the refresh interval. Nothing runs while nothing animates.

const { screen } = require('electron');

let api = null;
try {
  const koffi = require('koffi');
  const user32 = koffi.load('user32.dll');
  const gdi32 = koffi.load('gdi32.dll');
  const winmm = koffi.load('winmm.dll');
  const LUID = koffi.struct('CD_LUID', { LowPart: 'uint32_t', HighPart: 'int32_t' });
  koffi.struct('CD_OPENADAPTERFROMHDC', { hDc: 'intptr_t', hAdapter: 'uint32_t', AdapterLuid: LUID, VidPnSourceId: 'uint32_t' });
  koffi.struct('CD_WAITFORVERTICALBLANKEVENT', { hAdapter: 'uint32_t', hDevice: 'uint32_t', VidPnSourceId: 'uint32_t' });
  koffi.struct('CD_CLOSEADAPTER', { hAdapter: 'uint32_t' });
  koffi.struct('CD_PT', { x: 'int32_t', y: 'int32_t' });
  const MRECT = koffi.struct('CD_MRECT', { left: 'int32_t', top: 'int32_t', right: 'int32_t', bottom: 'int32_t' });
  koffi.struct('CD_MONITORINFOEXW', {
    cbSize: 'uint32_t', rcMonitor: MRECT, rcWork: MRECT, dwFlags: 'uint32_t', szDevice: koffi.array('char16_t', 32, 'String'),
  });
  api = {
    infoSize: koffi.sizeof('CD_MONITORINFOEXW'),
    MonitorFromPoint: user32.func('intptr_t __stdcall MonitorFromPoint(CD_PT pt, uint32_t flags)'),
    GetMonitorInfoW: user32.func('int __stdcall GetMonitorInfoW(intptr_t hMonitor, _Inout_ CD_MONITORINFOEXW *info)'),
    CreateDCW: gdi32.func('intptr_t __stdcall CreateDCW(const char16_t *driver, const char16_t *device, const char16_t *port, void *devmode)'),
    DeleteDC: gdi32.func('int __stdcall DeleteDC(intptr_t hdc)'),
    OpenAdapter: gdi32.func('int32_t __stdcall D3DKMTOpenAdapterFromHdc(_Inout_ CD_OPENADAPTERFROMHDC *p)'),
    WaitVBlank: gdi32.func('int32_t __stdcall D3DKMTWaitForVerticalBlankEvent(const CD_WAITFORVERTICALBLANKEVENT *p)'),
    CloseAdapter: gdi32.func('int32_t __stdcall D3DKMTCloseAdapter(const CD_CLOSEADAPTER *p)'),
    timeBeginPeriod: winmm.func('uint32_t __stdcall timeBeginPeriod(uint32_t ms)'),
    timeEndPeriod: winmm.func('uint32_t __stdcall timeEndPeriod(uint32_t ms)'),
  };
} catch {
  api = null;
}

const MONITOR_DEFAULTTONEAREST = 2;

const queue = new Set(); // callbacks waiting for the next frame
let waiting = false; // a frame has been asked for
let inFlight = 0; // adapter of the vertical-blank wait running on the worker thread (0 = none)
let vblank = null; // { hAdapter, hDevice, VidPnSourceId } of the screen the dock is on
let vblankDevice = '';
let staleAdapter = 0; // replaced while a wait was still using it; closed when that wait returns
let failures = 0;
let watchdog = null;
let fineTimers = false;
let hz = 60;
let onError = () => {};

const frameMs = () => 1000 / hz;

function closeAdapter(hAdapter) {
  if (!hAdapter) return;
  try {
    api.CloseAdapter({ hAdapter });
  } catch {
    // nothing more to do
  }
}

// The dock's screen: its refresh rate, and the adapter whose vertical blank times the frames.
function setDisplay(display) {
  hz = display && display.displayFrequency > 1 ? display.displayFrequency : 60;
  if (!api || !display) return;
  try {
    const b = display.bounds;
    const center = screen.dipToScreenPoint({ x: Math.round(b.x + b.width / 2), y: Math.round(b.y + b.height / 2) });
    const hMonitor = api.MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
    const info = { cbSize: api.infoSize };
    if (!hMonitor || !api.GetMonitorInfoW(hMonitor, info)) return;
    const device = info.szDevice;
    if (device === vblankDevice && vblank) return;
    const old = vblank ? vblank.hAdapter : 0;
    vblank = null;
    vblankDevice = device;
    if (old && old === inFlight) staleAdapter = old;
    else closeAdapter(old);
    const hdc = api.CreateDCW(device, device, null, null);
    if (!hdc) return;
    const open = { hDc: hdc, hAdapter: 0, AdapterLuid: { LowPart: 0, HighPart: 0 }, VidPnSourceId: 0 };
    const status = api.OpenAdapter(open);
    api.DeleteDC(hdc);
    if (status === 0) {
      vblank = { hAdapter: open.hAdapter, hDevice: 0, VidPnSourceId: open.VidPnSourceId };
      failures = 0;
    }
  } catch (err) {
    vblank = null;
    onError(err);
  }
}

function fine(on) {
  if (!api || on === fineTimers) return;
  fineTimers = on;
  try {
    if (on) api.timeBeginPeriod(1);
    else api.timeEndPeriod(1);
  } catch {
    // timers stay at their normal resolution
  }
}

function schedule() {
  if (waiting || !queue.size) return;
  waiting = true;
  clearTimeout(watchdog);
  if (vblank && !inFlight) {
    const used = vblank;
    inFlight = used.hAdapter;
    api.WaitVBlank.async(used, (err, status) => {
      inFlight = 0;
      if (staleAdapter) {
        closeAdapter(staleAdapter);
        staleAdapter = 0;
      }
      if (err || status !== 0) {
        failures += 1;
        if (failures >= 3 && vblank === used) { // this screen has no vertical blank to wait for: use timers
          vblank = null;
          closeAdapter(used.hAdapter);
          onError(err || new Error(`vertical blank wait failed: ${status}`));
        }
      } else {
        failures = 0;
      }
      tick();
    });
    // A screen that stopped refreshing (turned off) must not freeze an animation half way.
    watchdog = setTimeout(tick, Math.max(40, 3 * frameMs()));
  } else {
    fine(true);
    watchdog = setTimeout(tick, Math.max(1, Math.floor(frameMs()) - 1));
  }
}

function tick() {
  clearTimeout(watchdog);
  watchdog = null;
  if (!waiting) return; // this frame already ran (the vertical blank or the watchdog came first)
  waiting = false;
  const now = performance.now();
  const due = [...queue];
  queue.clear();
  for (const cb of due) {
    try {
      cb(now);
    } catch (err) {
      onError(err);
    }
  }
  if (queue.size) schedule();
  else fine(false);
}

// Like requestAnimationFrame: cb(now) runs once, at the next screen refresh.
function request(cb) {
  queue.add(cb);
  schedule();
}

function cancel(cb) {
  queue.delete(cb);
}

function init(opts = {}) {
  if (opts.onError) onError = opts.onError;
}

function info() {
  return { hz, mode: vblank ? 'vblank' : 'timer', device: vblankDevice };
}

module.exports = { init, setDisplay, request, cancel, info };
