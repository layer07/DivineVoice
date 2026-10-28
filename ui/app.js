// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * app.js
 *
 * Version: @(#)app.js 0.2.0 2026
 *
 * Description: The Settings Page Brain.
 *              No framework. View is pure function: state in, HTML out. One
 *              listener per event type, delegated. Rust is truth; page ask, Rust
 *              answer. Everything user-controlled go through esc(). Trust no one.
 *
 * Author: Magnus Leatti (layer07)
 * URL: https://kernelriot.com
 * Github: /layer07
 *
 *        ██▓    ▄▄▄     ▓██   ██▓▓█████  ██▀███
 *       ▓██▒   ▒████▄    ▒██  ██▒▓█   ▀ ▓██ ▒ ██▒
 *       ▒██░   ▒██  ▀█▄   ▒██ ██░▒███   ▓██ ░▄█ ▒
 *       ▒██░   ░██▄▄▄▄██  ░ ▐██▓░▒▓█ ▄ ▒██▀▀█▄
 *       ░██████▒▓█   ▓██▒ ░ ██▒▓░░▒████▒░██▓ ▒██▒
 *       ░ ▒░▓  ░▒▒   ▓▒█░  ██▒▒▒ ░░ ▒░ ░░ ▒▓ ░▒▓░
 *       ░ ░ ▒  ░ ▒   ▒▒ ░▓██ ░▒░  ░ ░  ░  ░▒ ░ ▒░
 *         ░ ░    ░   ▒   ▒ ▒ ░░     ░     ░░   ░
 *           ░  ░     ░  ░░ ░        ░  ░   ░
 */

// No framework. Views are pure functions: state in, HTML string out. Events
// are delegated off `document` via data-act / data-key attributes, so there is
// exactly one listener per event type and zero listener leaks. Rust is the
// source of truth: we send commands, it sends back the new world.
//
//   Rust → us   window.dv.recv({ev, data})
//   us → Rust   ipc('cmd', {...args})
//
// Everything user-controlled goes through esc() before touching innerHTML.
// Transcripts are user-controlled. You are user-controlled. Trust no one.
//
// The whole file is one IIFE: wry's injected bridge already squats on global
// names (it declares `ipc`), and top-level const in a classic script shares
// the global lexical scope. Ask us how we found out. The only export is
// window.dv.
(() => {
'use strict';

// ── plumbing ─────────────────────────────────────────────────────────────

const $ = (sel, root = document) => root.querySelector(sel);
const ipc = (cmd, args = {}) => window.ipc.postMessage(JSON.stringify({ cmd, ...args }));
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
/// Conditional HTML. Pass a thunk when the HTML dereferences something that
/// only exists if `cond` holds: template literals are eager, and
/// `when(dl, \`${dl.done}\`)` with dl = null is a TypeError, not a no-op.
const when = (cond, html) => (cond ? (typeof html === 'function' ? html() : html) : '');
const fmtBytes = (n) => {
  const u = ['B', 'KB', 'MB', 'GB'];
  const i = Math.min(u.length - 1, Math.floor(Math.log(Math.max(n, 1)) / Math.log(1024)));
  return `${(n / 1024 ** i).toFixed(i >= 2 ? (i === 3 ? 2 : 0) : 0)} ${u[i]}`;
};
const ago = (ms) => {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  return s < 60 ? 'just now' : s < 3600 ? `${s / 60 | 0}m ago` : s < 86400 ? `${s / 3600 | 0}h ago` : `${s / 86400 | 0}d ago`;
};
const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
const day = (ms) => new Date(ms).toLocaleDateString([], { month: 'short', day: 'numeric' });

// ── icons (stroke = currentColor, 24 grid) ──────────────────────────────

const svg = (d) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="square">${d}</svg>`;
const I = {
  core:     svg('<circle cx="12" cy="12" r="3"/><circle cx="12" cy="12" r="8"/><path d="M12 1v3M12 20v3M1 12h3M20 12h3"/>'),
  models:   svg('<path d="M12 2 3 7v10l9 5 9-5V7z"/><path d="M3 7l9 5 9-5M12 12v10"/>'),
  audio:    svg('<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v4M8 22h8"/>'),
  history:  svg('<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5M12 7v5l3 3"/>'),
  perf:     svg('<path d="M3 20h18"/><path d="M6 16v-3M10 16V8M14 16v-6M18 16V4"/>'),
  bench:    svg('<rect x="3" y="4" width="18" height="16"/><path d="M7 9l3 3-3 3M12 15h5"/>'),
  sound:    svg('<path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11"/>'),
  mute:     svg('<path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M17 9l5 6M22 9l-5 6"/>'),
  advanced: svg('<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><rect x="14" y="4" width="4" height="4"/><rect x="8" y="10" width="4" height="4"/><rect x="16" y="16" width="4" height="4"/>'),
  about:    svg('<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7v.01"/>'),
  globe:    svg('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"/>'),
  copy:     svg('<rect x="8" y="8" width="13" height="13"/><path d="M16 8V3H3v13h5"/>'),
  trash:    svg('<path d="M4 7h16M10 11v6M14 11v6M6 7l1 14h10l1-14M9 7V3h6v4"/>'),
  dl:       svg('<path d="M12 3v13M6 10l6 6 6-6M4 21h16"/>'),
  folder:   svg('<path d="M3 6h7l2 2h9v12H3z"/>'),
  refresh:  svg('<path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7"/>'),
};

// ── state ────────────────────────────────────────────────────────────────

const S = {
  snap: null,          // last full snapshot from Rust
  mics: [],
  page: 'general',
  learning: false,
  learnErr: null,
  confirm: null,       // "del:<id>" / "clear" — two-click destructive actions
};

const cfg = () => S.snap.config;

const PAGES = {
  general:  { name: 'Core',     icon: I.core,     view: () => viewGeneral() },
  models:   { name: 'Models',   icon: I.models,   view: () => viewModels() },
  audio:    { name: 'Audio',    icon: I.audio,    view: () => viewAudio() },
  perf:     { name: 'Perf',     icon: I.perf,     view: () => viewPerf() },
  bench:    { name: 'Benchmark', icon: I.bench,   view: () => viewBench() },
  history:  { name: 'History',  icon: I.history,  view: () => viewHistory() },
  advanced: { name: 'Advanced', icon: I.advanced, view: () => viewAdvanced() },
  about:    { name: 'About',    icon: I.about,    view: () => viewAbout() },
};

const OUTPUT = [
  { id: 'paste',       name: 'Paste',       how: 'Clipboard + Ctrl+V, your clipboard restored after.', tags: ['local', 'any language', 'instant'] },
  { id: 'unicode',     name: 'Unicode',     how: 'Types each character via SendInput. Clipboard untouched.', tags: ['local', 'any language'] },
  { id: 'paste_plain', name: 'Paste plain', how: 'Clipboard + Ctrl+Shift+V, for terminals that think Ctrl+V means something.', tags: ['terminals', 'any language'] },
  { id: 'type',        name: 'Type · HID',  how: 'Real scancodes through the Interception driver. Survives Parsec and hardware KVMs.', tags: ['local', 'parsec', 'kvm', 'us-ascii'], hid: true },
];

/// Engine chip text + what it means, for the hover.
const ENG = {
  tdt:      ['TDT', 'token-and-duration transducer: skips silence, fast'],
  unified:  ['RNNT', 'transducer, offline mode: full-clip context'],
  nemotron: ['RNNT·S', 'cache-aware streaming transducer'],
  cohere:   ['AED', 'attention encoder-decoder: big, careful, slower'],
};
const engChip = (e) => `<span class="chip" title="${esc((ENG[e] || [e, ''])[1])}">${esc((ENG[e] || [e])[0])}</span>`;

/// The language picker. Plain ISO codes; Nemotron's locale keys are mapped in Rust.
const LANGS = [['auto', 'Auto-detect'], ['en', 'English'], ['es', 'Spanish'], ['fr', 'French'], ['de', 'German'],
  ['it', 'Italian'], ['pt', 'Portuguese'], ['nl', 'Dutch'], ['pl', 'Polish'], ['ru', 'Russian'], ['uk', 'Ukrainian'],
  ['tr', 'Turkish'], ['el', 'Greek'], ['sv', 'Swedish'], ['da', 'Danish'], ['fi', 'Finnish'], ['no', 'Norwegian'],
  ['cs', 'Czech'], ['ro', 'Romanian'], ['hu', 'Hungarian'], ['ar', 'Arabic'], ['hi', 'Hindi'], ['ja', 'Japanese'],
  ['ko', 'Korean'], ['zh', 'Chinese'], ['vi', 'Vietnamese']];

/// What the language setting does for the model you have loaded right now.
const langHint = () => ({
  tdt: 'Parakeet detects it per clip; this is ignored',
  unified: 'English-only model; this is ignored',
  nemotron: 'Auto works; pinning it is more accurate',
  cohere: "Cohere can't detect: pick yours (auto = English)",
}[activeEngine()] || '');
const activeEngine = () => S.snap.models.find((x) => x.id === S.snap.active_model)?.engine;

const MODE_TEXT = {
  idle: 'Standing by',
  recording: 'Recording',
  transcribing: 'Transcribing',
  injecting: 'Typing',
};

// ── shared view bits ─────────────────────────────────────────────────────

const keycaps = (binding, size = '') => `<span class="keys ${size}">${
  String(binding).split(/\+(?!$)/).map((k) => `<kbd>${esc(k)}</kbd>`).join('<span class="plus">+</span>')
}</span>`;

const head = (title, sub) => `<header class="page-head"><h1>${esc(title)}</h1>${when(sub, `<p>${sub}</p>`)}</header>`;

const sw = (key, on, label) =>
  `<label class="switch"><input type="checkbox" data-key="${key}" ${on ? 'checked' : ''}><i></i><span>${label}</span></label>`;

/// Number field with our own −/+ stepper (the native spinner is a white
/// Windows 95 artefact that no theme can reach). Shift-click steps ×10.
const STEP = { min_ms: 50, max_ms: 1000, paste_settle_ms: 10, history_max: 10, perf_max: 8 };
const num = (key, val, unit, hint, min = 0, max = 999999) => `
  <div class="field"><label class="label" for="f-${key}">${esc(key.replace(/_/g, ' '))}</label>
    <div class="unit"><div class="stepper">
      <button class="st" data-act="step" data-for="f-${key}" data-d="-1" aria-label="decrease ${key}">−</button>
      <input id="f-${key}" type="number" data-key="${key}" data-num value="${esc(val)}" min="${min}" max="${max}" step="${STEP[key] || 1}">
      <button class="st" data-act="step" data-for="f-${key}" data-d="1" aria-label="increase ${key}">+</button>
    </div><span>${unit}</span></div>
    ${when(hint, `<div class="hint">${hint}</div>`)}</div>`;

/// Native <select> with our chevron — selects can't have ::after, the wrapper can.
const select = (key, optionsHtml, style = '') =>
  `<span class="select" style="${style}"><select data-key="${key}">${optionsHtml}</select></span>`;

const meter = (label, n) =>
  `<span class="label">${label}</span><span class="meter">${[1, 2, 3, 4, 5].map((i) => `<i class="${i <= n ? 'f' : ''}"></i>`).join('')}</span>`;

const activeModelName = () => {
  const s = S.snap;
  const m = s.models.find((x) => x.id === s.active_model);
  if (m) return m.name;
  const dir = s.model.dir || cfg().model_dir || '';
  return `Custom · ${dir.split(/[\\/]/).filter(Boolean).pop() || dir}`;
};

// Where Parakeet runs. Rust probes each once at startup (a real session with
// error_on_failure, see asr.rs) and ships {device, ok, reason}.
const DEV = { cpu: 'CPU', cuda: 'CUDA', directml: 'DirectML' };
const DEV_LONG = { cpu: 'CPU', cuda: 'CUDA · NVIDIA', directml: 'DirectML · any GPU' };
// null = Rust is still probing (a second or two at startup): assume the
// configured one works rather than flash it as unavailable.
const devices = () => S.snap.devices || [...new Set(['cpu', cfg().device])].map((device) => ({ device, ok: true }));
const devOk = (d) => devices().find((x) => x.device === d)?.ok ?? d === 'cpu';
/// Can the ACTIVE model run on device `d`? null = yes, else the reason (from
/// Rust's known-bad table: e.g. Cohere/Nemotron on DirectML).
const misfit = (d) => (S.snap.unsupported || []).find((u) => u.engine === activeEngine() && u.device === d)?.why || null;
/// A device button: unavailable on this machine, or unsupported by this model, both greyed with the why.
const devBtn = (d, on) => {
  const why = !d.ok ? `unavailable: ${d.reason}` : misfit(d.device);
  return `<button class="${on ? 'on' : ''}" ${why ? 'disabled' : `data-act="set" data-k="device" data-v="${d.device}"`}
    title="${esc(why || DEV_LONG[d.device])}">${DEV[d.device]}</button>`;
};

/// The plain-English half of a GPU error; the raw ORT dump lives in the tooltip.
const gist = (why) => String(why || '').split(' (')[0];

const modelState = () => {
  const m = S.snap.model;
  return {
    loading: { cls: 'warn', text: 'loading…' },
    ready:   { cls: m.note ? 'warn' : 'ok', text: `${DEV[m.device] || m.device} · ${m.load_ms} ms load` },
    missing: { cls: 'err',  text: 'no model — download one' },
    parked:  { cls: 'warn', text: 'parked · GPU race borrowed the VRAM' },
    failed:  { cls: 'err',  text: 'failed to load' },
  }[m.state] || { cls: '', text: m.state };
};

// ── pages ────────────────────────────────────────────────────────────────

function heroHtml() {
  const mode = S.snap.status.mode;
  const toggle = cfg().hotkey_mode === 'toggle';
  const hint = mode === 'idle'
    ? `${toggle ? 'Tap' : 'Hold'} ${keycaps(cfg().hotkey, 'small')} ${toggle ? 'to start, tap again to send.' : 'and talk. Let go to send.'}`
    : mode === 'recording' ? (toggle ? 'Listening. Tap the shortcut again to send.' : 'Listening. Let go to send.')
    : mode === 'transcribing' ? 'Parakeet is chewing on it…' : 'Putting words in your cursor.';
  return `<section class="card hero" id="hero">
    <div class="orb ${mode}"></div>
    <div><div class="label">status</div><div class="big ${mode}">${MODE_TEXT[mode] || mode}</div><div class="sub">${hint}</div></div>
    <div class="spacer"></div>
    <div class="status-line"><span class="dot ${modelState().cls}"></span>${esc(activeModelName())}</div>
  </section>`;
}

function viewGeneral() {
  const c = cfg();
  const learn = S.learning
    ? `<div class="learning"><span class="rec"></span>Press your new shortcut now… <span class="spacer"></span><kbd>Esc</kbd>&nbsp;cancels</div>`
    : when(S.learnErr, `<div class="learning" style="border-color:var(--io-error);color:var(--io-error);background:var(--io-error-subtle)">${esc(S.learnErr)}</div>`);
  const tiles = OUTPUT.map((o) => {
    const hidOff = o.hid && !S.snap.interception;
    return `<button class="tile ${c.inject_mode === o.id ? 'on' : ''}" data-act="set" data-k="inject_mode" data-v="${o.id}">
      <span class="radio"></span>
      <span class="t-name">${esc(o.name)}${when(o.hid, hidOff ? '<span class="pill off">driver off</span>' : '<span class="pill ok">driver on</span>')}</span>
      <span class="t-how">${esc(o.how)}${when(hidOff, ' Without the driver it falls back to Unicode.')}</span>
      <span class="t-tags">${o.tags.map((t) => `<span class="chip">${t}</span>`).join('')}</span>
    </button>`;
  }).join('');

  return `${head('Core', 'Hold the shortcut, say the thing, let go.')}
    ${heroHtml()}
    <section class="card">
      <div class="row" style="flex-wrap:nowrap">
        <div style="flex:1;min-width:0">
          <h2>Shortcut</h2>
          <p class="desc" style="margin:0">${c.hotkey_mode === 'hold' ? 'Hold to talk, release to send.' : 'Tap to start, tap again to send.'} Any chord works, even a lone <span class="mono">RAlt</span>.</p>
        </div>
        ${keycaps(c.hotkey, 'big')}
        <div class="seg" title="Hold: walkie-talkie. Toggle: for monologues.">
          <button class="${c.hotkey_mode === 'hold' ? 'on' : ''}" data-act="set" data-k="hotkey_mode" data-v="hold">Hold</button>
          <button class="${c.hotkey_mode === 'toggle' ? 'on' : ''}" data-act="set" data-k="hotkey_mode" data-v="toggle">Toggle</button>
        </div>
        <button class="btn ${S.learning ? '' : 'primary'}" data-act="learn">${S.learning ? 'Cancel' : 'Rebind'}</button>
      </div>
      ${learn}
    </section>
    ${engineCard()}
    <section class="card">
      <h2>Output</h2>
      <p class="desc">How the text gets from us to your cursor.</p>
      <div class="tiles">${tiles}</div>
    </section>`;
}

/// Core ▸ Engine: where it runs and what language it listens for. One click
/// swaps CPU ⇄ GPU (the model reloads in the background; dictation keeps
/// working on the old one until the new one is warm).
function engineCard() {
  const c = cfg(), m = S.snap.model;
  const where = m.state === 'ready' ? `<b>${esc(DEV_LONG[m.device] || m.device)}</b> · ${m.load_ms} ms load` : esc(m.state);
  const segs = devices().map((d) => devBtn(d, c.device === d.device)).join('');
  const langs = LANGS.map(([k, n]) => `<option value="${k}" ${c.language === k ? 'selected' : ''}>${n}</option>`).join('');
  return `<section class="card">
    <div class="row" style="flex-wrap:nowrap">
      <div style="flex:1;min-width:0">
        <h2 style="margin:0">Engine</h2>
        <p class="desc" style="margin:2px 0 0">${esc(activeModelName())} on ${where}</p>
      </div>
      <div class="seg" aria-label="device">${segs}</div>
      <span title="${esc(langHint())}">${select('language', langs, 'width:150px')}</span>
    </div>
    ${when(m.state === 'ready' && m.note, engineNote)}
  </section>`;
}

/// ⓘ panel: the model's papers, so "should I pick this one?" has an answer.
function modelInfo(m) {
  const f = m.facts;
  const row = (k, v) => when(v, () => `<dt>${k}</dt><dd>${esc(v)}</dd>`);
  return `<div class="model-info">
    <dl class="facts">
      ${row('made by', f.by)}${row('released', f.released)}
      ${row('size', `${f.params} parameters · ${fmtBytes(m.size)} download`)}
      ${row('architecture', f.arch)}${row('license', f.license)}${row('languages', f.languages)}
      ${row('published', f.published)}${row('our race', f.ours)}
    </dl>
    <div class="verdict">
      <div class="v-pick"><b>pick it if</b>${esc(f.pick)}</div>
      <div class="v-skip"><b>skip it if</b>${esc(f.skip)}</div>
    </div>
    <div class="info-foot"><a class="link" data-act="url" data-url="${esc(f.card)}">model card ↗</a><span class="dim">ONNX export: ${esc(m.source)}</span></div>
  </div>`;
}

function modelCard(m) {
  const s = S.snap;
  const active = s.active_model === m.id;
  const dl = s.download && s.download.id === m.id ? s.download : null;
  const busy = dl && !dl.error;
  const confirming = S.confirm === `del:${m.id}`;
  const partial = !m.installed && m.on_disk > 0;

  const actions = active
    ? `<span class="pill">active</span>`
    : busy ? `<button class="btn ghost" data-act="model_cancel">Cancel</button>`
    : m.installed
      ? `<button class="btn ${confirming ? 'danger' : 'ghost'}" data-act="model_delete" data-id="${m.id}">${I.trash}${confirming ? 'Sure?' : 'Delete'}</button>
         <button class="btn primary" data-act="model_use" data-id="${m.id}">Use</button>`
      : `<button class="btn primary" data-act="model_download" data-id="${m.id}">${I.dl}${dl?.error ? 'Retry' : partial ? 'Resume' : 'Download'}</button>`;

  const pct = dl ? Math.min(100, (dl.done / Math.max(dl.total, 1)) * 100) : 0;
  return `<div class="model ${active ? 'active' : ''}" data-model="${m.id}">
    <div class="model-top">
      <div class="model-id">
        <div class="model-name">${esc(m.name)}<span class="chip">${I.globe}${esc(m.langs)}</span>
          ${engChip(m.engine)}
          <span class="size">${fmtBytes(m.size)}${when(partial && !busy, ` · ${fmtBytes(m.on_disk)} here`)}</span></div>
        <div class="model-blurb" title="${esc(m.blurb)} — ONNX export: ${esc(m.source)}">${esc(m.blurb)}</div>
      </div>
      <div class="meters">${meter('accuracy', m.accuracy)}${meter('speed', m.speed)}</div>
      <div class="model-acts"><button class="btn ghost info-btn ${S.info === m.id ? 'on' : ''}" data-act="model_info" data-id="${m.id}" title="Who made it, license, published WER, when to pick it" aria-label="details">ⓘ</button>${actions}</div>
    </div>
    ${when(S.info === m.id, () => modelInfo(m))}
    ${when(busy, () => `<div class="progress"><div class="track"><div class="fill" style="width:${pct}%"></div></div>
      <div class="nums"><span class="dl-pct">${pct.toFixed(1)}%</span><span class="dl-bytes">${fmtBytes(dl.done)} / ${fmtBytes(dl.total)}</span></div></div>`)}
    ${when(dl?.error, () => `<div class="err-line">✕ ${esc(dl.error)}</div>`)}
  </div>`;
}

function viewModels() {
  const s = S.snap;
  const installed = s.models.filter((m) => m.installed);
  const available = s.models.filter((m) => !m.installed);
  const custom = !s.active_model;
  const ms = modelState();
  return `${head('Models', 'All local, CPU or GPU. Nothing you say leaves this machine.')}
    <div class="status-line"><span class="dot ${ms.cls}"></span>${esc(activeModelName())} — ${esc(ms.text)}</div>
    ${when(s.model.error, () => `<div class="err-line" style="padding:8px 0 0">${esc(s.model.error)}</div>`)}
    ${when(custom, () => `<div class="section-label">active · custom folder</div>
      <div class="model active"><div class="model-top">
        <div class="model-id"><div class="model-name">${esc(activeModelName())}<span class="chip">hand-picked</span></div>
          <div class="model-blurb mono">${esc(cfg().model_dir)}</div></div>
        <span></span>
        <div class="model-acts"><button class="btn ghost" data-act="open" data-what="model_dir">${I.folder}Open</button><span class="pill">active</span></div>
      </div></div>`)}
    ${when(installed.length, () => `<div class="section-label">downloaded</div>${installed.map(modelCard).join('')}`)}
    ${when(available.length, () => `<div class="section-label">available to download</div>${available.map(modelCard).join('')}`)}
    <div class="section-label">bring your own · any Parakeet, Nemotron or Cohere ONNX folder</div>
    <div class="row"><input type="text" id="custom-dir" placeholder="C:\\path\\to\\model  (ONNX export: encoder + decoder + vocab/tokenizer)" value="${esc(custom ? cfg().model_dir : '')}" style="flex:1">
      <button class="btn" data-act="model_dir">Use folder</button>
      <button class="btn ghost" data-act="open" data-what="models">${I.folder}Downloads</button></div>`;
}

function viewAudio() {
  const c = cfg();
  const pinned = c.input_device;
  const known = !pinned || S.mics.some((m) => m.toLowerCase().includes(pinned.toLowerCase()));
  const opts = [`<option value="" ${!pinned ? 'selected' : ''}>System default</option>`]
    .concat(S.mics.map((m) => `<option value="${esc(m)}" ${pinned === m ? 'selected' : ''}>${esc(m)}</option>`))
    .concat(pinned && !S.mics.includes(pinned) ? [`<option value="${esc(pinned)}" selected>${esc(pinned)}${known ? '' : ' (not connected)'}</option>`] : [])
    .join('');
  return `${head('Audio', 'Which mic, and how long a clip must be before we bother.')}
    <section class="card">
      <h2>Microphone</h2>
      <p class="desc">Pinned by name, so Windows reshuffling its default device can't hijack you. Unplugged? We fall back to the default.</p>
      <div class="row">${select('input_device', opts, 'flex:1')}
        <button class="btn ghost" data-act="mics">${I.refresh}Rescan</button></div>
    </section>
    <section class="card">
      <h2>Clip limits</h2>
      <p class="desc">Shorter than the minimum is a fat-fingered shortcut, not speech.</p>
      <div class="grid2">
        ${num('min_ms', c.min_ms, 'ms', 'discard anything shorter', 0, 5000)}
        ${num('max_ms', c.max_ms, 'ms', 'hard stop, even mid-sentence', 1000, 600000)}
      </div>
    </section>`;
}

function viewHistory() {
  const c = cfg();
  const items = S.snap.history;
  const list = items.length
    ? `<div class="hist">${items.map((e) => `<div class="h-item">
        <div class="h-when">${clock(e.id)}<br>${day(e.id)} · ${ago(e.id)}</div>
        <div><div class="h-text">${esc(e.text)}</div>
          <div class="h-meta">${(e.audio_ms / 1000).toFixed(1)}s audio → ${e.asr_ms} ms</div></div>
        <div class="h-acts">
          <button class="btn ghost" title="Copy" data-act="copy" data-id="${e.id}">${I.copy}</button>
          <button class="btn ghost danger" title="Delete" data-act="history_delete" data-id="${e.id}">${I.trash}</button>
        </div></div>`).join('')}</div>`
    : `<div class="empty">${c.history ? 'Nothing yet. Say something.' : 'History is off. Nothing is being kept.'}</div>`;
  return `${head('History', 'Recent transcripts, on this machine only. Off by default.')}
    <section class="card">
      <div class="row">${sw('history', c.history, 'Keep history')}
        <span class="spacer"></span>
        ${c.history ? num('history_max', c.history_max, 'entries', '', 1, 10000) : ''}
      </div>
    </section>
    <div class="section-label">${items.length} entr${items.length === 1 ? 'y' : 'ies'}
      ${when(items.length, `<button class="btn ${S.confirm === 'clear' ? 'danger' : 'ghost'}" data-act="history_clear">${S.confirm === 'clear' ? 'Really wipe?' : 'Clear all'}</button>`)}</div>
    ${list}`;
}

// ── perf ─────────────────────────────────────────────────────────────────

const f1 = (x) => (Number.isFinite(x) ? x.toFixed(x >= 100 ? 0 : 1) : '—');
const xrt = (e) => e.audio_ms / Math.max(e.asr_ms, 0.1);
const wps = (e) => (e.words ? e.words / (e.asr_ms / 1000) : NaN);

const tile = (label, value, unit, sub = '', hero = false) => `
  <div class="stat ${hero ? 'hero-stat' : ''}">
    <div class="label">${label}</div>
    <div class="stat-v">${value}<span class="stat-u">${unit}</span></div>
    ${when(sub, `<div class="stat-sub">${sub}</div>`)}
  </div>`;

/// Total latency per run, oldest → newest. One series, one colour: the title
/// names it, so no legend box. Cold runs carry a hatch AND a label — never
/// colour alone. p50 is the one direct label on the plot.
function perfChart(entries, p50) {
  const runs = [...entries].reverse();
  const W = 840, H = 160, L = 46, R = 10, T = 14, B = 24;
  const pw = W - L - R, ph = H - T - B;
  const max = Math.max(...runs.map((e) => e.total_ms), 1);
  // Nice ceiling: 1/2/5 × 10^k so gridlines land on round numbers.
  const mag = 10 ** Math.floor(Math.log10(max));
  const top = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((v) => v >= max);
  const y = (v) => T + ph - (v / top) * ph;
  const slot = pw / Math.max(runs.length, 16);
  const bw = Math.max(2, slot - 2); // 2px surface gap between neighbours
  const grid = [0, 0.25, 0.5, 0.75, 1].map((f) => {
    const v = top * f, yy = y(v);
    return `<line x1="${L}" x2="${W - R}" y1="${yy}" y2="${yy}" class="grid"/>
      <text x="${L - 8}" y="${yy + 3.5}" class="tick" text-anchor="end">${f1(v)}</text>`;
  }).join('');
  // Bar with a 4px rounded data-end, square at the baseline.
  const bar = (x, y0, h) => {
    const r = Math.min(4, bw / 2, h);
    return `M${x},${y0 + h}V${y0 + r}Q${x},${y0} ${x + r},${y0}H${x + bw - r}Q${x + bw},${y0} ${x + bw},${y0 + r}V${y0 + h}Z`;
  };
  const bars = runs.map((e, i) => {
    const x = L + i * slot + 1, yy = y(e.total_ms), h = Math.max(1, T + ph - yy);
    return `<g data-run="${e.id}">
      <path d="${bar(x, yy, h)}" class="${e.cold ? 'b cold' : 'b'}"/>
      ${when(e.cold, `<text x="${x + bw / 2}" y="${yy - 5}" class="cold-l" text-anchor="middle">cold</text>`)}
      <rect x="${x - 1}" y="${T}" width="${slot}" height="${ph}" class="hit"/>
    </g>`;
  }).join('');
  const py = y(p50);
  return `<svg viewBox="0 0 ${W} ${H}" class="chart" role="img" aria-label="Total latency per run, oldest to newest">
    <defs><pattern id="hatch" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
      <rect width="5" height="5" class="hatch-bg"/><line x1="0" y1="0" x2="0" y2="5" class="hatch-ln"/></pattern></defs>
    ${grid}
    <text x="${L}" y="${H - 6}" class="tick">oldest</text>
    <text x="${W - R}" y="${H - 6}" class="tick" text-anchor="end">newest →</text>
    ${bars}
    <line x1="${L}" x2="${W - R}" y1="${py}" y2="${py}" class="p50"/>
    <text x="${W - R - 4}" y="${py - 6}" class="p50-l" text-anchor="end">p50 ${f1(p50)} ms</text>
  </svg>`;
}

function viewPerf() {
  const { summary: s, entries } = S.snap.perf;
  const m = S.snap.model;
  const loadInfo = m.state === 'ready'
    ? `model load <b>${m.load_ms} ms</b> + warm-up <b>${m.warmup_ms} ms</b> — paid at startup so your first sentence doesn't`
    : `model ${esc(m.state)}`;
  if (!entries.length) {
    return `${head('Perf', 'Release → text delivered. Numbers only, never your words.')}
      <div class="status-line" style="margin-bottom:12px">${loadInfo}</div>
      <div class="empty">No runs yet. Hold ${keycaps(cfg().hotkey, 'small')} and say something. We'll take it from here.</div>`;
  }
  const med = (k) => { const v = entries.map((e) => e[k]).sort((a, b) => a - b); return v[Math.round((v.length - 1) / 2)]; };
  const tax = s.cold_tax_ms;
  const rows = entries.map((e) => `<tr>
    <td>${clock(e.id)}</td><td>${(e.audio_ms / 1000).toFixed(1)}s</td><td>${e.words}</td>
    <td class="hl">${f1(e.total_ms)}</td><td>${f1(e.asr_ms)}</td><td>${f1(e.inject_ms)}</td>
    <td>${f1(xrt(e))}×</td><td>${f1(wps(e))}</td>
    <td class="tl">${esc(e.mode)}</td><td class="tl">${when(e.cold, '<span class="pill warn">cold</span>')}</td></tr>`).join('');

  return `${head('Perf', 'Release → text delivered. Numbers only, never your words.')}
    <div class="status-line" style="margin-bottom:12px">${loadInfo}</div>
    <div class="stats">
      ${tile('latency p50', f1(s.p50_total_ms), 'ms', 'release → on screen', true)}
      ${tile('latency p95', f1(s.p95_total_ms), 'ms', 'the bad day')}
      ${tile('speed p50', f1(s.p50_xrt), '×rt', `asr p50 ${f1(s.p50_asr_ms)} ms`)}
      ${tile('throughput', f1(s.p50_wps), 'w/s', 'words per compute-sec')}
      ${tile('your pace', f1(s.p50_wpm), 'wpm', 'words per minute')}
      ${tile('cold tax', tax == null ? '—' : `${tax >= 0 ? '+' : ''}${f1(tax)}`, tax == null ? '' : 'ms',
             tax == null ? 'needs cold + warm' : '1st clip vs rest')}
    </div>
    <section class="card">
      <h2>Latency per run <span class="dim mono" style="font-size:12px;font-weight:400">${entries.length} / ${cfg().perf_max}</span></h2>
      <p class="desc" style="margin-bottom:6px">Median split: resample ${f1(med('resample_ms'))} · queue ${f1(med('queue_ms'))} · asr ${f1(med('asr_ms'))} · deliver ${f1(med('inject_ms'))} ms. Hover a bar for its breakdown.</p>
      <div class="chart-wrap">${perfChart(entries, s.p50_total_ms)}<div class="tip" id="tip"></div></div>
    </section>
    <div class="section-label">runs · newest first
      <span class="spacer"></span>
      ${num('perf_max', cfg().perf_max, 'kept', '', 8, 4096)}
      <button class="btn ${S.confirm === 'perf' ? 'danger' : 'ghost'}" data-act="perf_clear">${S.confirm === 'perf' ? 'Really wipe?' : 'Clear'}</button>
    </div>
    <div class="table-wrap"><table class="runs">
      <thead><tr><th>time</th><th>audio</th><th>words</th><th>total ms</th><th>asr ms</th><th>deliver ms</th><th>speed</th><th>words/s</th><th class="tl">mode</th><th class="tl"></th></tr></thead>
      <tbody>${rows}</tbody></table></div>`;
}

/// Hover tooltip for chart bars. One delegated listener; the hit rects are
/// bigger than the bars so you don't need a sniper's hand.
document.addEventListener('mouseover', (e) => {
  const g = e.target.closest?.('[data-run]');
  const tip = $('#tip');
  if (!tip) return;
  if (!g) { tip.classList.remove('on'); return; }
  const r = S.snap.perf.entries.find((x) => String(x.id) === g.dataset.run);
  if (!r) return;
  tip.innerHTML = `<div class="tip-h">${clock(r.id)} · ${day(r.id)}${when(r.cold, ' · <span class="pill warn">cold</span>')}</div>
    <div class="tip-g">
      <span>total</span><b>${f1(r.total_ms)} ms</b>
      <span>resample</span><i>${f1(r.resample_ms)} ms</i>
      <span>queue</span><i>${f1(r.queue_ms)} ms</i>
      <span>asr</span><i>${f1(r.asr_ms)} ms</i>
      <span>deliver</span><i>${f1(r.inject_ms)} ms</i>
      <span>audio</span><i>${(r.audio_ms / 1000).toFixed(2)} s · ${r.words} words</i>
      <span>speed</span><i>${f1(xrt(r))}× · ${f1(wps(r))} w/s</i>
      <span>via</span><i>${esc(r.mode)} · ${esc(r.model)} · ${r.threads}t</i>
    </div>`;
  const wrap = $('.chart-wrap').getBoundingClientRect(), b = g.getBoundingClientRect();
  const left = Math.min(Math.max(b.left - wrap.left + b.width / 2 - 120, 0), wrap.width - 240);
  tip.style.left = `${left}px`;
  tip.style.top = `${Math.max(b.top - wrap.top - tip.offsetHeight - 8, 0)}px`;
  tip.classList.add('on');
});

// ── benchmark ────────────────────────────────────────────────────────────
//
// Pick a clip → ▶ Run → every installed model hears the same tape (active
// model first) → its verdict shimmers into the terminal → the board ranks the
// rest. Clips come from Rust (bench/ baked into the exe, or your own in
// %APPDATA%\DivineVoice\bench). "Read it yourself" is the optional human mode:
// arm, read the clip's script with the hotkey, get graded, race your take.
//
// All state lives in S.bench; the reveal is a timer that only ever touches
// `.w[data-i]` spans, so a re-render mid-reveal just picks up where it left off.

const READ_WPS = 2.6; // comfortable reading pace, for the read-along ETA

// Per-viewer conveniences (sound, play-along). localStorage can vanish; never trust it.
const pref = {
  get: (k, d) => { try { const v = localStorage.getItem(k); return v === null ? d : JSON.parse(v); } catch (_) { return d; } },
  set: (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch (_) { /* fine */ } },
};

S.bench = {
  clip: null,          // selected clip id
  armed: false,        // read-it-yourself mode
  take: null,          // last bench_take payload (read mode)
  view: 'take',        // what the terminal shows: 'take' or a race contender id
  revealed: 0, timer: null,
  recStart: null, raf: null,
  race: null,          // { running, audio_ms, lineup:[...], rows:{id:{...}}, first }
  audio: null,         // play-along <audio>
  models: pref.get('dv.bmodels', 'active'),  // 'active' | 'all' | a catalogue id
  devs: pref.get('dv.bdevs', null),          // devices to race on; null = whatever dictation uses
};
S.sfx = pref.get('dv.sfx', true);
S.play = pref.get('dv.play', false);

/// Synth SFX. Oscillators only, no audio files. The AudioContext is created
/// on a click, because browsers don't allow sound before a user gesture.
const SFX = (() => {
  let ctx = null;
  const ac = () => (ctx ||= new (window.AudioContext || window.webkitAudioContext)());
  const tone = (f, at, dur, type = 'square', gain = 0.03) => {
    if (!S.sfx || !ctx) return;
    const t = ctx.currentTime + at, o = ctx.createOscillator(), v = ctx.createGain();
    o.type = type;
    o.frequency.setValueAtTime(f, t);
    v.gain.setValueAtTime(0.0001, t);
    v.gain.exponentialRampToValueAtTime(gain, t + 0.005);
    v.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    o.connect(v).connect(ctx.destination);
    o.start(t); o.stop(t + dur + 0.02);
  };
  const seq = (notes, step, dur, type, gain) => notes.forEach((f, i) => tone(f, i * step, dur, type, gain));
  return {
    unlock: () => { try { ac().resume(); } catch (_) { /* no audio device: stay silent */ } },
    tick:   () => tone(2200 + Math.random() * 500, 0, 0.016, 'square', 0.010),
    miss:   () => tone(150, 0, 0.09, 'sawtooth', 0.025),
    arm:    () => seq([660, 990], 0.07, 0.08, 'triangle', 0.04),
    rec:    () => tone(880, 0, 0.07, 'sine', 0.05),
    good:   () => seq([523, 659, 784, 1047], 0.09, 0.16, 'triangle', 0.05),
    meh:    () => seq([392, 330, 262], 0.11, 0.18, 'triangle', 0.045),
    step:   () => tone(1320, 0, 0.03, 'triangle', 0.02),
    win:    () => seq([784, 988, 1175, 1568, 1568], 0.08, 0.15, 'square', 0.025),
  };
})();

const grade = (wer) => wer === 0 ? 'S' : wer < 0.02 ? 'A' : wer < 0.05 ? 'B' : wer < 0.10 ? 'C' : wer < 0.20 ? 'D' : 'F';
const pct = (x) => `${(x * 100).toFixed(x < 0.1 ? 1 : 0)}`;
const clips = () => S.snap.clips || [];
const clip = () => clips().find((c) => c.id === S.bench.clip) || clips()[0] || null;

/// What a race covers: one model or all of them, on one device or several.
/// Stale picks (model deleted, GPU gone) quietly fall back instead of erroring.
const installed = () => S.snap.models.filter((m) => m.installed);
const benchModels = () => {
  const m = S.bench.models;
  return m === 'all' || installed().some((x) => x.id === m && x.id !== S.snap.active_model) ? m : 'active';
};
const benchDevs = () => {
  const d = (S.bench.devs || [cfg().device]).filter(devOk);
  return d.length ? d : ['cpu'];
};
const raceScope = () => {
  const m = benchModels();
  return { models: m === 'all' ? 'all' : m === 'active' ? undefined : [m], devices: benchDevs() };
};
/// "4 models × 2 devices", from what's actually on the board.
const lineupSummary = (lineup) => {
  const n = (k) => new Set(lineup.map((x) => x[k])).size;
  const m = n('model'), d = n('device');
  return `${m} model${m === 1 ? '' : 's'}${d > 1 ? ` × ${d} devices` : ` on ${DEV[lineup[0]?.device] || '?'}`}`;
};

function benchSetup() {
  const sel = benchModels(), others = installed().filter((x) => x.id !== S.snap.active_model);
  const opts = [`<option value="active" ${sel === 'active' ? 'selected' : ''}>${esc(activeModelName())} · active</option>`,
    ...others.map((x) => `<option value="${esc(x.id)}" ${sel === x.id ? 'selected' : ''}>${esc(x.name)}</option>`),
    ...(others.length ? [`<option value="all" ${sel === 'all' ? 'selected' : ''}>All installed (${others.length + 1})</option>`] : [])];
  const on = benchDevs();
  const chips = devices().map((d) => `<button class="${on.includes(d.device) ? 'on' : ''}" ${d.ok ? `data-act="bench_dev" data-dev="${d.device}"` : 'disabled'}
      title="${esc(d.ok ? `race on ${DEV_LONG[d.device]}` : `${DEV_LONG[d.device]} unavailable: ${d.reason}`)}">${DEV[d.device]}</button>`).join('');
  return `<span class="select"><select data-bmodels aria-label="models to race">${opts.join('')}</select></span>
    <span class="seg devs" aria-label="devices to race on">${chips}</span>`;
}

/// The result on screen, normalised: a read-along take or a race row.
function result() {
  const b = S.bench;
  if (b.view === 'take') {
    const t = b.take;
    return t && t.score ? { score: t.score, ms: t.asr_ms, audio_ms: t.audio_ms, model: t.model,
      sub: `latency ${f1(t.latency_ms)} ms`, pace: t.wpm, paceLabel: 'your pace' } : null;
  }
  const r = b.race?.rows[b.view];
  if (!r?.score) return null;
  const audio = b.race.audio_ms, words = clip()?.words || r.score.ref_words;
  return { score: r.score, ms: r.ms, audio_ms: audio, model: r.name, device: r.device,
    sub: `${DEV[r.device] || r.device} · load ${f1(r.load_ms)} ms`, pace: words / (audio / 60000), paceLabel: 'voice pace' };
}

const opSpan = (o, i) => {
  const word = o.op === 'del' ? o.r : (o.h ?? o.w);
  if (!word) return '';
  const tip = o.op === 'sub' ? ` title="expected: ${esc(o.r)}"` : o.op === 'del' ? ' title="missed"' : o.op === 'ins' ? ' title="not in the script"' : '';
  // Already-revealed words render settled (`done`) so a re-render mid-reveal
  // doesn't re-shimmer the whole paragraph.
  return `<span class="w ${o.op} ${i < S.bench.revealed ? 'on done' : ''}" data-i="${i}"${tip}>${esc(word)}</span>`;
};

function termBody() {
  const ops = result()?.score.ops;
  if (ops) {
    const done = S.bench.revealed >= ops.length;
    return ops.map(opSpan).join(' ') + (done ? '' : ' <span class="caret">▌</span>');
  }
  const c = clip();
  if (!c) return `<span class="pw pre">No benchmark clips yet.\n\nDrop a pair into %APPDATA%\\DivineVoice\\bench (or bench/ in the repo and rebuild):\n  classic.mp3  — the audio\n  classic.txt  — exactly what it says, numbers as words\n\nThen come back here.</span>`;
  return c.script.split(/\s+/).map((w) => `<span class="pw">${esc(w)}</span>`).join(' ');
}

function termTitle() {
  const r = result(), c = clip();
  if (r) return `~/divinevoice $ judge --model "${esc(r.model)}"${r.device ? ` --device ${esc(r.device)}` : ''}`;
  return c ? `~/divinevoice $ bench --clip ${esc(c.id)}` : '~/divinevoice $ ls bench/';
}

function benchStatus() {
  const b = S.bench, mode = S.snap.status.mode, hk = keycaps(cfg().hotkey, 'small'), c = clip();
  if (!c) return '<span class="dim">Waiting for clips.</span>';
  const race = b.race;
  if (race?.running) {
    const done = Object.values(race.rows).filter((x) => x.score || x.error).length;
    return `<span class="rec-dot"></span><b class="mono">RACING ${done}/${race.lineup.length}</b>
      <span class="dim">same tape · ${lineupSummary(race.lineup)}${S.play ? ' · playing along' : ''}</span><span class="spacer"></span>
      <button class="btn ghost" data-act="race_cancel">Stop</button>`;
  }
  if (b.armed) {
    const live = {
      recording:    `<span class="rec-dot"></span><b class="mono">REC <span id="rec-t">00.0</span>s</b><span class="dim">read the script out loud</span>`,
      transcribing: `<span class="dot warn"></span><b class="mono">JUDGING…</b><span class="dim">Parakeet is listening back</span>`,
    }[mode] || `<span class="dot rec"></span><b class="mono">ARMED</b><span class="dim">${cfg().hotkey_mode === 'toggle' ? 'Tap' : 'Hold'} ${hk} and read. Dictation comes here, not to your cursor.</span>`;
    return `${live}<span class="spacer"></span>
      ${when(b.take?.score, () => `${benchSetup()}<button class="btn" data-act="race">${I.models}Race my take</button>`)}
      <button class="btn ghost" data-act="bench_disarm">Done reading</button>`;
  }
  return `${benchSetup()}
    <button class="btn primary" data-act="race_clip">▶ ${race ? 'Run again' : 'Run'}</button>
    <span class="spacer"></span>
    <label class="switch" title="hear the clip while the models chew on it"><input type="checkbox" data-act="play" ${S.play ? 'checked' : ''}><i></i><span class="dim">play along</span></label>
    <a class="link mono" data-act="bench_arm" style="font-size:12px" title="arm the hotkey and read the script yourself">or read it →</a>`;
}

function benchScores() {
  const r = result();
  if (!r) return '';
  const s = r.score, g = grade(s.wer);
  const done = S.bench.revealed >= s.ops.length;
  return `<div class="stats bstats ${done ? 'in' : ''}">
    ${tile('accuracy', pct(1 - Math.min(s.wer, 1)), '%', `${s.ref_words - s.errors}/${s.ref_words} words`, true)}
    ${tile('wer', pct(s.wer), '%', `${s.errors} error${s.errors === 1 ? '' : 's'}`)}
    ${tile('asr', f1(r.ms), 'ms', r.sub)}
    ${tile('speed', f1(r.audio_ms / Math.max(r.ms, 0.1)), '×rt', `${(r.audio_ms / 1000).toFixed(1)}s of audio`)}
    ${tile(r.paceLabel, f1(r.pace), 'wpm', 'words per minute')}
    <div class="stat grade g-${g}"><div class="label">grade</div><div class="grade-l">${g}</div></div>
  </div>`;
}

function raceBoard() {
  const r = S.bench.race;
  if (!r) return '';
  const rows = r.lineup.map((c) => ({ ...c, ...(r.rows[c.id] || {}) }));
  const best = rows.filter((x) => x.score).sort((a, b) => a.score.wer - b.score.wer || a.ms - b.ms)[0];
  const order = r.running ? rows : rows.slice().sort((a, b) =>
    (a.score ? 0 : 1) - (b.score ? 0 : 1) || (a.score?.wer ?? 9) - (b.score?.wer ?? 9) || (a.ms ?? 9e9) - (b.ms ?? 9e9));
  // Same model on the CPU, if it raced too: the number people actually want.
  const vsCpu = (x) => {
    const cpu = x.device !== 'cpu' && rows.find((y) => y.model === x.model && y.device === 'cpu' && y.ms);
    return cpu ? `<span class="b-vs ${cpu.ms > x.ms ? 'up' : 'down'}">${f1(cpu.ms / x.ms)}× vs CPU</span>` : '<span class="dim">done</span>';
  };
  const phase = (x) => x.error ? `<span class="b-err" title="${esc(x.error)}">✕ ${esc(gist(x.error))}</span>`
    : x.score ? vsCpu(x)
    : x.phase ? `<span class="b-bar"><i style="width:${{ load: 15, warm: 30 }[x.phase] ?? (30 + 70 * (x.run - 1) / x.of)}%"></i></span><span class="mono dim">${x.phase === 'run' ? `run ${x.run}/${x.of}` : x.phase}</span>`
    : '<span class="dim">queued</span>';
  return `<div class="section-label">race · same tape · ${lineupSummary(r.lineup)} ${when(!r.running && best, '· click a row to see what it heard')}</div>
    <div class="board">${order.map((x, i) => `
      <div class="b-row ${best && x.id === best.id && !r.running ? 'best' : ''} ${S.bench.view === x.id ? 'viewing' : ''}" ${x.score ? `data-act="bench_view" data-id="${esc(x.id)}"` : ''}>
        <span class="b-rank">${r.running ? '·' : x.score ? `#${i + 1}` : '–'}</span>
        <span class="b-name">${esc(x.name)} ${engChip(x.engine)}<span class="chip dev ${x.device === 'cpu' ? '' : 'gpu'}">${DEV[x.device] || x.device}</span></span>
        <span class="b-phase">${phase(x)}</span>
        <span class="b-num">${x.ms ? `${f1(x.ms)} ms` : ''}</span>
        <span class="b-num">${x.xrt ? `${f1(x.xrt)}×` : ''}</span>
        <span class="b-num b-wer">${x.score ? `${pct(x.score.wer)}%` : ''}</span>
      </div>`).join('')}</div>`;
}

function viewBench() {
  const b = S.bench, c = clip();
  if (c && !b.clip) b.clip = c.id;
  const tabs = clips().map((x) =>
    `<button class="${c && c.id === x.id ? 'on' : ''}" data-act="bench_pick" data-id="${esc(x.id)}" title="${x.origin === 'user' ? 'your clip' : 'built in'}">${esc(x.title)}</button>`).join('');
  return `${head('Benchmark', 'Same tape, any model, CPU or GPU. Accuracy, speed, a grade, a winner.')}
    <div class="row" style="margin-bottom:10px">
      ${when(tabs, `<div class="seg">${tabs}</div>`)}
      ${when(c, () => `<span class="dim mono" style="font-size:12px">${c.words} words${c.origin === 'user' ? ' · your clip' : ''}</span>`)}
      <span class="spacer"></span>
      <button class="btn ghost" data-act="sfx" title="sound effects">${S.sfx ? I.sound : I.mute}${S.sfx ? 'sfx on' : 'sfx off'}</button>
    </div>
    <div class="term ${b.armed ? 'armed' : ''}">
      <div class="term-bar"><i></i><i></i><i></i><span id="term-title">${termTitle()}</span>
        ${when(b.view !== 'take' && b.take, '<button class="term-back" data-act="bench_view" data-id="take">← your take</button>')}</div>
      <div class="term-body" id="term-body">${termBody()}</div>
      <div class="term-prog ${b.race?.running && !b.race.first ? 'scan' : ''}" id="term-prog"><i id="rec-bar"></i></div>
    </div>
    <div class="row bstat" id="bstat">${benchStatus()}</div>
    ${when(b.take?.error, () => `<div class="err-line" style="padding:6px 0">✕ ${esc(b.take.error)}</div>`)}
    <div id="bscores">${benchScores()}</div>
    <div id="board">${raceBoard()}</div>`;
}

// Targeted refreshers: the reveal timer and race events must not rebuild the
// whole page (it would restart animations and steal focus).
const benchPart = (id, html) => { const el = document.getElementById(id); if (el) el.innerHTML = html; };
const refreshBench = () => {
  if (S.page !== 'bench') return;
  benchPart('bstat', benchStatus());
  benchPart('bscores', benchScores());
  benchPart('board', raceBoard());
  const tt = document.getElementById('term-title'); if (tt) tt.innerHTML = termTitle();
  document.getElementById('term-prog')?.classList.toggle('scan', !!(S.bench.race?.running && !S.bench.race.first));
};

/// Reveal the current result word by word: shimmer + tick per word, a low
/// blip per miss, and a verdict jingle at the end.
function startReveal() {
  const b = S.bench;
  clearInterval(b.timer);
  b.revealed = 0;
  benchPart('term-body', termBody());
  const tt = document.getElementById('term-title'); if (tt) tt.innerHTML = termTitle();
  const ops = result()?.score.ops || [];
  const per = Math.max(14, Math.min(45, 2800 / Math.max(ops.length, 1)));
  b.timer = setInterval(() => {
    const i = b.revealed;
    if (i >= ops.length) {
      clearInterval(b.timer);
      document.querySelector('#term-body .caret')?.remove();
      const s = result()?.score;
      if (s) (['S', 'A'].includes(grade(s.wer)) ? SFX.good : SFX.meh)();
      refreshBench();
      return;
    }
    const el = document.querySelector(`#term-body .w[data-i="${i}"]`);
    if (el) {
      el.classList.add('on');
      const caret = document.querySelector('#term-body .caret');
      if (caret) el.after(caret);
      el.scrollIntoView({ block: 'nearest' });
      (ops[i].op === 'ok' ? SFX.tick : SFX.miss)();
    }
    b.revealed = i + 1;
  }, per);
}

/// Progress bar: read-along clock vs. the script's ETA, or the play-along
/// audio's position.
function progressLoop() {
  const b = S.bench;
  cancelAnimationFrame(b.raf);
  const eta = (clip()?.words || 100) / READ_WPS;
  const frame = () => {
    const bar = document.getElementById('rec-bar');
    if (b.recStart != null) {
      const s = (performance.now() - b.recStart) / 1000;
      const t = document.getElementById('rec-t'); if (t) t.textContent = s.toFixed(1).padStart(4, '0');
      if (bar) { bar.style.width = `${Math.min(100, (s / eta) * 100)}%`; bar.classList.toggle('over', s > eta); }
    } else if (b.audio && !b.audio.paused && b.audio.duration) {
      if (bar) bar.style.width = `${(b.audio.currentTime / b.audio.duration) * 100}%`;
    } else {
      return; // nothing to animate; next start() re-arms the loop
    }
    b.raf = requestAnimationFrame(frame);
  };
  b.raf = requestAnimationFrame(frame);
}

const stopAudio = () => { const a = S.bench.audio; if (a) { a.pause(); S.bench.audio = null; } };

const benchDisarm = () => {
  stopAudio();
  if (!S.bench.armed) return;
  S.bench.armed = false;
  S.bench.recStart = null;
  ipc('bench_disarm');
};

// A hidden page must never keep eating dictation (Rust disarms too; belt + braces).
document.addEventListener('visibilitychange', () => { if (document.hidden) { benchDisarm(); refreshBench(); } });

/// Under the device picker: where it ACTUALLY runs if that's not where you
/// asked, else why the greyed-out GPUs are grey.
function engineNote() {
  const m = S.snap.model;
  if (m.state === 'ready' && m.note) return `<div class="dev-note warn" title="${esc(m.note)}">⚠ ${esc(gist(m.note))}</div>`;
  const bad = devices().filter((d) => !d.ok);
  if (!bad.length) return '<div class="dev-note">GPU or CPU: same model, same accuracy. Changing either reloads it.</div>';
  return bad.map((d) => `<div class="dev-note" title="${esc(d.reason)}"><b>${DEV[d.device]}</b> ${esc(gist(d.reason))}</div>`).join('');
}

function viewAdvanced() {
  const c = cfg();
  const fb = c.fbdirector != null;
  return `${head('Advanced', 'Knobs for people who read the source.')}
    <div class="cols">
      <section class="card">
        <h2>Typing</h2>
        <p class="desc">Unicode/Type speed and the Paste clipboard dance.</p>
        <div class="grid2">
          ${num('type_delay_ms', c.type_delay_ms, 'ms/char', '0 = flat out · 5 = KVM-safe', 0, 100)}
          ${num('paste_settle_ms', c.paste_settle_ms, 'ms', 'before clipboard restore', 0, 2000)}
        </div>
      </section>
      <section class="card">
        <h2>Engine</h2>
        <p class="desc">CPU threads for ONNX Runtime ≈ your physical P-cores. Device and language live on <a class="link" data-act="page" data-page="general">Core</a>.</p>
        ${num('intra_threads', c.intra_threads, 'threads', '0 = ORT guesses · change reloads the model', 0, 64)}
      </section>
      <section class="card">
        <h2>Interception ${S.snap.interception ? '<span class="pill ok">driver on</span>' : '<span class="pill off">driver off</span>'}</h2>
        <p class="desc">Hardware IDs Type must never inject through, one per line (your KVM's own HID gadget).</p>
        <textarea data-key="skip_hwids" data-lines rows="3" placeholder="VID_1209&amp;PID_4B4D">${esc(c.skip_hwids.join('\n'))}</textarea>
      </section>
      <section class="card">
        <h2>fbdirector bus</h2>
        <p class="desc">Claim the shortcut so fbdirector doesn't forward it to the remote box.</p>
        <div class="row">${sw('fbdirector', fb, 'Enabled')}
          ${when(fb, () => `${S.snap.status.bus ? '<span class="pill ok">connected</span>' : '<span class="pill warn">waiting</span>'}
            <input type="text" data-key="fbdirector" value="${esc(c.fbdirector)}" style="flex:1;width:auto">`)}</div>
      </section>
      <section class="card wide">
        <div class="row">
          <div><h2>Logging</h2><p class="desc" style="margin:0"><span class="mono">debug</span> also logs transcript text. Applies on restart.</p></div>
          <span class="spacer"></span>
          ${select('log_level', ['error', 'warn', 'info', 'debug', 'trace']
            .map((l) => `<option ${c.log_level === l ? 'selected' : ''}>${l}</option>`).join(''), 'width:130px')}
          <button class="btn ghost" data-act="open" data-what="config">${I.folder}config.toml</button>
          <button class="btn ghost" data-act="open" data-what="logs">${I.folder}Logs</button>
        </div>
      </section>
    </div>`;
}

function viewAbout() {
  const s = S.snap;
  const link = (url, text) => `<a class="link" data-act="url" data-url="${esc(url)}">${esc(text)}</a>`;
  return `${head('About')}
    <section class="card about-hero">
      <div class="wordmark">DIVINE<b>//</b>VOICE</div>
      <p class="sec" style="max-width:560px">Local push-to-talk dictation. NVIDIA Parakeet on your CPU or GPU, text wherever your cursor is: through Parsec, through a hardware KVM, through whatever.</p>
      <div class="row"><span class="pill">v${esc(s.version)}</span><span class="chip">GPL-2.0-or-later</span><span class="chip">no cloud · no telemetry</span></div>
    </section>
    <section class="card">
      <dl class="kv">
        <dt>config</dt><dd class="mono">${esc(s.paths.config)}</dd>
        <dt>data</dt><dd class="mono">${esc(s.paths.home)}</dd>
        <dt>models</dt><dd class="mono">${esc(s.paths.models)}</dd>
        <dt>speech model</dt><dd>${link('https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3', 'NVIDIA Parakeet TDT 0.6B')} (CC-BY-4.0) via ${link('https://github.com/altunenes/parakeet-rs', 'parakeet-rs')}</dd>
        <dt>onnx exports</dt><dd>${link('https://huggingface.co/istupakov', 'istupakov on HuggingFace')}</dd>
        <dt>hid driver</dt><dd>${link('https://github.com/oblitum/Interception', 'Interception')} by oblitum, optional</dd>
      </dl>
    </section>`;
}

// ── chrome: nav + footer ─────────────────────────────────────────────────

const renderNav = () => {
  $('#nav').innerHTML = Object.entries(PAGES).map(([id, p]) =>
    `<button class="nav-item ${S.page === id ? 'on' : ''}" data-act="page" data-page="${id}">${p.icon}<span>${p.name}</span></button>`).join('');
};

const renderChrome = () => {
  if (!S.snap) return;
  const s = S.snap, c = cfg(), ms = modelState(), mode = s.status.mode;
  const dl = s.download && !s.download.error ? s.download : null;
  const p50 = s.perf?.entries.length ? f1(s.perf.summary.p50_total_ms) : null;
  const dotCls = mode === 'idle' ? 'ok' : mode === 'recording' ? 'rec' : 'warn';

  // Titlebar: where you are, what it's doing, how fast it's been doing it.
  document.body.classList.toggle('recording', mode === 'recording');
  $('#tb-crumb').textContent = `/ ${PAGES[S.page].name}`;
  if (!document.getElementById('theme-panel')) paint(c.theme);
  $('#tb-theme').innerHTML = `${swatch(theme(c.theme), 'big')}<svg viewBox="0 0 12 12" class="caret"><path d="M3 4.5l3 3 3-3"/></svg>`;
  $('#tb-status').innerHTML = `
    ${when(p50, () => `<span>p50 <b>${p50}</b> ms</span>`)}
    <span>asr <b>${s.model.state === 'ready' ? esc((DEV[s.model.device] || s.model.device).toLowerCase()) : esc(s.model.state)}</b></span>
    <span class="tb-mode ${mode}"><span class="dot ${dotCls}"></span>${esc(MODE_TEXT[mode] || mode)}</span>`;

  $('#nav-foot').innerHTML = `
    <span class="label">push-to-talk · ${esc(c.hotkey_mode)}</span>
    ${keycaps(c.hotkey, 'small')}
    <span class="label" style="margin-top:4px">output</span>
    <span class="mono sec" style="font-size:12px">${esc(c.inject_mode.replace('_', '-'))}</span>`;

  $('#bar').innerHTML = `
    <span class="item"><span class="dot ${ms.cls}"></span><b>${esc(activeModelName())}</b>${esc(ms.text)}</span>
    ${when(dl, () => `<span class="item dl-mini">${I.dl.replace('<svg', '<svg width="12" height="12"')}${Math.floor(dl.done / Math.max(dl.total, 1) * 100)}%</span>`)}
    <span class="spacer"></span>
    ${when(c.fbdirector != null, `<span class="item">fb:${s.status.bus ? 'ok' : '--'}</span>`)}
    ${when(s.interception, '<span class="item">hid:on</span>')}
    <span class="item">v${esc(s.version)}</span>`;
};

/// Re-render the page, keeping focus + caret in whatever input you were typing in.
const render = () => {
  if (!S.snap) return;
  const a = document.activeElement;
  const keep = a && a.id ? { id: a.id, s: a.selectionStart, e: a.selectionEnd } : null;
  const top = $('#page').scrollTop;
  const entering = S.page !== render.last;
  render.last = S.page;
  renderNav();
  $('#page').innerHTML = `<div class="page ${entering ? 'enter' : ''}">${PAGES[S.page].view()}</div>`;
  $('#page').scrollTop = top;
  renderChrome();
  if (keep) {
    const el = document.getElementById(keep.id);
    if (el) { el.focus(); try { el.setSelectionRange(keep.s, keep.e); } catch (_) { /* number inputs say no */ } }
  }
};

// ── toasts ───────────────────────────────────────────────────────────────

const toast = (kind, msg) => {
  const t = document.createElement('div');
  t.className = `toast ${kind}`;
  t.textContent = msg;
  $('#toasts').append(t);
  setTimeout(() => { t.classList.add('bye'); setTimeout(() => t.remove(), 300); }, kind === 'error' ? 6000 : 2600);
};

// ── Rust → us ────────────────────────────────────────────────────────────

const ON = {
  hello: (d) => { S.snap = d; S.mics = d.mics || []; render(); },
  state: (d) => { S.snap = { ...d, mics: undefined }; render(); },
  mics:  (d) => { S.mics = d; render(); toast('info', `${d.length} input device${d.length === 1 ? '' : 's'} found`); },
  status: (d) => {
    S.snap.status = d;
    const hero = $('#hero');
    if (hero) hero.outerHTML = heroHtml();
    renderChrome();
    // Benchmark: start/stop the reading clock with the hotkey.
    const b = S.bench;
    if (b.armed && d.mode === 'recording' && b.recStart == null) {
      b.recStart = performance.now();
      SFX.rec();
      if (b.view !== 'take' || b.take) { b.take = null; b.race = null; b.view = 'take'; clearInterval(b.timer); b.revealed = 0; benchPart('term-body', termBody()); }
      progressLoop();
    } else if (d.mode !== 'recording') {
      b.recStart = null;
    }
    refreshBench();
  },
  download: (d) => {
    S.snap.download = d;
    // 10 Hz: poke the bar in place instead of re-rendering the page.
    const card = d && $(`[data-model="${d.id}"]`);
    const fill = card && $('.fill', card);
    if (fill && !d.error) {
      const pct = Math.min(100, (d.done / Math.max(d.total, 1)) * 100);
      fill.style.width = `${pct}%`;
      $('.dl-pct', card).textContent = `${pct.toFixed(1)}%`;
      $('.dl-bytes', card).textContent = `${fmtBytes(d.done)} / ${fmtBytes(d.total)}`;
    } else if (S.page === 'models') render();
    renderChrome();
  },
  learned: (d) => {
    S.learning = false;
    S.learnErr = d.error ? `${d.binding}: ${d.error}` : null;
    if (d.binding && !d.error) ipc('set', { key: 'hotkey', value: d.binding });
    render();
  },
  history_add: (e) => {
    S.snap.history = [e, ...S.snap.history].slice(0, cfg().history_max);
    if (S.page === 'history') render();
  },
  toast: (d) => toast(d.kind, d.msg),

  // ── benchmark ──
  bench_take: (d) => {
    const b = S.bench;
    b.take = d; b.view = 'take'; b.race = null;
    if (S.page !== 'bench') return;
    refreshBench();
    if (d.score) startReveal();
  },
  race_start: (d) => {
    S.bench.race = { running: true, audio_ms: d.audio_ms, lineup: d.lineup, rows: {}, first: null };
    refreshBench();
  },
  race_step: (d) => {
    const r = S.bench.race; if (!r) return;
    r.rows[d.id] = { ...(r.rows[d.id] || {}), phase: d.phase, run: d.run, of: d.of };
    if (d.phase === 'load') SFX.step();
    benchPart('board', raceBoard());
  },
  race_result: (d) => {
    const r = S.bench.race; if (!r) return;
    r.rows[d.id] = { ...(r.rows[d.id] || {}), ...d };
    // Clip runs: the first verdict in (your active model — it races first)
    // goes straight to the terminal. A read-along take keeps showing yours.
    if (!r.first && d.score && !S.bench.take) {
      r.first = d.id;
      S.bench.view = d.id;
      if (S.page === 'bench') { refreshBench(); startReveal(); }
      return;
    }
    (d.error ? SFX.miss : SFX.step)();
    benchPart('board', raceBoard());
  },
  race_end: (d) => {
    const r = S.bench.race; if (!r) return;
    r.running = false;
    stopAudio();
    if (!d.cancelled && Object.values(r.rows).some((x) => x.score)) SFX.win();
    refreshBench();
  },
  perf: (d) => {
    S.snap.perf = d;
    // Don't yank the table out from under someone typing in perf_max.
    if (S.page === 'perf' && !document.activeElement?.matches('input')) render();
  },
};

// Scripts Rust runs via evaluate_script don't always surface on window.onerror,
// so catch here and ship the stack home to the log.
window.dv = {
  recv: (m) => {
    try { (ON[m.ev] || ((d) => console.warn('unhandled', m.ev, d)))(m.data); }
    catch (e) { ipc('jserror', { msg: `recv ${m.ev}: ${e && e.stack || e}` }); }
  },
};

// ── us → Rust ────────────────────────────────────────────────────────────

/// Two-click guard for destructive buttons. First click arms, second fires,
/// three seconds of hesitation disarms.
const armed = (tag, fire) => {
  if (S.confirm === tag) { S.confirm = null; fire(); return; }
  S.confirm = tag;
  render();
  setTimeout(() => { if (S.confirm === tag) { S.confirm = null; render(); } }, 3000);
};

const ACT = {
  page: (el) => {
    if (S.page === 'bench' && el.dataset.page !== 'bench') benchDisarm(); // leaving = dictation back to your cursor
    S.page = el.dataset.page; S.confirm = null; render(); $('#page').scrollTop = 0;
  },
  // ── benchmark ──
  bench_pick: (el) => {
    const b = S.bench;
    if (b.race?.running) return; // one tape at a time
    b.clip = el.dataset.id; b.take = null; b.race = null; b.view = 'take'; b.revealed = 0; clearInterval(b.timer);
    stopAudio();
    if (b.armed) ipc('bench_arm', { reference: clip().script }); // re-arm against the new script
    render();
  },
  // ▶ Run benchmark: the clip, every installed model, active one first.
  race_clip: () => {
    const b = S.bench, c = clip();
    if (!c || b.race?.running) return;
    SFX.unlock(); SFX.arm();
    b.take = null; b.view = 'take'; b.revealed = 0; clearInterval(b.timer);
    benchPart('term-body', termBody());
    ipc('race', { clip: c.id, ...raceScope() });
    stopAudio();
    if (S.play) {
      b.audio = new Audio(`/clip/${encodeURIComponent(c.id)}`);
      b.audio.play().then(progressLoop).catch(() => { b.audio = null; });
    }
  },
  play: (el) => { S.play = !S.play; pref.set('dv.play', S.play); el.checked = S.play; },
  // "or read it yourself →": arm against the clip's script.
  bench_arm: () => {
    const c = clip(); if (!c) return;
    SFX.unlock(); // the click is our one chance to get an AudioContext
    stopAudio();
    Object.assign(S.bench, { armed: true, take: null, race: null, view: 'take', revealed: 0 });
    clearInterval(S.bench.timer);
    ipc('bench_arm', { reference: c.script });
    SFX.arm();
    render();
  },
  bench_disarm: () => { benchDisarm(); render(); },
  race: () => ipc('race', raceScope()),
  // Device chips are multi-select, but never zero: an empty race is a no-op with extra steps.
  bench_dev: (el) => {
    const d = el.dataset.dev, cur = benchDevs();
    const next = Object.keys(DEV).filter((x) => x === d ? !cur.includes(d) : cur.includes(x));
    if (!next.length) return;
    S.bench.devs = next; pref.set('dv.bdevs', next);
    refreshBench();
  },
  race_cancel: () => { ipc('race_cancel'); stopAudio(); },
  bench_view: (el) => {
    S.bench.view = el.dataset.id;
    render();
    startReveal();
  },
  sfx: () => { S.sfx = !S.sfx; pref.set('dv.sfx', S.sfx); SFX.unlock(); render(); },
  step: (el, e) => {
    const input = document.getElementById(el.dataset.for);
    if (!input) return;
    const by = Number(input.step) || 1, k = e && e.shiftKey ? 10 : 1;
    const next = Math.min(Number(input.max), Math.max(Number(input.min), (Number(input.value) || 0) + Number(el.dataset.d) * by * k));
    if (next === Number(input.value)) return;
    input.value = next;
    input.dispatchEvent(new Event('change', { bubbles: true })); // same path as typing
  },
  // Both go to the tray. Real quitting lives in the tray menu, on purpose.
  win_min: () => ipc('win_min'),
  win_close: () => ipc('win_close'),
  set: (el) => ipc('set', { key: el.dataset.k, value: el.dataset.v }),
  learn: () => {
    S.learnErr = null;
    S.learning = !S.learning;
    ipc(S.learning ? 'learn' : 'learn_cancel');
    render();
  },
  mics: () => ipc('mics'),
  model_download: (el) => ipc('model_download', { id: el.dataset.id }),
  model_cancel: () => ipc('model_cancel'),
  model_use: (el) => ipc('model_use', { id: el.dataset.id }),
  theme_menu: () => {
    if (document.getElementById('theme-panel')) { themeClose(); return; }
    const p = document.createElement('div');
    p.id = 'theme-panel'; p.className = 'theme-panel'; p.setAttribute('role', 'dialog');
    p.innerHTML = themePanel();
    const r = $('#tb-theme').getBoundingClientRect(); // hang it off the button, right-aligned
    p.style.top = `${r.bottom + 6}px`;
    p.style.right = `${Math.max(6, innerWidth - r.right)}px`;
    document.body.append(p);
    p.querySelector('.tp-tile.on')?.scrollIntoView({ block: 'center' });
  },
  theme_pick: (el) => {
    const id = el.dataset.id;
    paint(id);
    pref.set('dv.theme', id);
    S.snap.config.theme = id; // so the close below doesn't snap back before Rust confirms
    themeClose();
    ipc('set', { key: 'theme', value: id });
  },
  model_info: (el) => { S.info = S.info === el.dataset.id ? null : el.dataset.id; render(); },
  // Right-click ▸ Benchmark: straight to the race, active model, current device.
  bench_go: () => { ACT.page({ dataset: { page: 'bench' } }); ACT.race_clip(); },
  model_delete: (el) => armed(`del:${el.dataset.id}`, () => ipc('model_delete', { id: el.dataset.id })),
  model_dir: () => ipc('model_dir', { path: $('#custom-dir').value }),
  history_delete: (el) => ipc('history_delete', { id: Number(el.dataset.id) }),
  history_clear: () => armed('clear', () => ipc('history_clear')),
  perf_clear: () => armed('perf', () => ipc('perf_clear')),
  copy: (el) => ipc('copy', { id: Number(el.dataset.id) }),
  open: (el) => ipc('open', { what: el.dataset.what }),
  url: (el) => ipc('url', { url: el.dataset.url }),
};

// ── themes ───────────────────────────────────────────────────────────────
//
// A theme is a bag of --io-* variables (themes.js) poured over :root as
// inline styles; "default" pours nothing and theme.css shows through. Hover a
// tile = preview on the whole app, live. Leave = back to what you had. Click
// = keep it (config.toml, plus localStorage so the next launch paints the
// right colours before Rust even says hello).

const THEMES = window.DV_THEMES || [];
const THEME_GROUPS = [['dark', 'dark'], ['light', 'light'], ['retro', 'phosphor'], ['extreme', 'extreme'], ['a11y', 'accessible']];
const theme = (id) => THEMES.find((t) => t.id === id) || THEMES[0];
const tri = (t) => ['--io-accent', '--io-surface-raised', '--io-text-primary'].map((k) => t?.vars[k] || 'transparent');
let painted = null;
const paint = (id) => {
  if (painted === id) return;
  const t = theme(id), root = document.documentElement;
  [...root.style].filter((p) => p.startsWith('--io-')).forEach((p) => root.style.removeProperty(p));
  if (t && t.id !== 'default') Object.entries(t.vars).forEach(([k, v]) => root.style.setProperty(k, v));
  root.dataset.mode = t?.mode || 'dark';
  painted = id;
};
paint(pref.get('dv.theme', 'default')); // before the first frame, not after hello

const swatch = (t, cls = '') => `<span class="swatch ${cls}">${tri(t).map((c) => `<i style="background:${c}"></i>`).join('')}</span>`;

function themePanel() {
  const cur = cfg().theme;
  return `<div class="tp-head"><span>theme</span><b id="tp-name">${esc(theme(cur).name)}</b><span class="spacer"></span><span class="dim" id="tp-desc">${esc(theme(cur).desc)}</span></div>
    <div class="tp-body">${THEME_GROUPS.map(([mode, label]) => {
      const ts = THEMES.filter((t) => t.mode === mode);
      return when(ts.length, () => `<div class="tp-label">${label}</div><div class="tp-grid">${ts.map((t) =>
        `<button class="tp-tile ${t.id === cur ? 'on' : ''}" data-act="theme_pick" data-id="${t.id}" title="${esc(t.desc)}">${swatch(t)}<span>${esc(t.name)}</span></button>`).join('')}</div>`);
    }).join('')}</div>`;
}

const themeClose = () => {
  const p = document.getElementById('theme-panel');
  if (!p) return;
  p.remove();
  paint(cfg().theme); // a preview never outlives the panel
};

document.addEventListener('mouseover', (e) => {
  const p = document.getElementById('theme-panel');
  if (!p) return;
  const tile = e.target.closest('.tp-tile');
  const t = tile ? theme(tile.dataset.id) : theme(cfg().theme);
  if (tile || !e.target.closest('#theme-panel')) {
    paint(t.id);
    $('#tp-name').textContent = t.name;
    $('#tp-desc').textContent = t.desc;
  }
});

// ── right-click menu ─────────────────────────────────────────────────────
//
// WebView2's own menu (Back / Refresh / Save as / Print) is a web page's
// menu, and this isn't a web page. Ours does app things: swap the device,
// swap the model, race it, grab the last transcript, get out of the way.
// Text fields keep the native one: cut/copy/paste is what you want there.

const ctxHtml = () => {
  const s = S.snap, c = cfg(), last = s.history[0];
  const item = (act, label, attrs = '', hint = '') =>
    `<button class="ctx-item" data-act="${act}" ${attrs}><span>${label}</span>${when(hint, `<span class="ctx-hint">${hint}</span>`)}</button>`;
  const devs = devices().map((d) => devBtn(d, c.device === d.device)).join('');
  const models = s.models.filter((m) => m.installed).map((m) =>
    item(m.id === s.active_model ? 'noop' : 'model_use', `<i class="ctx-dot ${m.id === s.active_model ? 'on' : ''}"></i>${esc(m.name)}`, `data-id="${m.id}"`, (ENG[m.engine] || [m.engine])[0]));
  return `<div class="ctx-head"><span class="dot ${modelState().cls}"></span>${esc(MODE_TEXT[s.status.mode] || s.status.mode)}<span class="spacer"></span>v${esc(s.version)}</div>
    <div class="ctx-label">run on</div><div class="seg ctx-seg">${devs}</div>
    ${when(models.length > 1, () => `<div class="ctx-label">model</div>${models.join('')}`)}
    <div class="ctx-sep"></div>
    ${item('bench_go', `${I.bench}Benchmark this setup`)}
    ${when(last, () => item('copy', `${I.copy}Copy last transcript`, `data-id="${last.id}"`))}
    ${item('open', `${I.folder}Open logs`, 'data-what="logs"')}
    <div class="ctx-sep"></div>
    ${item('win_min', 'Minimise to tray', '', 'Esc')}
    ${item('win_close', 'Close to tray', '', 'Alt+F4')}`;
};

const ctxClose = () => document.getElementById('ctx')?.remove();
document.addEventListener('contextmenu', (e) => {
  if (e.target.closest('input, textarea')) return;
  e.preventDefault();
  ctxClose();
  const m = document.createElement('div');
  m.id = 'ctx'; m.className = 'ctx'; m.setAttribute('role', 'menu');
  m.innerHTML = ctxHtml();
  document.body.append(m);
  // Open where you clicked, flipped back inside the window near the edges.
  const r = m.getBoundingClientRect();
  m.style.left = `${Math.min(e.clientX, innerWidth - r.width - 6)}px`;
  m.style.top = `${Math.min(e.clientY, innerHeight - r.height - 6)}px`;
});
// Any click closes it: inside, after the item's action runs (bubbling hits
// the action handler first); outside, straight away.
document.addEventListener('click', () => setTimeout(ctxClose), true);
document.addEventListener('mousedown', (e) => {
  if (!e.target.closest('#theme-panel, #tb-theme')) themeClose();
}, true);
window.addEventListener('blur', ctxClose);
document.addEventListener('wheel', ctxClose, { passive: true });

document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-act]');
  // Inputs keep their native toggle: preventDefault on a checkbox click makes
  // the browser undo the flip AFTER our handler runs.
  if (el && ACT[el.dataset.act]) { if (el.tagName !== 'INPUT') e.preventDefault(); ACT[el.dataset.act](el, e); }
});

