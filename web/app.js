// Voloo — LibreZoo Instrument. No framework, no build step.
// Device data is only ever inserted as text; innerHTML is used for static icons only.
// Power changes are hold-to-confirm: the switch blade closes (or opens) while you hold.
// Results are what the device confirmed; an unknown outcome is highlighted, never retried.
'use strict';

const HOLD_MS = 600;
const HISTORY_MS = 60_000;

// ---------- static icons ----------
// The device control: a knife switch. Source on the left, load on the right, a blade
// hinged on the source whose tip is the brand dot. Raised and dark = open; lying on the
// load terminal with the circuit lit = connected. The brand mark itself stays in the header.
const GLYPH = '<svg class="glyph" viewBox="0 0 48 48" aria-hidden="true"><path class="wire" d="M3 30H7.5M40.5 30H45"/><circle class="term" cx="11.5" cy="30" r="3.6"/><circle class="term" cx="36.5" cy="30" r="3.6"/><g class="blade"><line class="lever" x1="11.5" y1="30" x2="35" y2="30"/><circle class="tip" cx="36.5" cy="30" r="4.6"/></g></svg>';
const ICON = {
  lock: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="5" y="11" width="14" height="10" rx="2.5"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>',
  warn: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4 2.8 19.5h18.4z"/><path d="M12 10v4.2M12 17.2v.1"/></svg>',
  info: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.1"/></svg>',
  check: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12.5 4.5 4.5L19 7.5"/></svg>',
  x: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg>',
  bolt: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M13 2.5 5 13.5h6l-1 8 8-11h-6z"/></svg>',
  off: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.5v7.5"/><path d="M6.6 7.2a7.6 7.6 0 1 0 10.8 0"/></svg>',
  cycle: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19 8a7.5 7.5 0 0 0-13.4-1.6M5 4v4h4"/><path d="M5 16a7.5 7.5 0 0 0 13.4 1.6M19 20v-4h-4"/></svg>',
  drain: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3c3 4 6 7 6 10.5a6 6 0 0 1-12 0C6 10 9 7 12 3z"/></svg>',
  pencil: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16z"/></svg>',
  spin: '<svg class="spin" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a9 9 0 1 1-9 9"/></svg>',
};

const FAULT = {
  DEV_DISCHARGE_TIMEOUT: 'Discharge timed out',
  DEV_MEASUREMENT_INVALID: 'Discharge stopped: output unreadable',
  DEV_INTERNAL_ERROR: 'Internal error',
  DEV_VIN_INVALID: 'Input voltage unreadable',
  DEV_VOUT_INVALID: 'Output voltage unreadable',
  DEV_RESET_DETECTED: 'Restarted',
  DEV_EXCEPTION: 'Request rejected',
  DEV_BUSY: 'Busy',
  LINK_RESPONSE_TIMEOUT: 'Slow reply',
  LINK_CRC_ERROR: 'Noisy connection',
  LINK_LATE_FRAME: 'Late reply discarded',
  LINK_PORT_BUSY: 'Port in use by another app',
  LINK_UNPLUGGED: 'Disconnected',
  LINK_NO_RESPONSE: 'Not responding',
  LINK_IDENTITY_MISMATCH: 'Identity mismatch',
  LINK_IO: 'Connection error',
  HOST_LOG_WRITE_FAILED: "Can't write logs",
};
const SOURCE = { ui: 'Web', cli: 'Terminal', mcp: 'AI agent', http: 'API', daemon: 'Voloo' };
const DONE = { on: 'switched on', off: 'switched off', discharge: 'discharged', cycle: 'power-cycled', nickname: 'renamed' };
const FAIL = { on: 'switch on', off: 'switch off', discharge: 'discharge', cycle: 'power-cycle', nickname: 'rename' };
const ACTION_STYLE = {
  on: ['var(--volt)', ICON.bolt], off: ['var(--text-2)', ICON.off], discharge: ['var(--apricot)', ICON.drain],
  cycle: ['var(--sky)', ICON.cycle], nickname: ['var(--lilac)', ICON.pencil],
};

// ---------- state ----------
const state = {
  devices: new Map(), history: new Map(), pending: new Map(), unknown: new Map(),
  ops: [], faults: [], view: 'devices', feed: 'all', filter: '', live: false,
  sheet: null, cycleMs: 1000, cycleDischarge: true,
};
let signedOut = false;

// ---------- helpers ----------
const $ = (id) => document.getElementById(id);
function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    if (k === 'class') el.className = v;
    else if (k === 'html') el.innerHTML = v; // static templates only
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c != null && c !== false) el.append(c instanceof Node ? c : String(c));
  return el;
}
const svgEl = (tag, attrs) => {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs || {})) el.setAttribute(k, v);
  return el;
};
const micro = (text) => h('span', { class: 'micro' }, text);
const v2 = (mv) => (mv / 1000).toFixed(2);
const v3 = (mv) => (mv / 1000).toFixed(3);
const shortUid = (uid) => uid.replace(/-/g, '').slice(-4);
const nameOf = (d) => (d && d.nickname) || `Voloo ${shortUid(d.uid)}`;
const enc = encodeURIComponent;
function ago(iso) {
  if (!iso) return '';
  const s = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 1000));
  if (s < 5) return 'now';
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  return new Date(iso).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}
const clock = (iso) => new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
function dayLabel(iso) {
  const d = new Date(iso), today = new Date(), y = new Date();
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return 'Today';
  if (d.toDateString() === y.toDateString()) return 'Yesterday';
  return d.toLocaleDateString([], { weekday: 'long', month: 'short', day: 'numeric' });
}
// Replace children only when the content changed, so buttons inside survive live updates.
function setKeyed(el, key, build) {
  if (el._key === key) return;
  el._key = key;
  el.replaceChildren(...build());
}

