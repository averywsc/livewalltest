'use strict';
const TAURI = window.__TAURI__;
const invoke = (cmd, args) => TAURI.core.invoke(cmd, args);

const PAGES = [['library', 'Library'], ['overlays', 'Overlays'], ['music', 'Music'], ['settings', 'Settings']];
const FITS = [['fill', 'Fill'], ['fit', 'Fit'], ['stretch', 'Stretch'], ['center', 'Center'], ['tile', 'Tile']];
const MEDIA_EXT = ['mp4', 'mkv', 'webm', 'mov', 'avi', 'gif', 'png', 'jpg', 'jpeg', 'bmp', 'webp'];
const AUDIO_EXT = ['mp3', 'flac', 'wav', 'ogg', 'm4a'];

let S = null;            // last state from the renderer
let offline = false;
let page = 'library';
let busy = '';
let renaming = null;     // { kind: 'item' | 'scene', id }
let confirming = null;   // { kind, id }
let targetMon = null;
let selOverlay = null;
let pointerDown = false;
let lastSig = '';
let dropActive = false;
const draft = { rename: '', scene: '' };
const thumbCache = new Map();

/* ---------- helpers ---------- */

async function rpc(obj) {
  try {
    const raw = await invoke('rpc', { request: JSON.stringify(obj) });
    return JSON.parse(raw);
  } catch (e) {
    if (String(e) === 'offline') return { ok: false, offline: true };
    return { ok: false, error: String(e) };
  }
}

function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    if (k === 'class') el.className = v;
    else if (k === 'style') Object.assign(el.style, v);
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const btn = (text, onclick, attrs) => h('button', { type: 'button', onclick, ...attrs }, text);

function seg(label, options, value, onpick) {
  return h('div', { class: 'seg', role: 'group', 'aria-label': label },
    options.map(([v, t]) => btn(t, () => onpick(v), { 'aria-pressed': String(v === value) })));
}

function toggle(label, on, onchange) {
  return btn(on ? 'On' : 'Off', () => onchange(!on), { role: 'switch', 'aria-checked': String(on), 'aria-label': label });
}

function slider(label, min, max, value, oninput, oncommit, fmt) {
  const out = h('span', { class: 'value' }, fmt ? fmt(value) : value);
  const input = h('input', { type: 'range', min, max, value, 'aria-label': label });
  input.addEventListener('input', () => { out.textContent = fmt ? fmt(+input.value) : input.value; oninput(+input.value); });
  if (oncommit) input.addEventListener('change', () => oncommit(+input.value));
  return h('span', { class: 'controls' }, input, out);
}

function row(label, ...controls) {
  return h('div', { class: 'row' }, h('div', { class: 'label' }, label), h('div', { class: 'controls' }, controls));
}

function throttle(fn, ms) {
  let last = 0, timer = null, pending = null;
  return (...a) => {
    const now = Date.now();
    if (now - last >= ms) { last = now; fn(...a); return; }
    pending = a;
    clearTimeout(timer);
    timer = setTimeout(() => { last = Date.now(); fn(...pending); }, ms - (now - last));
  };
}

function fmtDur(s) {
  if (!s || s <= 0) return '';
  const m = Math.floor(s / 60), r = Math.round(s % 60);
  return `${m}:${String(r).padStart(2, '0')}`;
}

function kindLabel(k) { return k === 'video' ? 'Video' : k === 'gif' ? 'GIF' : 'Image'; }

function metaOf(it) {
  const p = [kindLabel(it.kind)];
  if (it.duration > 0) p.push(fmtDur(it.duration));
  if (it.fps > 0 && it.kind === 'video') p.push(`${Math.round(it.fps)} fps`);
  return p.join(', ');
}

