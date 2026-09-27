'use strict';

// Small always-on log (userData/chatdock.log) so problems seen by the user can be traced afterwards.
// Only discrete events are logged (open/close/clicks/focus), never page content.

const fs = require('node:fs');
const path = require('node:path');
const { app } = require('electron');

const MAX_BYTES = 1024 * 1024;
let file = null;
let echo = false;

function init({ console: toConsole = false } = {}) {
  echo = toConsole;
  try {
    const dir = app.getPath('userData');
    fs.mkdirSync(dir, { recursive: true });
    file = path.join(dir, 'chatdock.log');
    if (fs.existsSync(file) && fs.statSync(file).size > MAX_BYTES) {
      fs.renameSync(file, `${file}.old`);
    }
  } catch {
    file = null;
  }
}

function format(value) {
  if (typeof value === 'string') return value;
  if (value instanceof Error) return value.stack || value.message;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function log(...args) {
  const d = new Date();
  const stamp = `${d.toLocaleDateString('en-CA')} ${d.toTimeString().slice(0, 8)}.${String(d.getMilliseconds()).padStart(3, '0')}`;
  const line = `[${stamp}] ${args.map(format).join(' ')}`;
  if (echo) console.log(line);
  if (!file) return;
  try {
    fs.appendFileSync(file, `${line}\n`);
  } catch {
    // logging must never break the app
  }
}

module.exports = { init, log, path: () => file };