async function api(method, path, body) {
  const res = await fetch(path, {
    method, credentials: 'same-origin',
    headers: { 'X-Voloo-Source': 'ui', ...(body ? { 'Content-Type': 'application/json' } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (res.status === 401) {
    state.live = false;
    signedOut = true;
    render();
    throw { code: 'unauthorized', message: 'This page is signed out. Run `voloo ui` again.' };
  }
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw data.error || { code: 'error', message: `HTTP ${res.status}` };
  return data;
}

// ---------- notifications ----------
function notify(kind, title, sub) {
  const icon = kind === 'ok' ? ICON.check : kind === 'warn' ? ICON.warn : ICON.x;
  const t = h('div', { class: `toast ${kind}${sub ? ' long' : ''}` },
    h('span', { class: 'tdot', html: icon }),
    h('span', {}, title, sub ? h('span', { class: 'tsub' }, sub) : null));
  const island = $('island');
  island.prepend(t);
  while (island.children.length > 3) island.lastChild.remove();
  setTimeout(() => { t.classList.add('leaving'); setTimeout(() => t.remove(), 300); }, kind === 'ok' ? 3200 : 9000);
}

// ---------- device model ----------
const activeFaults = (d) => (d.faults || []).filter((f) => f.active);
const recentEvents = (d) => (d.faults || []).filter((f) => !f.active && !f.condition && Date.now() - Date.parse(f.last_seen) < 120_000);
// Colour = what is happening; the dot's position = whether the output is connected.
function visual(d) {
  const s = d.status || {};
  const pending = state.pending.get(d.uid);
  if (!d.online) return 'offline';
  if (pending === 'discharge' || s.power_state === 'DISCHARGING') return 'discharge';
  if (pending || s.busy || s.power_state === 'TURNING_ON') return 'busy';
  if (state.unknown.has(d.uid) || activeFaults(d).some((f) => f.severity === 'error')) return 'fault';
  return s.output_enabled ? 'on' : 'off';
}
function chipText(d) {
  const s = d.status || {};
  const pending = state.pending.get(d.uid);
  if (!d.online) return 'Offline';
  if (pending) return { on: 'Switching on', off: 'Switching off', discharge: 'Discharging', cycle: 'Cycling', nickname: 'Renaming' }[pending] || 'Working';
  if (s.power_state === 'TURNING_ON') return 'Switching on';
  if (s.power_state === 'DISCHARGING') return 'Discharging';
  if (s.busy) return 'Busy';
  if (state.unknown.has(d.uid)) return 'Check state';
  const f = activeFaults(d).find((x) => x.severity === 'error');
  if (f) return 'Fault';
  return s.output_enabled ? 'On' : 'Off';
}
function offlineReason(d) {
  const why = (d.faults || []).find((f) => f.active && ['LINK_UNPLUGGED', 'LINK_NO_RESPONSE', 'LINK_PORT_BUSY'].includes(f.code));
  return why ? FAULT[why.code] : 'Not connected';
}
const canAct = (d) => d.online && !d.locked && !state.pending.has(d.uid) && !(d.status && d.status.busy);
const isFullyOff = (d) => { const s = d.status || {}; return s.state_code === 0 && !s.output_enabled && !s.discharge_enabled && !s.busy; };

function upsert(snapshot) {
  const d = { ...(state.devices.get(snapshot.uid) || {}), ...snapshot };
  state.devices.set(d.uid, d);
  if (d.status && d.online) record(d.uid, d.status, d.observed_at);
}
function record(uid, s, at) {
  const t = at ? Date.parse(at) : Date.now();
  const list = state.history.get(uid) || [];
  if (!list.length || list[list.length - 1].t < t) list.push({ t, vout: s.vout_valid ? s.vout_mv : null, vin: s.vin_valid ? s.vin_mv : null });
  while (list.length && list[0].t < Date.now() - HISTORY_MS - 2000) list.shift();
  state.history.set(uid, list);
}

// ---------- hold to confirm ----------
function holdable(btn, { onFire, hint }) {
  let timer = null, started = 0;
  btn.style.setProperty('--hold', `${HOLD_MS}ms`); // CSSOM: allowed by the page's CSP
  const begin = (e) => {
    if (btn.disabled || timer || btn.classList.contains('working')) return;
    if (e.type === 'pointerdown') {
      if (e.button !== 0) return;
      try { btn.setPointerCapture(e.pointerId); } catch { /* synthetic or already-released pointer */ }
    }
    e.preventDefault();
    e.stopPropagation();
    started = performance.now();
    btn.classList.add('holding');
    timer = setTimeout(async () => {
      timer = null;
      btn.classList.remove('holding');
      btn.classList.add('working', 'fired');
      setTimeout(() => btn.classList.remove('fired'), 700);
      try { await onFire(); } finally { btn.classList.remove('working'); }
    }, HOLD_MS);
  };
  const end = (e) => {
    if (e) e.stopPropagation();
    if (!timer) return;
    clearTimeout(timer);
    timer = null;
    btn.classList.remove('holding');
    if (performance.now() - started < HOLD_MS) hint(btn);
  };
  btn.addEventListener('pointerdown', begin);
  btn.addEventListener('pointerup', end);
  btn.addEventListener('pointercancel', end);
  btn.addEventListener('lostpointercapture', end);
  btn.addEventListener('click', (e) => e.stopPropagation());
  btn.addEventListener('keydown', (e) => { if ((e.key === ' ' || e.key === 'Enter') && !e.repeat) begin(e); });
  btn.addEventListener('keyup', (e) => { if (e.key === ' ' || e.key === 'Enter') end(e); });
  btn.addEventListener('contextmenu', (e) => e.preventDefault());
}
function bubble(anchor, text) {
  const host = anchor.closest('.tile') || anchor.closest('.sheet-scroll') || anchor.parentElement;
  host.querySelectorAll(':scope > .hint').forEach((x) => x.remove());
  const b = h('div', { class: 'hint' }, text);
  host.append(b);
  setTimeout(() => b.remove(), 1900);
}
function relabel(btn, text) {
  const sub = btn.querySelector('.sub');
  if (!sub) return bubble(btn, text);
  sub.dataset.text = sub.dataset.text || sub.textContent;
  sub.textContent = text;
  clearTimeout(btn._relabel);
  btn._relabel = setTimeout(() => { sub.textContent = sub.dataset.text; delete sub.dataset.text; }, 1600);
}
const holdHint = (d) => (d && d.status && d.status.output_enabled ? 'Hold to switch off' : 'Hold to switch on');

// ---------- actions ----------
async function act(d, action, method, path, body) {
  const name = nameOf(d);
  state.pending.set(d.uid, action);
  state.unknown.delete(d.uid);
  render();
  try {
    const r = await api(method, path, body);
    let title = `${name} ${DONE[action]}`;
    if (r.outcome === 'already_satisfied') title = `${name} was already ${action === 'discharge' ? 'discharged' : action}`;
    const vout = (r.status && r.status.vout_mv) ?? (r.on && r.on.status && r.on.status.vout_mv);
    const extra = action === 'discharge' && vout != null ? ` · ${v2(vout)} V` : r.elapsed_ms ? ` · ${r.elapsed_ms} ms` : '';
    notify('ok', title + extra);
  } catch (e) {
    if (e.code === 'unauthorized') return notify('bad', 'Signed out', e.message);
    if (e.code === 'unknown') state.unknown.set(d.uid, e.message);
    const hint = (e.faults || []).filter((f) => f.active || !f.condition).map((f) => f.suggestion)[0];
    notify(e.code === 'unknown' ? 'warn' : 'bad', e.code === 'unknown' ? `${name}: result unknown` : `Couldn't ${FAIL[action]} ${name}`, hint || e.message);
    const tile = document.querySelector(`.tile[data-uid="${CSS.escape(d.uid)}"]`);
    if (tile) { tile.classList.remove('shake'); void tile.offsetWidth; tile.classList.add('shake'); }
  } finally {
    state.pending.delete(d.uid);
    render();
  }
}
const power = (d, action) => act(d, action, 'POST', `/v1/devices/${enc(d.uid)}/power`, { action });
const toggle = (d) => d && power(d, d.status && d.status.output_enabled ? 'off' : 'on');
const cycle = (d) => act(d, 'cycle', 'POST', `/v1/devices/${enc(d.uid)}/cycle`, { off_ms: state.cycleMs, discharge: state.cycleDischarge });
const rename = (d, nickname) => act(d, 'nickname', 'PUT', `/v1/devices/${enc(d.uid)}/nickname`, { nickname });
async function setLock(d, locked) {
  try {
    const r = await api('POST', `/v1/devices/${enc(d.uid)}/lock`, { locked });
    Object.assign(d, { locked: r.locked, lock_source: r.lock_source });
    notify('ok', `${nameOf(d)} ${r.locked ? 'locked' : 'unlocked'}`, r.locked ? 'No app, script or agent can switch it until you unlock it.' : null);
  } catch (e) {
    notify('bad', "Couldn't change the lock", e.message);
  }
  render();
}

// ---------- scope (oscilloscope trace) ----------
function scopeGeometry(uid, W, H, padT, padB) {
  const now = Date.now();
  const pts = (state.history.get(uid) || []).filter((p) => p.t >= now - HISTORY_MS);
  const max = Math.max(1000, ...pts.map((p) => Math.max(p.vout || 0, p.vin || 0))) * 1.12;
  const X = (t) => W - ((now - t) / HISTORY_MS) * W;
  const Y = (mv) => padT + (1 - mv / max) * (H - padT - padB);
  let line = '', pen = false, start = null, last = null;
  const areas = [];
  const close = () => { if (pen && last) areas.push(`${line.slice(line.lastIndexOf('M'))}L${last[0]},${H - padB}L${start},${H - padB}Z`); };
  for (const p of pts) {
    if (p.vout == null) { close(); pen = false; continue; }
    const x = X(p.t).toFixed(2), y = Y(p.vout).toFixed(2);
    if (!pen) start = x;
    line += `${pen ? 'L' : 'M'}${x},${y}`;
    pen = true;
    last = [x, y];
  }
  close();
  return { line, areas, last, max, pts, X, Y, now };
}
function drawScope(svg, uid, glow) {
  const W = 300, H = 44;
  const g = scopeGeometry(uid, W, H, 4, 3);
  const kids = [svgEl('line', { class: 'rail', x1: 0, x2: W, y1: H - 3, y2: H - 3 }), svgEl('line', { class: 'rail', x1: 0, x2: W, y1: H / 2, y2: H / 2 })];
  if (g.line) kids.push(svgEl('path', { class: `trace${glow ? ' glow' : ''}`, d: g.line }));
  if (g.last) kids.push(svgEl('circle', { class: 'head', cx: g.last[0], cy: g.last[1], r: 3 }));
  svg.replaceChildren(...kids);
}

// ---------- tiles ----------
const tiles = new Map();
function makeTile(uid) {
  const el = h('article', { class: 'tile', 'data-uid': uid });
  const glyphBtn = h('button', { class: 'glyph-btn', html: `${GLYPH}<span class="ripple"></span>` });
  const chip = h('span', { class: 'chip' });
  const flags = h('div', { class: 'flags' });
  const nameText = h('div', { class: 'tile-name' });
  const uidText = h('div', { class: 'tile-uid' }, uid);
  const open = h('button', { class: 'tile-open' }, nameText, uidText);
  const big = h('div', { class: 'big' });
  const vin = h('b');
  const scope = svgEl('svg', { viewBox: '0 0 300 44', preserveAspectRatio: 'none', 'aria-hidden': 'true' });
  const note = h('div', { class: 'tile-note' });
  el.append(
    h('div', { class: 'tile-top' }, glyphBtn, h('div', {}, chip, flags)),
    open,
    h('div', { class: 'readout' }, h('div', {}, micro('VOUT'), big), h('div', { class: 'side' }, micro('VIN'), vin)),
    h('div', { class: 'scope' }, scope),
    note);
  el.addEventListener('click', () => openSheet(uid));
  holdable(glyphBtn, { onFire: () => toggle(state.devices.get(uid)), hint: (b) => bubble(b, holdHint(state.devices.get(uid))) });
  return { el, glyphBtn, chip, flags, nameText, open, big, vin, scope, note };
}
function updateTile(t, d) {
  const s = d.status || {};
  const vis = visual(d);
  const label = nameOf(d);
  t.el.dataset.state = vis;
  t.glyphBtn.dataset.caught = String(!!s.output_enabled && d.online);
  t.glyphBtn.disabled = !canAct(d);
  t.glyphBtn.setAttribute('aria-label', d.locked ? `${label} is locked` : `${holdHint(d)}: ${label}`);
  t.glyphBtn.title = d.locked ? 'Locked — open to unlock' : holdHint(d);
  t.chip.replaceChildren(h('i'), chipText(d));
  t.nameText.textContent = label;
  t.nameText.classList.toggle('unnamed', !d.nickname);
  t.open.setAttribute('aria-label', `${label}, ${chipText(d)}. Open details`);

  if (!d.online) t.big.replaceChildren('—');
  else if (s.vout_valid) t.big.replaceChildren(v3(s.vout_mv), h('small', {}, 'V'));
  else t.big.replaceChildren('Unreadable');
  t.big.className = `big${d.online && !s.vout_valid ? ' invalid' : ''}`;
  t.vin.textContent = !d.online ? '—' : s.vin_valid ? `${v2(s.vin_mv)} V` : '—';
  t.scope.parentNode.hidden = !d.online;
  if (d.online) drawScope(t.scope, d.uid, vis === 'on');

  const faults = activeFaults(d);
  const flags = [];
  if (d.locked) flags.push(h('span', { class: 'flag-lock', title: d.lock_source === 'config' ? 'Locked in config.toml' : 'Locked', html: ICON.lock }));
  if (faults.length) flags.push(h('span', { class: faults.some((f) => f.severity === 'error') ? 'flag-bad' : 'flag-warn', title: faults.map((f) => FAULT[f.code] || f.code).join(', '), html: ICON.warn }));
  t.flags.replaceChildren(...flags);

  let note = '', cls = '';
  if (state.unknown.has(d.uid)) { note = 'Result unknown — check before acting'; cls = 'bad'; }
  else if (!d.online) { note = `${offlineReason(d)} · output keeps its last state`; cls = 'quiet'; }
  else if (faults.length) { note = FAULT[faults[0].code] || faults[0].code; cls = faults[0].severity === 'error' ? 'bad' : ''; }
  else if (!s.output_enabled && !s.busy && s.vout_valid && s.vout_mv > 1000) note = `Load still holds ${v2(s.vout_mv)} V`;
  t.note.textContent = note;
  t.note.className = `tile-note ${cls}`;
  t.note.hidden = !note;
}

// ---------- sheet ----------
function openSheet(uid) {
  const dlg = $('sheet');
  if (state.sheet && state.sheet.uid === uid && dlg.open) return;
  state.sheet = makeSheet(uid);
  dlg.replaceChildren(state.sheet.root);
  if (!dlg.open) dlg.showModal();
  state.sheet.update(state.devices.get(uid));
  state.sheet.close.focus();
}
const closeSheet = () => { const dlg = $('sheet'); if (dlg.open) dlg.close(); };

function holdButton(cls, label, sub) {
  return h('button', { class: `hold-btn ${cls}` }, h('span', { class: 'fill' }), h('span', { class: 'label' }, label, h('span', { class: 'sub' }, sub)), h('span', { class: 'kicker' }));
}

function makeSheet(uid) {
  const sheet = { uid, editing: false, hoverX: null };
  const current = () => state.devices.get(uid);
  const glyphBtn = h('button', { class: 'glyph-btn', html: `${GLYPH}<span class="ripple"></span>` });
  const nameInput = h('input', { type: 'text', maxlength: '64', autocomplete: 'off', spellcheck: 'false', 'aria-label': 'Name', id: 'sheet-name' });
  const sub = h('div', { class: 'sub' });
  const close = h('button', { class: 'close', 'aria-label': 'Close', html: ICON.x, onclick: closeSheet });
  const renameBar = h('div', { class: 'rename-bar', hidden: true });
  const callouts = h('div');
  const hero = h('div', { class: 'hero' });
  const chart = h('div', { class: 'chart' });
  const primary = holdButton('primary', 'Switch On', 'Hold to confirm');
  const discharge = holdButton('btn-discharge', 'Discharge', 'Off, then drain ≤ 1 V');
  const cycleBtn = holdButton('btn-cycle', 'Power Cycle', '');
  const cycleSeg = h('div', { class: 'seg', role: 'radiogroup', 'aria-label': 'Off for' });
  for (const ms of [500, 1000, 2000, 5000, 10000]) {
    cycleSeg.append(h('button', { role: 'radio', 'data-ms': ms, onclick: () => { state.cycleMs = ms; sheet.update(current()); } }, ms < 1000 ? '0.5 s' : `${ms / 1000} s`));
  }
  const dischargeFirst = h('input', { type: 'checkbox', role: 'switch', 'aria-label': 'Discharge before waiting', onchange: (e) => { state.cycleDischarge = e.target.checked; sheet.update(current()); } });
  const lockSwitch = h('input', { type: 'checkbox', role: 'switch', 'aria-label': 'Allow changes', onchange: (e) => { const d = current(); if (d) setLock(d, !e.target.checked); } });
  const lockDesc = h('span', { class: 'desc' });
  const faultsTitle = h('div', { class: 'group-title' }, micro('Needs attention'));
  const faultsGroup = h('div', { class: 'group' });
  const info = h('div', { class: 'group' });
  const recentTitle = h('div', { class: 'group-title' }, micro('Recent activity'));
  const recent = h('div', { class: 'group' });

  const root = h('div', { class: 'sheet-scroll' },
    h('div', { class: 'sheet-head' }, glyphBtn, h('div', { class: 'name-field' }, nameInput, sub), close),
    renameBar, callouts, hero, chart,
    h('div', { class: 'actions' }, primary, h('div', { class: 'row2' }, discharge, cycleBtn)),
    h('div', { class: 'group-title' }, micro('Power cycle')),
    h('div', { class: 'group' },
      h('div', { class: 'field' }, micro('Off for'), cycleSeg),
      h('div', { class: 'item' }, h('span', { class: 'k' }, h('b', {}, 'Discharge before waiting'), h('span', { class: 'desc' }, 'Wait only once VOUT ≤ 1 V is confirmed')), h('label', { class: 'switch' }, dischargeFirst))),
    faultsTitle, faultsGroup,
    h('div', { class: 'group-title' }, micro('Access')),
    h('div', { class: 'group' }, h('div', { class: 'item' }, h('span', { class: 'k' }, h('b', {}, 'Allow changes'), lockDesc), h('label', { class: 'switch' }, lockSwitch))),
    h('div', { class: 'group-title' }, micro('Device')),
    info, recentTitle, recent);

  holdable(glyphBtn, { onFire: () => toggle(current()), hint: (b) => bubble(b, holdHint(current())) });
  holdable(primary, { onFire: () => toggle(current()), hint: (b) => relabel(b, 'Keep holding…') });
  holdable(discharge, { onFire: () => { const d = current(); return d && power(d, 'discharge'); }, hint: (b) => relabel(b, 'Keep holding…') });
  holdable(cycleBtn, { onFire: () => { const d = current(); return d && cycle(d); }, hint: (b) => relabel(b, 'Keep holding…') });

  // Rename in place; needs the output fully off.
  const bytes = (s) => new TextEncoder().encode(s).length;
  const showRename = () => {
    const d = current();
    const changed = nameInput.value !== (d.nickname || '');
    const n = bytes(nameInput.value);
    sheet.editing = changed;
    renameBar.hidden = !changed;
    renameBar.replaceChildren(
      h('button', { class: 'pill-btn primary', disabled: n > 24, onclick: saveName }, 'Save'),
      h('button', { class: 'pill-btn', onclick: cancelName }, 'Cancel'),
      h('span', {}, n > 24 ? `${n}/24 bytes · too long` : `Stored on the device · ${n}/24 bytes`));
  };
  const saveName = () => {
    if (bytes(nameInput.value) > 24) return;
    sheet.editing = false;
    renameBar.hidden = true;
    nameInput.blur();
    rename(current(), nameInput.value);
  };
  const cancelName = () => {
    sheet.editing = false;
    renameBar.hidden = true;
    nameInput.value = current().nickname || '';
    nameInput.blur();
  };
  nameInput.addEventListener('input', showRename);
  nameInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); if (sheet.editing) saveName(); }
    if (e.key === 'Escape' && sheet.editing) { e.preventDefault(); e.stopPropagation(); cancelName(); }
  });
  chart.addEventListener('pointermove', (e) => {
    const r = chart.getBoundingClientRect();
    sheet.hoverX = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    drawChart(chart, uid, sheet.hoverX);
  });
  chart.addEventListener('pointerleave', () => { sheet.hoverX = null; drawChart(chart, uid, null); });

  sheet.root = root;
  sheet.close = close;
  sheet.update = (d) => {
    if (!d) return;
    const s = d.status || {};
    const label = nameOf(d);
    const vis = visual(d);
    root.dataset.state = vis;
    const offOk = d.online && isFullyOff(d) && !d.locked && !state.pending.has(uid);
    if (!sheet.editing) nameInput.value = d.nickname || '';
    nameInput.placeholder = label;
    nameInput.disabled = !offOk;
    nameInput.title = offOk ? 'Click to rename' : d.locked ? 'Unlock to rename' : 'Switch off to rename';
    sub.replaceChildren(...[h('span', { class: 'chip' }, h('i'), chipText(d)), d.locked ? h('span', { class: 'chip lock-chip' }, h('span', { class: 'flag-lock', html: ICON.lock }), 'Locked') : null, micro(d.uid)].filter(Boolean));
    glyphBtn.dataset.caught = String(!!s.output_enabled && d.online);
    glyphBtn.disabled = !canAct(d);
    glyphBtn.setAttribute('aria-label', `${holdHint(d)}: ${label}`);

    const items = [];
    if (state.unknown.has(uid)) items.push(h('div', { class: 'callout bad', html: ICON.warn },
      h('span', {}, h('b', {}, 'Result unknown. '), state.unknown.get(uid), ' The reading below is fresh — check it before doing anything else.'),
      h('button', { class: 'x', onclick: () => { state.unknown.delete(uid); render(); } }, 'OK')));
    if (!d.online) items.push(h('div', { class: 'callout warn', html: ICON.info }, h('span', {}, h('b', {}, `${offlineReason(d)}. `), 'The output keeps whatever state it had, which is unknown from here. Reconnect the USB cable.')));
    if (d.online && !s.output_enabled && !s.busy && s.vout_valid && s.vout_mv > 1000) items.push(h('div', { class: 'callout warn', html: ICON.info }, h('span', {}, h('b', {}, `The load still holds ${v2(s.vout_mv)} V. `), 'Switching off does not drain it — hold Discharge to bring it to ≤ 1 V.')));
    setKeyed(callouts, items.map((x) => x.textContent).join('|'), () => items);

    const reading = (cls, tag, sublabel, mv, valid, digits) => h('div', { class: cls },
      h('div', { class: 'label' }, micro(tag), micro(sublabel)),
      !d.online ? h('div', { class: 'value invalid' }, '—')
        : valid ? h('div', { class: 'value' }, digits(mv), h('small', {}, 'V'))
          : h('div', { class: 'value invalid', title: 'Invalid reading — not zero volts' }, 'Unreadable'));
    hero.replaceChildren(
      reading('primary', 'VOUT', '· to load', s.vout_mv, s.vout_valid, v3),
      reading('secondary', 'VIN', '· source', s.vin_mv, s.vin_valid, v2),
      h('div', { class: 'when' }, micro(d.online ? `Sampled ${ago(d.observed_at)} · ${state.live ? 'live' : 'paused'}` : 'No live data')));
    drawChart(chart, uid, sheet.hoverX);

    const can = canAct(d);
    primary.disabled = discharge.disabled = cycleBtn.disabled = !can;
    primary.classList.toggle('primary', !s.output_enabled);
    primary.classList.toggle('btn-off', !!s.output_enabled);
    const pLabel = primary.querySelector('.label');
    if (!primary.querySelector('.sub').dataset.text) pLabel.firstChild.textContent = s.output_enabled ? 'Switch Off' : 'Switch On';
    cycleBtn.querySelector('.sub').dataset.text || (cycleBtn.querySelector('.sub').textContent = `${state.cycleDischarge ? 'Drain' : 'Off'} · ${state.cycleMs < 1000 ? '0.5' : state.cycleMs / 1000} s · on`);
    for (const b of cycleSeg.children) b.setAttribute('aria-checked', String(Number(b.dataset.ms) === state.cycleMs));
    dischargeFirst.checked = state.cycleDischarge;
    lockSwitch.checked = !d.locked;
    lockSwitch.disabled = d.lock_source === 'config';
    lockDesc.textContent = d.lock_source === 'config' ? 'Locked in config.toml' : d.locked ? 'Locked — no app, script or agent can switch or rename it' : 'Web, Terminal, AI agents and the API can switch it';

    const shown = [...activeFaults(d), ...recentEvents(d)];
    faultsTitle.hidden = faultsGroup.hidden = !shown.length;
    setKeyed(faultsGroup, JSON.stringify(shown.map((f) => [f.code, f.active, f.count, Math.round((Date.now() - Date.parse(f.active ? f.first_seen : f.last_seen)) / 60000)])), () => shown.map(faultItem));

    const id = d.identity || {};
    const m = d.metrics || {};
    const row = (k, v) => h('div', { class: 'item' }, h('span', { class: 'k' }, k), h('span', { class: 'v' }, v));
    const p50 = m.status_p50_us ? (m.status_p50_us / 1000).toFixed(2) : null;
    setKeyed(info, JSON.stringify([d.uid, d.port, id, p50]), () => [
      h('div', { class: 'item' }, h('span', { class: 'k' }, 'UID'), h('span', { class: 'v' }, d.uid),
        h('button', { class: 'copy', onclick: () => navigator.clipboard && navigator.clipboard.writeText(d.uid).then(() => notify('ok', 'UID copied')) }, 'Copy')),
      row('Port', d.port || '—'),
      row('Firmware', id.firmware_version ? `${id.firmware_version} · protocol ${id.protocol_version}` : '—'),
      row('Hardware', id.hardware_version || '—'),
      row('Round trip', p50 ? `${p50} ms p50 · ${(m.status_p99_us / 1000).toFixed(2)} ms p99` : '—')]);

    const mine = state.ops.filter((o) => o.uid === uid).slice(0, 5);
    recentTitle.hidden = recent.hidden = !mine.length;
    setKeyed(recent, mine.map((o) => `${o.at}${o.action}${o.ok}${ago(o.at)}`).join('|'), () => mine.map(opRow));
  };
  return sheet;
}

