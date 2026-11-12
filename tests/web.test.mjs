import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

// Exercise the actual dashboard handlers without touching USB or adding a browser dependency.
function dashboard() {
  const timers = new Map();
  let nextTimer = 0;
  class Button extends EventTarget {
    disabled = false;
    isConnected = true;
    dataset = {};
    style = { setProperty() {} };
    classes = new Set();
    classList = {
      add: (...names) => names.forEach((n) => this.classes.add(n)),
      remove: (...names) => names.forEach((n) => this.classes.delete(n)),
      contains: (name) => this.classes.has(name),
    };
  }
  const document = Object.assign(new EventTarget(), { hidden: false, querySelector: () => null });
  const window = new EventTarget();
  let events;
  const ctx = vm.createContext({
    document, window, Event, AbortController, performance, console, TextEncoder,
    URL, CSS: { escape: (s) => s },
    setTimeout: (fn) => { timers.set(++nextTimer, fn); return nextTimer; },
    clearTimeout: (id) => timers.delete(id),
    EventSource: class extends EventTarget { constructor() { super(); events = this; } close() {} },
    requestAnimationFrame: () => {},
  });
  vm.runInContext(readFileSync(new URL('../web/app.js', import.meta.url), 'utf8'), ctx);
  vm.runInContext('render = () => {}; notify = () => {};', ctx);
  const run = (s) => vm.runInContext(s, ctx);
  return { run, ctx, Button, document, window, timers, get events() { return events; }, async fire() {
    const callbacks = [...timers.values()]; timers.clear();
    for (const fn of callbacks) await fn();
  } };
}
function press(button) {
  const e = new Event('keydown', { cancelable: true });
  Object.assign(e, { key: 'Enter', repeat: false });
  button.dispatchEvent(e);
}

test('a hold fires once, while short presses and focus/visibility loss cancel it', async () => {
  for (const stop of [null, 'keyup', 'blur', 'window', 'hidden', 'pointercancel']) {
    const d = dashboard();
    let fired = 0;
    const btn = new d.Button();
    d.ctx.btn = btn;
    d.ctx.onFire = () => { fired++; };
    d.run('holdable(btn, { onFire, hint: () => {} })');
    press(btn);
    if (stop === 'window') d.window.dispatchEvent(new Event('blur'));
    else if (stop === 'hidden') { d.document.hidden = true; d.document.dispatchEvent(new Event('visibilitychange')); }
    else if (stop) {
      const e = new Event(stop); Object.assign(e, { key: 'Enter' }); btn.dispatchEvent(e);
    }
    await d.fire();
    assert.equal(fired, stop ? 0 : 1, stop || 'held');
  }
});

test('disabled, removed, and changed-state controls cannot complete an existing hold', async () => {
  for (const change of ['btn.disabled = true', 'btn.isConnected = false', 'btn.dataset.caught = "true"']) {
    const d = dashboard(); let fired = 0;
    d.ctx.btn = new d.Button(); d.ctx.onFire = () => { fired++; };
    d.run('holdable(btn, { onFire, hint: () => {} })');
    press(d.ctx.btn); d.run(change); await d.fire(); assert.equal(fired, 0, change);
  }
});

test('a UI operation from another page clears the pending state when it finishes', () => {
  const d = dashboard();
  d.run('connect()');
  const send = (kind, data) => {
    const e = new Event(kind); e.data = JSON.stringify({ at: new Date().toISOString(), data }); d.events.dispatchEvent(e);
  };
  send('operation.started', { uid: 'a', action: 'on' });
  assert.equal(d.run('state.pending.get("a")'), 'on');
  send('operation.finished', { uid: 'a', action: 'on', source: 'ui', ok: true });
  assert.equal(d.run('state.pending.has("a")'), false);
});

test('stale, locked, busy, and unknown devices are blocked before sending an action', async () => {
  const d = dashboard();
  d.run('state.live = true; globalThis.device = { uid: "a", online: true, status: { busy: false } };');
  for (const setup of ['state.live = false', 'device.locked = true', 'device.status.busy = true', 'state.unknown.set("a", "unknown")']) {
    d.run('state.live = true; device.locked = false; device.status.busy = false; state.unknown.clear();');
    d.run(setup);
    assert.equal(d.run('canAct(device)'), false, setup);
    let calls = 0; d.ctx.fetch = () => { calls++; };
    await d.run('power(device, "on")'); assert.equal(calls, 0, setup);
  }
});

test('a lost write response becomes unknown, and only a successful fresh read clears it', async () => {
  const d = dashboard();
  d.run('state.live = true; globalThis.device = { uid: "a", online: true, status: {} }; state.devices.set("a", device); api = async () => { throw new TypeError("network") };');
  await d.run('power(device, "on")');
  assert.equal(d.run('state.unknown.has("a")'), true);
  assert.equal(d.run('state.pending.has("a")'), false);
  await d.run('checkState("a")');
  assert.equal(d.run('state.unknown.has("a")'), true);
  d.run('api = async () => ({ status: { output_enabled: true, busy: false } });');
  await d.run('checkState("a")');
  assert.equal(d.run('state.unknown.has("a")'), false);
  assert.equal(d.run('state.devices.get("a").status.output_enabled'), true);
});

test('rename uses the reply immediately and cannot send a name when output is on', async () => {
  const d = dashboard();
  d.run('state.live = true; globalThis.device = { uid: "a", online: true, status: { state_code: 0 } }; state.devices.set("a", device); api = async () => ({ nickname: "工作台" });');
  await d.run('rename(device, "工作台")');
  assert.equal(d.run('device.nickname'), '工作台');
  d.run('device.status.output_enabled = true; api = async () => { throw new Error("must not send") };');
  await d.run('rename(device, "other")');
  assert.equal(d.run('device.nickname'), '工作台');
});