function thumbBox(path, cls) {
  const box = h('div', { class: cls || 'pic' });
  if (!path) { box.textContent = cls ? '' : 'No preview'; return box; }
  if (!thumbCache.has(path)) thumbCache.set(path, invoke('read_data_url', { path }).catch(() => ''));
  thumbCache.get(path).then(url => {
    if (url) box.replaceChildren(h('img', { src: url, alt: '' }));
    else if (!cls) box.textContent = 'No preview';
  });
  return box;
}

async function pick(exts, name) {
  const r = await TAURI.dialog.open({ multiple: true, filters: [{ name, extensions: exts }] });
  if (!r) return [];
  return Array.isArray(r) ? r : [r];
}

async function act(obj, then) {
  const r = await rpc(obj);
  if (!r.ok && !r.offline) showError(r.error || 'Something went wrong.');
  await refresh(true);
  if (then) then(r);
  return r;
}

let errorText = '';
function showError(t) { errorText = t; setTimeout(() => { if (errorText === t) { errorText = ''; render(); } }, 6000); }

/* ---------- state ---------- */

async function refresh(force) {
  const r = await rpc({ cmd: 'get_state' });
  if (r.offline) { offline = true; } else if (r.ok) { offline = false; S = r; }
  const sig = JSON.stringify([offline, S && [S.library, S.live, S.scenes, S.settings, S.runtime, S.music], errorText, busy]);
  const typing = document.activeElement && document.activeElement.tagName === 'INPUT';
  if (force || (sig !== lastSig && !pointerDown && !typing)) { lastSig = sig; render(); }
}

function sendOverlay(o, commit) {
  rpc({ cmd: 'set_overlay', id: o.id, cx: o.cx, cy: o.cy, w: o.w, h: o.h, rotation: o.rotation, opacity: o.opacity, commit: !!commit });
}
const sendOverlayThrottled = throttle(sendOverlay, 40);

/* ---------- shell ---------- */

function statusLine() {
  if (offline) return ['The wallpaper service is not running.', true];
  if (!S) return ['Loading', false];
  const rt = S.runtime;
  if (busy) return [busy, false];
  if (rt.paused) return [`Paused: ${rt.paused}`, false];
  if (rt.active) return ['Playing', false];
  return ['No wallpaper is set', false];
}

function header() {
  const [text, alert] = statusLine();
  const canPause = S && !offline;
  return h('header', { class: 'top' },
    h('div', {}, h('h1', {}, 'Live Wallpaper'), h('div', { class: 'status' + (alert ? ' alert' : ''), role: 'status' }, text)),
    canPause && btn(S.runtime.manual_pause ? 'Resume wallpaper' : 'Pause wallpaper', () => act({ cmd: 'pause_toggle' })));
}

function navBar() {
  return h('nav', { role: 'tablist', 'aria-label': 'Sections' },
    PAGES.map(([id, label]) => btn(label, () => { page = id; renaming = null; confirming = null; render(); }, { role: 'tab', 'aria-selected': String(page === id) })));
}

function render() {
  const active = document.activeElement;
  const fid = active && active.dataset ? active.dataset.fid : null;
  const root = document.getElementById('app');
  root.replaceChildren(header(), navBar(), h('main', { id: 'main' }, mainContent()));
  let n = 0;
  root.querySelectorAll('button,input').forEach(el => { el.dataset.fid = page + ':' + n++; });
  if (fid) { const el = root.querySelector(`[data-fid="${fid}"]`); if (el) el.focus(); }
}

function mainContent() {
  if (offline) {
    return h('div', { class: 'notice' },
      h('p', {}, 'The wallpaper service is not running, so nothing can be shown or changed. Start it to continue.'),
      btn('Start wallpaper service', async () => { await invoke('start_renderer').catch(e => showError(String(e))); setTimeout(() => refresh(true), 1200); }));
  }
  if (!S) return h('p', { class: 'dim' }, 'Loading');
  const errs = [...(S.runtime.errors || [])];
  if (errorText) errs.push(errorText);
  return [errs.map(e => h('p', { class: 'err', role: 'alert' }, e)),
    page === 'library' ? pageLibrary() : page === 'overlays' ? pageOverlays() : page === 'music' ? pageMusic() : pageSettings()];
}