function drawChart(box, uid, hoverX) {
  const W = 540, H = 170, PL = 40, PR = 46, PT = 6, PB = 22;
  const IW = W - PL - PR;
  const g = scopeGeometry(uid, IW, H, PT, PB);
  const top = g.max / 1.12;
  const step = [0.5, 1, 2, 5, 10, 20, 25, 50].map((x) => x * 1000).find((x) => x * 2 >= top * 1.05) || Math.ceil(top / 2000) * 1000;
  const max = step * 2;
  const X = (t) => PL + (1 - (g.now - t) / HISTORY_MS) * IW;
  const Y = (mv) => PT + (1 - mv / max) * (H - PT - PB);
  const svg = svgEl('svg', { viewBox: `0 0 ${W} ${H}`, role: 'img', 'aria-label': `Output voltage, last minute, 0 to ${max / 1000} V` });
  const grid = svgEl('g', { class: 'grid' });
  const axis = svgEl('g', { class: 'axis' });
  for (let i = 0; i <= 4; i++) {
    const mv = (max / 4) * i;
    grid.append(svgEl('line', { class: i % 2 === 0 ? 'major' : '', x1: PL, x2: W - PR, y1: Y(mv), y2: Y(mv) }));
    if (i % 2 === 0) { const t = svgEl('text', { x: PL - 8, y: Y(mv) + 3.5, 'text-anchor': 'end' }); t.textContent = `${mv / 1000}V`; axis.append(t); }
  }
  for (let i = 0; i <= 6; i++) grid.append(svgEl('line', { x1: PL + (IW / 6) * i, x2: PL + (IW / 6) * i, y1: PT, y2: H - PB }));
  for (const [frac, txt] of [[0, '-60s'], [0.5, '-30s'], [1, 'NOW']]) {
    const t = svgEl('text', { x: PL + frac * IW, y: H - 5, 'text-anchor': frac === 0 ? 'start' : frac === 1 ? 'end' : 'middle' });
    t.textContent = txt;
    axis.append(t);
  }
  svg.append(grid, axis);
  const path = (key) => {
    let d = '', pen = false, start = null, last = null;
    const areas = [];
    const close = () => { if (pen && last && key === 'vout') areas.push(`${d.slice(d.lastIndexOf('M'))}L${last[0]},${Y(0)}L${start},${Y(0)}Z`); };
    for (const p of g.pts) {
      if (p[key] == null) { close(); pen = false; continue; }
      const x = X(p.t).toFixed(1), y = Y(p[key]).toFixed(1);
      if (!pen) start = x;
      d += `${pen ? 'L' : 'M'}${x},${y}`;
      pen = true;
      last = [x, y];
    }
    close();
    return { d, areas, last };
  };
  const vin = path('vin'), vout = path('vout');
  for (const a of vout.areas) svg.append(svgEl('path', { class: 'vout-area', d: a }));
  if (vin.d) svg.append(svgEl('path', { class: 'vin', d: vin.d }));
  if (vout.d) svg.append(svgEl('path', { class: 'vout', d: vout.d }));
  if (vout.last) svg.append(svgEl('circle', { class: 'head', cx: vout.last[0], cy: vout.last[1], r: 3.5 }));
  const labels = [];
  if (vout.last) labels.push({ y: Number(vout.last[1]), text: 'VOUT', cls: 'l-vout' });
  if (vin.last) labels.push({ y: Number(vin.last[1]), text: 'VIN', cls: 'l-vin' });
  if (labels.length === 2 && Math.abs(labels[0].y - labels[1].y) < 13) {
    const mid = (labels[0].y + labels[1].y) / 2;
    const [a, b] = labels[0].y <= labels[1].y ? labels : [labels[1], labels[0]];
    a.y = mid - 6.5; b.y = mid + 6.5;
  }
  for (const l of labels) { const t = svgEl('text', { class: `series-label ${l.cls}`, x: W - PR + 8, y: l.y + 3.5 }); t.textContent = l.text; svg.append(t); }
  const old = box.querySelector('.tip');
  if (old) old.remove();
  if (hoverX != null && g.pts.length) {
    const tAt = g.now - (1 - (hoverX * W - PL) / IW) * HISTORY_MS;
    const p = g.pts.reduce((a, b) => (Math.abs(b.t - tAt) < Math.abs(a.t - tAt) ? b : a));
    const x = X(p.t);
    svg.append(svgEl('line', { class: 'cross', x1: x, x2: x, y1: PT, y2: H - PB }));
    if (p.vout != null) svg.append(svgEl('circle', { class: 'dot', cx: x, cy: Y(p.vout), r: 5 }));
    const secs = Math.round((g.now - p.t) / 1000);
    const tip = h('div', { class: 'tip' }, h('b', {}, p.vout == null ? 'Unreadable' : `${v3(p.vout)} V`), h('span', {}, `  VIN ${p.vin == null ? '—' : v2(p.vin)} · ${secs ? `${secs}s ago` : 'now'}`));
    tip.style.left = `${Math.min(84, Math.max(16, (x / W) * 100))}%`; // CSSOM, allowed by the CSP
    box.append(tip);
  }
  const prev = box.querySelector('svg');
  if (prev) prev.replaceWith(svg); else box.prepend(svg);
}

