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
  const document = Object.assign(new EventTarget(), { hidden: false, querySelector: () => null, documentElement: { lang: '', dataset: {} } });
  const window = new EventTarget();
  let events;
  const ctx = vm.createContext({
    document, window, Event, AbortController, performance, console, TextEncoder,
    navigator: { language: 'en' },
    URL, CSS: { escape: (s) => s },
    setTimeout: (fn) => { timers.set(++nextTimer, fn); return nextTimer; },
    clearTimeout: (id) => timers.delete(id),
    EventSource: class extends EventTarget { constructor() { super(); events = this; } close() {} },
    requestAnimationFrame: () => {},
  });
  vm.runInContext(readFileSync(new URL('../web/i18n.js', import.meta.url), 'utf8'), ctx);
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
  d.run('api = async (method, path) => { globalThis.readRequest = [method, path]; return { uid: "a", output_enabled: true, busy: false }; };');
  await d.run('checkState("a")');
  assert.equal(d.run('state.unknown.has("a")'), false);
  assert.equal(d.run('state.devices.get("a").status.output_enabled'), true);
  assert.equal(d.run('readRequest.join(" ")'), 'GET /v1/devices/a');
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

test('locales fall back safely, reorder template values, and preserve device names', () => {
  const d = dashboard();
  assert.equal(d.run('VolooI18n.matchLanguage("zh-TW")'), 'zh-Hant');
  assert.equal(d.run('VolooI18n.matchLanguage("zh-HK")'), 'zh-Hant');
  assert.equal(d.run('VolooI18n.matchLanguage("zh-CN")'), 'zh-Hans');
  assert.equal(d.run('VolooI18n.matchLanguage("fr-FR")'), 'en');
  d.run('VolooI18n.setLanguage("zh-Hans")');
  assert.equal(d.run('tr`Couldn\'t ${"开启供电"} ${"我的板子"}`'), '无法为 我的板子 开启供电');
  assert.equal(d.run('FAULT.DEV_DISCHARGE_TIMEOUT'), '泄放超时');
  assert.equal(d.run('nameOf({ uid: "ab-cd", nickname: "Off" })'), 'Off');
  d.run('VolooI18n.setLanguage("zh-Hant")');
  assert.equal(d.run('FAULT.DEV_DISCHARGE_TIMEOUT'), '洩放逾時');
  assert.equal(d.run('tr("Unknown diagnostic detail")'), 'Unknown diagnostic detail');
});

test('every shipped locale has the same interpolation fields as its source', () => {
  const d = dashboard();
  const messages = d.run('VolooI18n.messages');
  const fields = (s) => [...s.matchAll(/\{\d+\}/g)].map((m) => m[0]).sort();
  for (const [source, locales] of Object.entries(messages)) {
    for (const text of locales) {
      assert.ok(text.length > 0, source);
      assert.deepEqual(fields(text), fields(source), source);
    }
  }
});

test('firmware maintenance from another client blocks power controls until it finishes', () => {
  const d = dashboard();
  d.run('state.live = true; state.devices.set("a", { uid: "a", online: true, status: { busy: false } }); connect();');
  const send = (stage) => { const e = new Event('firmware.progress'); e.data = JSON.stringify({ data: { uid: 'a', firmware: { stage, percent: 50 } } }); d.events.dispatchEvent(e); };
  send('uploading');
  assert.equal(d.run('canAct(state.devices.get("a"))'), false);
  send('reconnecting');
  assert.equal(d.run('canAct(state.devices.get("a"))'), false);
  send('completed');
  assert.equal(d.run('canAct(state.devices.get("a"))'), true);
});

test('firmware controls require valid idle OFF and no residual voltage above 1 V', () => {
  const d = dashboard();
  d.run('state.live = true; globalThis.fwDevice = { uid: "a", online: true, status: { state_code: 0, power_state: "OFF", output_enabled: false, discharge_enabled: false, busy: false, vin_valid: true, vout_valid: true, vout_mv: 1000 } };');
  assert.equal(d.run('firmwareReady(fwDevice)'), true);
  for (const code of ['fwDevice.locked = true', 'fwDevice.status.vin_valid = false', 'fwDevice.status.vout_valid = false', 'fwDevice.status.vout_mv = 1001', 'fwDevice.status.output_enabled = true']) {
    d.run('fwDevice.locked = false; Object.assign(fwDevice.status, {vin_valid: true, vout_valid: true, vout_mv: 1000, output_enabled: false});');
    d.run(code); assert.equal(d.run('firmwareReady(fwDevice)'), false, code);
  }
});

test('changing the lock while a firmware file is read cannot send an update', async () => {
  const d = dashboard();
  let resolve;
  d.ctx.file = { size: 10, arrayBuffer: () => new Promise((r) => { resolve = r; }) };
  d.run('state.live = true; globalThis.fwDevice = { uid: "a", online: true, status: { state_code: 0, power_state: "OFF", output_enabled: false, discharge_enabled: false, busy: false, vin_valid: true, vout_valid: true, vout_mv: 0 } }; state.devices.set("a", fwDevice); state.firmwareFiles.set("a", file); globalThis.calls = 0; act = async () => { calls++; };');
  const pending = d.run('updateFirmware(fwDevice)');
  d.run('fwDevice.locked = true'); resolve(new ArrayBuffer(10)); await pending;
  assert.equal(d.run('calls'), 0);
});

test('structured firmware journal errors retain the recovery outcome and readable message', () => {
  const d = dashboard();
  d.ctx.record = { uid: 'a', action: 'firmware_update', outcome: 'failed', error: { code: 'device_failed', message: 'Previous firmware restored', details: { update: { outcome: 'rolled_back' } } } };
  assert.equal(d.run('normalizeOp(record).error.message'), 'Previous firmware restored');
  assert.equal(d.run('normalizeOp(record).error.details.update.outcome'), 'rolled_back');
  assert.equal(d.run('normalizeOp({uid:"a",error:"old string error",outcome:"unknown"}).error.message'), 'old string error');
});


test('online firmware requires protected initialization and sends no local file bytes', async () => {
  const d = dashboard();
  d.run('state.live = true; globalThis.fwDevice = { uid: "a", online: true, identity: { capabilities: 0x1000 }, status: { state_code: 0, power_state: "OFF", output_enabled: false, discharge_enabled: false, busy: false, vin_valid: true, vout_valid: true, vout_mv: 0 } }; state.devices.set("a", fwDevice); globalThis.calls = []; act = async (...args) => { calls.push(args); }; refreshFirmware = () => {};');
  for (const flags of [null, 0, 2, 6, 12]) {
    d.ctx.flags = flags;
    d.run('state.firmwareBoot.set("a", flags == null ? null : {encrypted: true, protection_flags: flags})');
    await d.run('updateFirmware(fwDevice)');
    assert.equal(d.run('calls.length'), 0);
  }
  d.run('state.firmwareBoot.set("a", {encrypted: true, protection_flags: 14})');
  await d.run('updateFirmware(fwDevice)');
  assert.equal(d.run('calls.length'), 1);
  assert.equal(d.run('JSON.stringify(calls[0].slice(1))'), '["firmware_update","POST","/v1/devices/a/firmware",{}]');
  d.run('VolooI18n.setLanguage("zh-Hans")');
  assert.equal(d.run('firmwareStage("downloading")'), '正在下载加密固件');
});