/* ---------- drop zone ---------- */

function dropZone(text) {
  return h('div', { class: 'drop' + (dropActive ? ' over' : ''), id: 'dropzone' }, text);
}

async function handleDrop(paths) {
  if (!paths || !paths.length || offline) return;
  const audio = paths.filter(p => AUDIO_EXT.includes(p.split('.').pop().toLowerCase()));
  const media = paths.filter(p => MEDIA_EXT.includes(p.split('.').pop().toLowerCase()));
  busy = 'Adding files';
  render();
  if (page === 'music') { if (audio.length) await rpc({ cmd: 'music_add', paths: audio }); }
  else if (page === 'overlays') { if (media.length) { const r = await rpc({ cmd: 'add_overlay', paths: media }); if (r.added && r.added[0]) selOverlay = r.added[0].id; } }
  else { if (media.length) await rpc({ cmd: 'add_library', paths: media }); if (page !== 'library') page = 'library'; }
  busy = '';
  await refresh(true);
}

/* ---------- library ---------- */

function pageLibrary() {
  const L = S.live, w = L.wallpaper, mons = S.runtime.monitors;
  if (!targetMon || !mons.find(m => m.device === targetMon)) targetMon = (mons.find(m => m.primary) || mons[0] || {}).device;
  const assign = w.monitors.find(m => m.device === targetMon);
  const activeIds = new Set(w.span ? [w.span_item] : w.monitors.map(m => m.item));

  const setWall = (patch) => act({ cmd: 'set_wallpaper', wallpaper: { ...w, ...patch } });
  const setFit = (fit) => {
    if (w.span) return setWall({ span_fit: fit });
    const list = w.monitors.map(m => m.device === targetMon ? { ...m, fit } : m);
    return setWall({ monitors: list });
  };
  const setItem = (id) => {
    if (w.span) return setWall({ span_item: id });
    const list = w.monitors.some(m => m.device === targetMon)
      ? w.monitors.map(m => m.device === targetMon ? { ...m, item: id } : m)
      : [...w.monitors, { device: targetMon, item: id, fit: 'fill' }];
    return setWall({ monitors: list });
  };
  const clearIt = () => w.span ? setWall({ span_item: '' }) : setWall({ monitors: w.monitors.filter(m => m.device !== targetMon) });

  const currentId = w.span ? w.span_item : (assign && assign.item);
  const currentFit = w.span ? w.span_fit : (assign ? assign.fit : 'fill');
  const currentItem = S.library.find(i => i.id === currentId);

  const st = S.settings;
  const setSettings = (patch, commit = true) => {
    S.settings = { ...S.settings, ...patch };
    return commit ? act({ cmd: 'set_settings', settings: S.settings }) : rpc({ cmd: 'set_settings', settings: S.settings });
  };
  const sendVol = throttle(() => rpc({ cmd: 'set_settings', settings: S.settings }), 80);

  return [
    h('section', {},
      h('h2', {}, 'Wallpaper'),
      mons.length > 1 && row('Displays', seg('Displays', [['sep', 'A different wallpaper on each'], ['span', 'One across all displays']], w.span ? 'span' : 'sep',
        v => setWall({ span: v === 'span' }))),
      !w.span && mons.length > 1 && row('Change wallpaper on', seg('Display to change',
        mons.map(m => [m.device, `${m.label}, ${m.w} by ${m.h}${m.primary ? ', main' : ''}`]), targetMon, v => { targetMon = v; render(); })),
      row('Fit', seg('Fit', FITS, currentFit, setFit),
        currentFit === 'tile' && h('span', { class: 'dim small' }, 'Tile uses the processor. Best for images and short GIFs.')),
      row('Now showing', h('span', { class: 'now' }, currentItem ? currentItem.name : 'Nothing'),
        currentItem && btn('Remove wallpaper', clearIt)),
      row('Video sound', toggle('Video sound', !st.video_muted, on => setSettings({ video_muted: !on })),
        !st.video_muted && slider('Video volume', 0, 100, st.video_volume, v => { S.settings.video_volume = v; sendVol(); }, null, v => `${v}`))),
    h('section', {},
      h('h2', {}, 'Library'),
      dropZone('Drop videos, GIFs or images here, or use the button below.'),
      h('div', { class: 'controls', style: { marginBottom: '24px' } },
        btn('Add video, GIF or image', async () => {
          const paths = await pick(MEDIA_EXT, 'Video, GIF and images');
          if (!paths.length) return;
          busy = 'Adding files'; render();
          await rpc({ cmd: 'add_library', paths });
          busy = ''; refresh(true);
        })),
      S.library.length === 0
        ? h('p', { class: 'dim' }, 'The library is empty. Add a file to start.')
        : h('div', { class: 'grid' }, S.library.map(it => tile(it, activeIds.has(it.id), it.id === currentId, setItem)))),
  ];
}