// ---------- activity ----------
function faultItem(f) {
  const c = f.severity === 'error' ? 'var(--coral)' : f.severity === 'warning' ? 'var(--apricot)' : 'var(--sky)';
  const ico = h('span', { class: 'badge-ico', html: f.severity === 'info' ? ICON.info : ICON.warn });
  ico.style.setProperty('--c', c);
  return h('div', { class: 'item' }, ico,
    h('span', { class: 'k' }, h('b', {}, FAULT[f.code] || f.code), h('span', { class: 'desc' }, f.suggestion),
      h('span', { class: 'code' }, `${f.code} · ${f.active ? 'since' : 'at'} ${ago(f.active ? f.first_seen : f.last_seen)}${f.count > 1 ? ` · ×${f.count}` : ''}`)));
}
function opRow(o) {
  const d = state.devices.get(o.uid);
  const name = d ? nameOf(d) : o.uid;
  const failed = o.ok === false;
  const [c, icon] = failed ? ['var(--coral)', ICON.x] : ACTION_STYLE[o.action] || ['var(--text-2)', ICON.off];
  let title;
  if (failed) title = o.error && o.error.code === 'unknown' ? `${name}: ${FAIL[o.action] || o.action} — result unknown` : `Couldn't ${FAIL[o.action] || o.action} ${name}`;
  else if (o.outcome === 'already_satisfied') title = `${name} was already ${o.action === 'discharge' ? 'discharged' : o.action}`;
  else title = `${name} ${DONE[o.action] || o.action}`;
  const ico = h('span', { class: 'badge-ico', html: icon });
  ico.style.setProperty('--c', c);
  return h('div', { class: 'item' }, ico,
    h('span', { class: 'k' }, h('span', { class: 'title' }, title), h('span', { class: 'meta' }, [SOURCE[o.source] || o.source, failed && o.error ? o.error.message : null].filter(Boolean).join(' · '))),
    h('span', { class: 'time' }, clock(o.at)));
}
function faultRow(f) {
  const d = state.devices.get(f.uid);
  const st = f.active ? ['active', 'Active'] : f.condition ? ['resolved', 'Resolved'] : ['event', 'Event'];
  const row = faultItem(f);
  row.querySelector('b').textContent = `${d ? nameOf(d) : f.uid || f.port || ''} · ${FAULT[f.code] || f.code}`;
  row.append(h('span', { class: `state ${st[0]}` }, st[1]));
  return row;
}
function renderFeed() {
  const items = state.feed === 'faults' ? state.faults.map((f) => ({ at: f.last_seen, row: faultRow(f) })) : state.ops.map((o) => ({ at: o.at, row: opRow(o) }));
  if (!items.length) {
    $('feed').replaceChildren(h('div', { class: 'feed-empty' }, state.feed === 'faults' ? 'No faults. Everything is working.' : 'Nothing has happened yet.'));
    return;
  }
  const out = [];
  let day = null, group = null;
  for (const it of items.slice(0, 200)) {
    const label = dayLabel(it.at);
    if (label !== day) { day = label; out.push(h('div', { class: 'day' }, micro(label))); group = h('div', { class: 'group' }); out.push(group); }
    group.append(it.row);
  }
  $('feed').replaceChildren(...out);
}