/// Every settings input is `data-key=<config field>`. Coerce by kind, ship it.
const valueOf = (el) => {
  const k = el.dataset.key;
  if (el.type === 'checkbox') {
    if (k === 'fbdirector') return el.checked ? '127.0.0.1:17879' : null;
    return el.checked;
  }
  if ('num' in el.dataset) return Number(el.value) || 0;
  if ('lines' in el.dataset) return el.value.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
  if (k === 'input_device' || k === 'fbdirector') return el.value.trim() || null;
  return el.value;
};

document.addEventListener('change', (e) => {
  if (e.target.matches('[data-bmodels]')) { S.bench.models = e.target.value; pref.set('dv.bmodels', e.target.value); return; }
  const el = e.target.closest('[data-key]');
  if (el) ipc('set', { key: el.dataset.key, value: valueOf(el) });
});

// The titlebar is ours, so dragging is too: mousedown anywhere on it that
// isn't a button hands the drag to Windows (Rust calls drag_window()).
// Double-click does nothing — there is no maximise, by design.
$('#titlebar').addEventListener('mousedown', (e) => {
  if (e.button === 0 && !e.target.closest('button')) { e.preventDefault(); ipc('win_drag'); }
});
$('#titlebar').addEventListener('dblclick', (e) => e.preventDefault());

document.addEventListener('keydown', (e) => {
  // While rebinding, the keyboard belongs to the hook. Don't let Space click
  // buttons or Tab wander off.
  if (S.learning) { e.preventDefault(); return; }
  if (e.key === 'Escape') { if (document.getElementById('theme-panel')) themeClose(); else if (document.getElementById('ctx')) ctxClose(); else if (!e.target.matches('input, textarea, select')) ACT.win_min(); return; }
  if (e.key === 'Enter' && e.target.id === 'custom-dir') ACT.model_dir();
  if (e.key === 'Enter' && e.target.matches('input[data-key]')) e.target.blur();
});

// Refresh relative timestamps without spamming Rust.
setInterval(() => { if (S.page === 'history' && !document.activeElement?.matches('input')) render(); }, 30000);

ipc('hello');
})();