function tile(it, inUse, isCurrent, setItem) {
  const isRen = renaming && renaming.kind === 'item' && renaming.id === it.id;
  const isConf = confirming && confirming.kind === 'item' && confirming.id === it.id;
  return h('div', { class: 'tile' + (isCurrent ? ' current' : '') },
    thumbBox(it.thumb),
    inUse && h('span', { class: 'badge' }, 'In use'),
    isRen
      ? h('div', { class: 'acts' },
          h('input', { type: 'text', value: draft.rename, 'aria-label': 'New name', oninput: e => { draft.rename = e.target.value; }, onkeydown: e => { if (e.key === 'Enter') doRename(); if (e.key === 'Escape') { renaming = null; render(); } } }),
          btn('Save', doRename), btn('Cancel', () => { renaming = null; render(); }))
      : h('div', { class: 'name', title: it.path }, it.name),
    h('div', { class: 'meta' }, metaOf(it)),
    isConf
      ? h('div', { class: 'acts' }, h('span', { class: 'small' }, 'Remove from library?'), btn('Remove', () => { confirming = null; act({ cmd: 'remove_item', id: it.id }); }), btn('Keep', () => { confirming = null; render(); }))
      : !isRen && h('div', { class: 'acts' },
          btn('Set as wallpaper', () => setItem(it.id)),
          btn('Rename', () => { renaming = { kind: 'item', id: it.id }; draft.rename = it.name; render(); }),
          btn('Remove', () => { confirming = { kind: 'item', id: it.id }; render(); })));
}

async function doRename() {
  if (!renaming) return;
  const r = renaming; renaming = null;
  await act({ cmd: r.kind === 'item' ? 'rename_item' : 'scene_rename', id: r.id, name: draft.rename });
}

/* ---------- overlays ---------- */