// ---------- render ----------
let scheduled = false;
function render() {
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(() => { scheduled = false; draw(); });
}
function stat(color, value, label) {
  const i = h('i');
  if (color) i.style.setProperty('--c', color);
  return h('span', { class: 'stat' }, i, h('b', {}, value), label);
}
function draw() {
  const all = [...state.devices.values()];
  const online = all.filter((d) => d.online);
  const on = online.filter((d) => d.status && d.status.output_enabled);
  const attention = all.filter((d) => !d.online || activeFaults(d).length || state.unknown.has(d.uid));
  const pulse = h('span', { class: `pulse${state.live && !signedOut ? '' : ' down'}` });
  $('subtitle').replaceChildren(pulse, signedOut ? 'Signed out — run voloo ui' : state.live ? 'Live · LibreZoo' : 'Reconnecting');
  setKeyed($('strip'), `${all.length}|${online.length}|${on.length}|${attention.length}`, () => [
    stat('var(--text-2)', all.length, all.length === 1 ? 'device' : 'devices'),
    stat('var(--volt)', on.length, 'powered'),
    stat('var(--off)', online.length - on.length, 'off'),
    attention.length ? stat('var(--coral)', attention.length, 'need attention') : null,
  ].filter(Boolean));
  const active = state.faults.filter((f) => f.active).length;
  $('badge').textContent = String(active);
  $('badge').hidden = !active;

  $('view-devices').hidden = state.view !== 'devices';
  $('view-activity').hidden = state.view !== 'activity';
  for (const b of document.querySelectorAll('[data-view]')) b.setAttribute('aria-selected', String(b.dataset.view === state.view));
  for (const b of document.querySelectorAll('[data-feed]')) b.setAttribute('aria-selected', String(b.dataset.feed === state.feed));

  const f = state.filter.trim().toLowerCase();
  const list = all.filter((d) => !f || nameOf(d).toLowerCase().includes(f) || d.uid.toLowerCase().includes(f))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), undefined, { numeric: true }));
  const box = $('tiles');
  for (const [uid, t] of tiles) if (!state.devices.has(uid)) { t.el.remove(); tiles.delete(uid); }
  list.forEach((d, i) => {
    let t = tiles.get(d.uid);
    if (!t) { t = makeTile(d.uid); tiles.set(d.uid, t); }
    updateTile(t, d);
    if (box.children[i] !== t.el) box.insertBefore(t.el, box.children[i] || null);
  });
  for (const [uid, t] of tiles) if (!list.some((d) => d.uid === uid) && t.el.parentNode) t.el.remove();
  $('empty').hidden = all.length > 0;

  if (state.view === 'activity') renderFeed();
  if (state.sheet && $('sheet').open) state.sheet.update(state.devices.get(state.sheet.uid));
}

// ---------- data ----------
function normalizeOp(r) {
  if ('ok' in r) return { at: r.at, uid: r.uid, action: r.action, source: r.source, ok: r.ok, outcome: r.outcome, error: r.error || null };
  const failed = !!r.error;
  return { at: r.at, uid: r.uid, action: r.action, source: r.source, ok: !failed, outcome: r.outcome, error: failed ? { code: r.outcome, message: r.error } : null };
}
async function refresh() {
  const [devices, ops, faults] = await Promise.all([api('GET', '/v1/devices'), api('GET', '/v1/operations?limit=200'), api('GET', '/v1/faults?limit=300')]);
  state.devices.clear();
  devices.forEach(upsert);
  state.ops = ops.slice().reverse().map(normalizeOp);
  state.faults = faults;
  render();
}
function mergeFault(f) {
  const key = (x) => `${x.uid || x.port}|${x.code}`;
  state.faults = [f, ...state.faults.filter((x) => key(x) !== key(f))].slice(0, 300);
  const d = state.devices.get(f.uid);
  if (d) d.faults = [f, ...(d.faults || []).filter((x) => x.code !== f.code)].filter((x) => x.active || !x.condition);
}
function connect() {
  const es = new EventSource('/v1/events');
  const on = (kind, fn) => es.addEventListener(kind, (e) => { fn(JSON.parse(e.data)); render(); });
  es.onopen = () => { state.live = true; render(); };
  es.onerror = () => { state.live = false; render(); };
  on('hello', (e) => e.devices.forEach(upsert));
  for (const k of ['device.added', 'device.removed', 'device.offline', 'device.online']) on(k, (e) => upsert(e.data));
  on('status', (e) => {
    const d = state.devices.get(e.data.uid) || { uid: e.data.uid };
    Object.assign(d, { status: e.data.status, observed_at: e.data.observed_at, faults: e.data.faults, online: true, error: null });
    state.devices.set(d.uid, d);
    record(d.uid, e.data.status, e.data.observed_at);
  });
  on('device.locked', (e) => { const d = state.devices.get(e.data.uid); if (d) Object.assign(d, e.data); });
  on('operation.started', (e) => { if (!state.pending.has(e.data.uid)) state.pending.set(e.data.uid, e.data.action); });
  on('operation.finished', (e) => {
    const op = normalizeOp({ ...e.data, at: e.at });
    state.ops = [op, ...state.ops].slice(0, 300);
    if (op.source !== 'ui') {
      state.pending.delete(op.uid);
      if (op.ok === false && op.error && op.error.code === 'unknown') state.unknown.set(op.uid, op.error.message);
      const d = state.devices.get(op.uid);
      if (d && op.ok) notify('ok', `${nameOf(d)} ${DONE[op.action] || op.action}`, `by ${SOURCE[op.source] || op.source}`);
    }
  });
  for (const k of ['fault.raised', 'fault.occurred', 'fault.cleared']) on(k, (e) => mergeFault(e.data));
  on('lagged', () => refresh().catch(() => {}));
}

document.addEventListener('DOMContentLoaded', () => {
  $('filter').addEventListener('input', (e) => { state.filter = e.target.value; render(); });
  for (const b of document.querySelectorAll('[data-view]')) b.addEventListener('click', () => { state.view = b.dataset.view; render(); });
  for (const b of document.querySelectorAll('[data-feed]')) b.addEventListener('click', () => { state.feed = b.dataset.feed; render(); });
  const dlg = $('sheet');
  dlg.addEventListener('click', (e) => { if (e.target === dlg) closeSheet(); });
  dlg.addEventListener('close', () => { state.sheet = null; });
  document.addEventListener('keydown', (e) => { if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); $('filter').focus(); } });
  refresh().then(connect).catch((e) => { signedOut = e.code === 'unauthorized'; render(); });
  setInterval(render, 1000);
});