function pageOverlays() {
  const ovs = S.live.overlays, mons = S.runtime.monitors;
  if (!ovs.find(o => o.id === selOverlay)) selOverlay = ovs.length ? ovs[0].id : null;
  const sel = ovs.find(o => o.id === selOverlay);

  const addBtn = btn('Add overlay', async () => {
    const paths = await pick(MEDIA_EXT, 'GIF, images and videos');
    if (!paths.length) return;
    busy = 'Adding overlay'; render();
    const r = await rpc({ cmd: 'add_overlay', paths });
    if (r.added && r.added[0]) selOverlay = r.added[0].id;
    busy = ''; refresh(true);
  });

  return [
    h('section', {},
      h('h2', {}, 'Overlays'),
      h('p', { class: 'dim', style: { marginBottom: '16px', maxWidth: '62ch' } },
        'Overlays sit on the wallpaper, behind desktop icons, and never block clicks. Drag one in the layout below to move it, drag the square corner to resize, and drag the top square to rotate.'),
      dropZone('Drop GIFs, images or short videos here, or use the button below.'),
      h('div', { class: 'controls' }, addBtn),
      ovs.length === 0 ? h('p', { class: 'dim', style: { marginTop: '16px' } }, 'There are no overlays. Add one to place it on the wallpaper.') : previewBox(mons, ovs)),
    ovs.length > 0 && h('section', {},
      h('h2', {}, 'Overlay list'),
      h('ul', { class: 'list' }, ovs.map(o => overlayRow(o, o.id === selOverlay))),
      sel && h('div', { style: { marginTop: '16px' } },
        row('Opacity', slider('Opacity', 0, 100, Math.round(sel.opacity * 100), v => { sel.opacity = v / 100; sendOverlayThrottled(sel, false); }, () => sendOverlay(sel, true), v => `${v}%`)),
        row('Rotation', slider('Rotation in degrees', -180, 180, Math.round(sel.rotation), v => { sel.rotation = v; sendOverlayThrottled(sel, false); refreshPreview(); }, () => sendOverlay(sel, true), v => `${v} deg`)),
        row('Width', slider('Width in pixels', 16, Math.max(64, virtualWidth(mons)), Math.round(sel.w), v => { const k = sel.h / sel.w; sel.w = v; sel.h = Math.max(8, v * k); sendOverlayThrottled(sel, false); refreshPreview(); }, () => sendOverlay(sel, true), v => `${v} px`)))),
  ];
}

function overlayRow(o, isSel) {
  const isConf = confirming && confirming.kind === 'overlay' && confirming.id === o.id;
  return h('li', { class: isSel ? 'sel' : '' },
    thumbBox(o.thumb, 'thumb'),
    h('button', { type: 'button', class: isSel ? 'on' : '', style: { border: '0', textAlign: 'left', background: 'transparent', color: 'inherit' }, 'aria-pressed': String(isSel), onclick: () => { selOverlay = o.id; render(); } },
      `${o.name}`, h('span', { class: 'dim small' }, `  ${kindLabel(o.kind)}`)),
    isConf
      ? h('div', { class: 'acts' }, btn('Delete', () => { confirming = null; act({ cmd: 'remove_overlay', id: o.id }); }), btn('Keep', () => { confirming = null; render(); }))
      : h('div', { class: 'acts' }, btn('Delete', () => { confirming = { kind: 'overlay', id: o.id }; render(); })));
}

function virtualRect(mons) {
  const x0 = Math.min(...mons.map(m => m.x)), y0 = Math.min(...mons.map(m => m.y));
  const x1 = Math.max(...mons.map(m => m.x + m.w)), y1 = Math.max(...mons.map(m => m.y + m.h));
  return { x0, y0, vw: x1 - x0, vh: y1 - y0 };
}
const virtualWidth = mons => mons.length ? virtualRect(mons).vw : 1920;

let previewState = null;

function previewBox(mons, ovs) {
  if (!mons.length) return h('p', { class: 'dim' }, 'No displays were found.');
  const { x0, y0, vw, vh } = virtualRect(mons);
  const maxW = Math.min(900, (document.getElementById('app').clientWidth || 900) - 48);
  const scale = Math.min(maxW / vw, 360 / vh);
  const wrap = h('div', { class: 'preview', tabindex: '0', role: 'group', 'aria-label': 'Overlay layout. Use the arrow keys to move the selected overlay. Hold Shift for larger steps.', style: { width: vw * scale + 'px', height: vh * scale + 'px' } });
  mons.forEach(m => wrap.append(h('div', { class: 'mon', style: { left: (m.x - x0) * scale + 'px', top: (m.y - y0) * scale + 'px', width: m.w * scale + 'px', height: m.h * scale + 'px' } }, m.label)));
  previewState = { wrap, x0, y0, scale, boxes: new Map() };

  for (const o of ovs) {
    const isSel = o.id === selOverlay;
    const box = h('div', { class: 'ovbox' + (isSel ? ' sel' : ''), 'data-id': o.id });
    const pic = thumbBox(o.thumb, 'ph-wrap');
    pic.style.width = '100%'; pic.style.height = '100%';
    box.append(pic);
    if (isSel) {
      box.append(h('div', { class: 'handle stem' }), h('div', { class: 'handle rot', 'data-mode': 'rotate' }), h('div', { class: 'handle br', 'data-mode': 'resize' }));
    }
    previewState.boxes.set(o.id, box);
    place(box, o);
    box.addEventListener('pointerdown', e => beginDrag(e, o));
    wrap.append(box);
  }
  wrap.addEventListener('keydown', e => {
    const o = S.live.overlays.find(x => x.id === selOverlay);
    if (!o) return;
    const step = e.shiftKey ? 50 : 10;
    const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key];
    if (!d) return;
    e.preventDefault();
    o.cx += d[0]; o.cy += d[1];
    place(previewState.boxes.get(o.id), o);
    sendOverlayThrottled(o, false);
    clearTimeout(wrap._commit);
    wrap._commit = setTimeout(() => sendOverlay(o, true), 400);
  });
  return wrap;
}

function place(box, o) {
  const p = previewState;
  const w = o.w * p.scale, hh = o.h * p.scale;
  Object.assign(box.style, {
    width: w + 'px', height: hh + 'px',
    left: (o.cx - p.x0) * p.scale - w / 2 + 'px', top: (o.cy - p.y0) * p.scale - hh / 2 + 'px',
    transform: `rotate(${o.rotation}deg)`, opacity: String(Math.max(0.15, o.opacity)),
  });
}

function refreshPreview() {
  if (!previewState) return;
  for (const o of S.live.overlays) { const b = previewState.boxes.get(o.id); if (b) place(b, o); }
}

function beginDrag(e, o) {
  e.preventDefault();
  const wasSel = selOverlay === o.id;
  selOverlay = o.id;
  if (!wasSel) { render(); const b = previewState.boxes.get(o.id); if (b) b.dispatchEvent(new PointerEvent('pointerdown', e)); return; }
  const mode = e.target.dataset && e.target.dataset.mode || 'move';
  const p = previewState, r = p.wrap.getBoundingClientRect();
  const toPx = ev => [ev.clientX - r.left, ev.clientY - r.top];
  const [sx, sy] = toPx(e);
  const start = { cx: o.cx, cy: o.cy, w: o.w, h: o.h };
  const cxp = (o.cx - p.x0) * p.scale, cyp = (o.cy - p.y0) * p.scale;
  const d0 = Math.hypot(sx - cxp, sy - cyp) || 1;
  pointerDown = true;
  const move = ev => {
    const [px, py] = toPx(ev);
    if (mode === 'move') { o.cx = start.cx + (px - sx) / p.scale; o.cy = start.cy + (py - sy) / p.scale; }
    else if (mode === 'resize') { const f = Math.max(0.05, Math.hypot(px - cxp, py - cyp) / d0); o.w = Math.max(16, start.w * f); o.h = Math.max(16, start.h * f); }
    else { let a = Math.atan2(py - cyp, px - cxp) * 180 / Math.PI + 90; if (a > 180) a -= 360; if (ev.shiftKey) a = Math.round(a / 15) * 15; o.rotation = Math.round(a); }
    place(p.boxes.get(o.id), o);
    sendOverlayThrottled(o, false);
  };
  const up = () => {
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    pointerDown = false;
    sendOverlay(o, true);
    setTimeout(() => refresh(true), 100);
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

/* ---------- music ---------- */

function pageMusic() {
  const m = S.live.music, ms = S.music, list = m.playlist;
  const playing = ms.playing && !ms.paused;
  const set = (patch, commit) => { Object.assign(m, patch); return rpc({ cmd: 'music_set', ...patch, commit: commit !== false }); };
  const sendVol = throttle(v => rpc({ cmd: 'music_set', volume: v, commit: false }), 60);
  const loopLabel = { off: 'Loop off', all: 'Loop all', one: 'Loop one' };
  const nextLoop = { off: 'all', all: 'one', one: 'off' };

  return [
    h('section', {},
      h('h2', {}, 'Music'),
      h('p', { class: 'dim', style: { marginBottom: '16px', maxWidth: '62ch' } }, 'Music keeps playing when this window is closed. Its volume is separate from the video sound.'),
      dropZone('Drop audio files here (mp3, flac, wav, ogg, m4a), or use the button below.'),
      h('div', { class: 'controls', style: { marginBottom: '24px' } },
        btn('Add music', async () => {
          const paths = await pick(AUDIO_EXT, 'Audio files');
          if (!paths.length) return;
          await act({ cmd: 'music_add', paths });
        })),
      row('Playback',
        btn('Previous', () => act({ cmd: 'music_ctl', action: 'prev' }), { disabled: list.length === 0 }),
        btn(playing ? 'Pause' : 'Play', () => act({ cmd: 'music_ctl', action: 'toggle' }), { disabled: list.length === 0 }),
        btn('Next', () => act({ cmd: 'music_ctl', action: 'next' }), { disabled: list.length === 0 })),
      row('Order', btn('Shuffle', async () => { await set({ shuffle: !m.shuffle }); refresh(true); }, { 'aria-pressed': String(m.shuffle) }),
        btn(loopLabel[m.loop_mode] || 'Loop all', async () => { await set({ loop_mode: nextLoop[m.loop_mode] || 'all' }); refresh(true); }, { 'aria-pressed': String(m.loop_mode !== 'off') })),
      row('Music volume', slider('Music volume', 0, 100, m.volume, v => { m.volume = v; sendVol(v); }, v => rpc({ cmd: 'music_set', volume: v, commit: true }), v => `${v}`)),
      ms.error && h('p', { class: 'err', role: 'alert' }, ms.error)),
    h('section', {},
      h('h2', {}, 'Playlist'),
      list.length === 0
        ? h('p', { class: 'dim' }, 'The playlist is empty. Add audio files to start.')
        : h('ul', { class: 'list' }, list.map((t, i) => {
            const cur = i === ms.index && (ms.playing || ms.paused);
            return h('li', { class: cur ? 'sel' : '' },
              h('span', { class: 'num' }, String(i + 1)),
              h('span', {}, t.name, cur ? h('span', { class: 'small' }, playing ? '  Playing' : '  Paused') : null),
              h('div', { class: 'acts' }, btn('Play', () => act({ cmd: 'music_ctl', action: 'select', index: i })), btn('Remove', () => act({ cmd: 'music_remove', index: i }))));
          }))),
  ];
}

/* ---------- settings ---------- */

function pageSettings() {
  const st = S.settings;
  const set = patch => { S.settings = { ...S.settings, ...patch }; return act({ cmd: 'set_settings', settings: S.settings }); };

  return [
    h('section', {},
      h('h2', {}, 'Scenes'),
      h('p', { class: 'dim', style: { marginBottom: '16px', maxWidth: '62ch' } }, 'A scene is the wallpaper, its overlays and the music playlist saved together. You can also switch scenes from the tray icon.'),
      h('div', { class: 'controls', style: { marginBottom: '24px' } },
        h('input', { type: 'text', placeholder: 'Scene name', value: draft.scene, 'aria-label': 'Scene name', oninput: e => { draft.scene = e.target.value; }, onkeydown: e => { if (e.key === 'Enter') saveScene(); } }),
        btn('Save current as scene', saveScene)),
      S.scenes.length === 0
        ? h('p', { class: 'dim' }, 'No scenes are saved yet.')
        : h('ul', { class: 'list' }, S.scenes.map(sc => sceneRow(sc)))),
    h('section', {},
      h('h2', {}, 'Frame rate'),
      row('Frame rate limit', seg('Frame rate limit', [[15, '15'], [30, '30'], [60, '60'], [0, 'Match video']], st.fps_cap, v => set({ fps_cap: v }))),
      h('p', { class: 'dim small' }, 'A limit below the video frame rate makes the player copy frames through the processor. It saves graphics power but may use a little more processor time.')),
    h('section', {},
      h('h2', {}, 'Pause'),
      row('Pause when game is open', toggle('Pause when game or fullscreen app is open', st.pause_fullscreen, v => set({ pause_fullscreen: v }))),
      row('Pause when any window is maximized', toggle('Pause when any window is maximized', st.pause_maximized, v => set({ pause_maximized: v }))),
      row('Pause on battery', toggle('Pause when running on battery', st.pause_battery, v => set({ pause_battery: v }))),
      h('p', { class: 'dim small' }, 'The wallpaper always pauses when the screen is locked or off, and while the computer sleeps.')),
    h('section', {},
      h('h2', {}, 'System'),
      row('Start with Windows', toggle('Start with Windows', st.start_with_windows, v => set({ start_with_windows: v }))),
      row('Restore last scene on launch', toggle('Restore last scene on launch', st.restore_last, v => set({ restore_last: v }))),
      row('Stop everything', btn('Quit Live Wallpaper', async () => { await rpc({ cmd: 'quit' }); setTimeout(() => refresh(true), 800); }))),
  ];
}

async function saveScene() {
  const name = draft.scene.trim();
  if (!name) { showError('Enter a name for the scene.'); render(); return; }
  draft.scene = '';
  await act({ cmd: 'scene_save', name });
}

function sceneRow(sc) {
  const isRen = renaming && renaming.kind === 'scene' && renaming.id === sc.id;
  const isConf = confirming && confirming.kind === 'scene' && confirming.id === sc.id;
  const isCur = S.current_scene === sc.id;
  return h('li', { class: isCur ? 'sel' : '', style: { gridTemplateColumns: '1fr auto' } },
    isRen
      ? h('div', { class: 'controls' },
          h('input', { type: 'text', value: draft.rename, 'aria-label': 'New name', oninput: e => { draft.rename = e.target.value; }, onkeydown: e => { if (e.key === 'Enter') doRename(); if (e.key === 'Escape') { renaming = null; render(); } } }),
          btn('Save', doRename), btn('Cancel', () => { renaming = null; render(); }))
      : h('span', {}, sc.name, isCur ? h('span', { class: 'small' }, '  Current') : null),
    isConf
      ? h('div', { class: 'acts' }, h('span', { class: 'small' }, 'Delete this scene?'), btn('Delete', () => { confirming = null; act({ cmd: 'scene_delete', id: sc.id }); }), btn('Keep', () => { confirming = null; render(); }))
      : !isRen && h('div', { class: 'acts' },
          btn('Load', () => act({ cmd: 'scene_load', id: sc.id })),
          btn('Rename', () => { renaming = { kind: 'scene', id: sc.id }; draft.rename = sc.name; render(); }),
          btn('Delete', () => { confirming = { kind: 'scene', id: sc.id }; render(); })));
}

/* ---------- start ---------- */

TAURI.webview.getCurrentWebview().onDragDropEvent(ev => {
  const p = ev.payload;
  if (p.type === 'enter' || p.type === 'over') { if (!dropActive) { dropActive = true; const z = document.getElementById('dropzone'); if (z) z.classList.add('over'); } }
  else if (p.type === 'leave') { dropActive = false; const z = document.getElementById('dropzone'); if (z) z.classList.remove('over'); }
  else if (p.type === 'drop') { dropActive = false; handleDrop(p.paths); }
});

refresh(true);
setInterval(() => { if (!document.hidden) refresh(false); }, 1500);
