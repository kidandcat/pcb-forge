pub const HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>pcb-forge UI</title>
<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
:root {
  --bg-dark: #0f1117;
  --bg-panel: #161822;
  --bg-panel-alt: #1c1e2e;
  --bg-input: #232538;
  --border: #2a2d42;
  --border-hover: #3d4160;
  --text: #c8cad8;
  --text-dim: #6b6f8a;
  --text-bright: #e8eaf4;
  --accent: #e94560;
  --accent-dim: #a83248;
  --green: #2ecc71;
  --blue: #4a90d9;
  --yellow: #f1c40f;
  --red: #e74c3c;
  --pcb-green: #1a5c36;
  --pcb-bg: #0d3320;
}

body {
  background: var(--bg-dark);
  color: var(--text);
  font-family: 'SF Mono', 'Fira Code', 'Consolas', 'Monaco', monospace;
  font-size: 12px;
  overflow: hidden;
  height: 100vh;
  display: flex;
  flex-direction: column;
}

/* ─── Header ─── */
#header {
  background: var(--bg-panel);
  border-bottom: 1px solid var(--border);
  padding: 8px 16px;
  display: flex;
  align-items: center;
  gap: 16px;
  min-height: 40px;
  flex-shrink: 0;
}
#header h1 { font-size: 14px; color: var(--accent); font-weight: 700; white-space: nowrap; letter-spacing: 0.5px; }
#header .sep { width: 1px; height: 20px; background: var(--border); }
.layer-toggle { display: flex; align-items: center; gap: 4px; cursor: pointer; user-select: none; font-size: 11px; color: var(--text-dim); }
.layer-toggle:hover { color: var(--text); }
.layer-toggle input { cursor: pointer; accent-color: var(--accent); }
.layer-toggle .swatch { width: 10px; height: 10px; border-radius: 2px; display: inline-block; }
.grid-select { background: var(--bg-input); color: var(--text); border: 1px solid var(--border); border-radius: 4px; padding: 2px 6px; font-size: 11px; font-family: inherit; }
.grid-select:focus { outline: none; border-color: var(--accent); }
#coords-display { margin-left: auto; color: var(--text-dim); font-size: 11px; white-space: nowrap; }

/* ─── Main layout ─── */
#main {
  display: flex;
  flex: 1;
  overflow: hidden;
}

/* ─── Left panel: Component list ─── */
#left-panel {
  width: 320px;
  min-width: 280px;
  background: var(--bg-panel);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}
#left-panel .panel-header {
  padding: 10px 12px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 1px;
  color: var(--text-dim);
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 8px;
}
#left-panel .panel-header span.count { color: var(--accent); }
#component-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 0;
}
#component-list::-webkit-scrollbar { width: 6px; }
#component-list::-webkit-scrollbar-track { background: transparent; }
#component-list::-webkit-scrollbar-thumb { background: var(--border); border-radius: 3px; }

.comp-item {
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background 0.15s;
}
.comp-item:hover { background: var(--bg-panel-alt); }
.comp-item.selected { background: rgba(233, 69, 96, 0.12); border-left: 3px solid var(--accent); }
.comp-item .comp-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 4px;
}
.comp-item .ref-des { font-weight: 700; color: var(--text-bright); font-size: 12px; }
.comp-item .value { color: var(--accent); font-size: 11px; }
.comp-item .footprint { color: var(--text-dim); font-size: 10px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-bottom: 4px; }
.comp-item .comp-fields {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 4px;
}
.comp-field {
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.comp-field label { font-size: 9px; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.5px; }
.comp-field input {
  background: var(--bg-input);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 3px 5px;
  border-radius: 3px;
  font-size: 11px;
  font-family: inherit;
  width: 100%;
}
.comp-field input:focus { outline: none; border-color: var(--accent); }

/* ─── Center panel: Canvas ─── */
#center-panel {
  flex: 1;
  position: relative;
  overflow: hidden;
  cursor: grab;
  background: var(--bg-dark);
}
#center-panel.dragging-component { cursor: move; }
#center-panel.panning { cursor: grabbing; }

#pcb-canvas {
  width: 100%;
  height: 100%;
  display: block;
}

/* ─── Right panel: Info & Controls ─── */
#right-panel {
  width: 260px;
  min-width: 240px;
  background: var(--bg-panel);
  border-left: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  overflow-y: auto;
}
#right-panel::-webkit-scrollbar { width: 6px; }
#right-panel::-webkit-scrollbar-track { background: transparent; }
#right-panel::-webkit-scrollbar-thumb { background: var(--border); border-radius: 3px; }

.info-section {
  padding: 12px;
  border-bottom: 1px solid var(--border);
}
.info-section h3 {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 1px;
  color: var(--text-dim);
  margin-bottom: 8px;
}
.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 3px 0;
}
.info-row .label { color: var(--text-dim); }
.info-row .value { color: var(--text-bright); font-weight: 500; }
.info-row input {
  background: var(--bg-input);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 3px 6px;
  border-radius: 3px;
  font-size: 11px;
  font-family: inherit;
  width: 70px;
  text-align: right;
}
.info-row input:focus { outline: none; border-color: var(--accent); }

.stat-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 6px;
}
.stat-card {
  background: var(--bg-input);
  border-radius: 6px;
  padding: 8px;
  text-align: center;
}
.stat-card .stat-value { font-size: 18px; font-weight: 700; color: var(--text-bright); }
.stat-card .stat-label { font-size: 9px; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.5px; margin-top: 2px; }

.btn {
  display: block;
  width: 100%;
  padding: 10px;
  border: none;
  border-radius: 6px;
  font-family: inherit;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.btn:active { transform: scale(0.98); }
.btn-build {
  background: var(--accent);
  color: white;
  font-size: 14px;
  padding: 14px;
  margin-bottom: 8px;
}
.btn-build:hover { background: #ff5577; }
.btn-build:disabled { background: var(--accent-dim); cursor: not-allowed; opacity: 0.7; }
.btn-secondary {
  background: var(--bg-input);
  color: var(--text);
  border: 1px solid var(--border);
  margin-bottom: 6px;
}
.btn-secondary:hover { background: var(--bg-panel-alt); border-color: var(--border-hover); }

#progress-container {
  display: none;
  margin-top: 8px;
}
#progress-container.visible { display: block; }
#progress-bar-outer {
  background: var(--bg-input);
  border-radius: 4px;
  height: 8px;
  overflow: hidden;
  margin-bottom: 4px;
}
#progress-bar {
  background: var(--accent);
  height: 100%;
  width: 0%;
  transition: width 0.3s ease;
  border-radius: 4px;
}
#progress-bar.complete { background: var(--green); }
#progress-bar.error { background: var(--red); }
#progress-text {
  font-size: 10px;
  color: var(--text-dim);
  text-align: center;
}

.error-list {
  max-height: 120px;
  overflow-y: auto;
}
.error-item {
  padding: 4px 8px;
  font-size: 10px;
  color: var(--red);
  background: rgba(231, 76, 60, 0.1);
  border-radius: 3px;
  margin-bottom: 3px;
}
.success-item {
  padding: 4px 8px;
  font-size: 10px;
  color: var(--green);
  background: rgba(46, 204, 113, 0.1);
  border-radius: 3px;
}

/* ─── Status bar ─── */
#status-bar {
  background: var(--bg-panel);
  border-top: 1px solid var(--border);
  padding: 4px 16px;
  font-size: 11px;
  display: flex;
  gap: 24px;
  min-height: 26px;
  align-items: center;
  flex-shrink: 0;
}
#status-bar .status-text { color: var(--text-dim); }
#status-bar .hover-info { color: var(--accent); flex: 1; }

/* ─── Variant tabs ─── */
.variant-tab {
  flex: 1;
  padding: 8px 4px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg-input);
  color: var(--text-dim);
  font-family: inherit;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  text-align: center;
  transition: all 0.15s;
}
.variant-tab:hover { border-color: var(--border-hover); color: var(--text); }
.variant-tab.active { border-color: var(--accent); color: var(--accent); background: rgba(233,69,96,0.1); }
.variant-score-row {
  display: flex;
  justify-content: space-between;
  padding: 2px 0;
  font-size: 10px;
}
.variant-score-row .label { color: var(--text-dim); }
.variant-score-row .value { color: var(--text-bright); font-weight: 500; }
</style>
</head>
<body>

<!-- Header -->
<div id="header">
  <h1>pcb-forge</h1>
  <div class="sep"></div>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="edge-cuts"><span class="swatch" style="background:#cccc00"></span>Edge</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="fcu"><span class="swatch" style="background:#ff3333"></span>F.Cu</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="bcu"><span class="swatch" style="background:#4444ff"></span>B.Cu</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="pads"><span class="swatch" style="background:#c8a84e"></span>Pads</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="vias"><span class="swatch" style="background:#888"></span>Vias</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="zones"><span class="swatch" style="background:rgba(100,100,255,0.3)"></span>Zones</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="silkscreen"><span class="swatch" style="background:#fff"></span>Silk</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="courtyard"><span class="swatch" style="background:#666"></span>Court</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="ratsnest"><span class="swatch" style="background:#ffaa00"></span>Ratsnest</label>
  <div class="sep"></div>
  <label class="layer-toggle">Grid:
    <select id="grid-snap" class="grid-select">
      <option value="0">Off</option>
      <option value="0.5">0.5mm</option>
      <option value="1" selected>1mm</option>
      <option value="2.54">2.54mm</option>
    </select>
  </label>
  <span id="coords-display">X: — Y: —</span>
</div>

<!-- Main layout -->
<div id="main">
  <!-- Left panel -->
  <div id="left-panel">
    <div class="panel-header">Components <span class="count" id="comp-count">0</span></div>
    <div id="component-list"></div>
  </div>

  <!-- Center canvas -->
  <div id="center-panel">
    <svg id="pcb-canvas" xmlns="http://www.w3.org/2000/svg"></svg>
  </div>

  <!-- Right panel -->
  <div id="right-panel">
    <div class="info-section">
      <h3>Board</h3>
      <div class="info-row">
        <span class="label">Width</span>
        <span><input type="number" id="board-width" step="0.5" min="1"> mm</span>
      </div>
      <div class="info-row">
        <span class="label">Height</span>
        <span><input type="number" id="board-height" step="0.5" min="1"> mm</span>
      </div>
    </div>

    <div class="info-section">
      <h3>Statistics</h3>
      <div class="stat-grid">
        <div class="stat-card"><div class="stat-value" id="stat-components">0</div><div class="stat-label">Components</div></div>
        <div class="stat-card"><div class="stat-value" id="stat-nets">0</div><div class="stat-label">Nets</div></div>
        <div class="stat-card"><div class="stat-value" id="stat-traces">0</div><div class="stat-label">Traces</div></div>
        <div class="stat-card"><div class="stat-value" id="stat-vias">0</div><div class="stat-label">Vias</div></div>
      </div>
    </div>

    <div class="info-section" id="errors-section" style="display:none">
      <h3>Routing Errors</h3>
      <div class="error-list" id="error-list"></div>
    </div>

    <div class="info-section" id="variants-section" style="display:none">
      <h3>Placement Variants</h3>
      <div id="variant-tabs" style="display:flex;gap:4px;margin-bottom:8px;"></div>
      <div id="variant-scores"></div>
    </div>

    <div class="info-section">
      <h3>Actions</h3>
      <button class="btn btn-build" id="btn-build" onclick="triggerBuild()">BUILD</button>
      <div id="progress-container">
        <div id="progress-bar-outer"><div id="progress-bar"></div></div>
        <div id="progress-text"></div>
      </div>
      <button class="btn btn-secondary" onclick="exportToml()">Export TOML</button>
      <button class="btn btn-secondary" onclick="downloadZip()">Download ZIP</button>
    </div>
  </div>
</div>

<!-- Status bar -->
<div id="status-bar">
  <span class="hover-info" id="hover-info">Ready</span>
  <span class="status-text" id="board-dims"></span>
</div>

<script>
// ─── State ───
let boardData = null;
let routedNets = null;
let selectedRef = null;
let variants = [];
let selectedVariant = 0;
let viewBox = { x: 0, y: 0, w: 100, h: 100 };
let isPanning = false;
let panStart = { x: 0, y: 0 };
let isDragging = false;
let dragRef = null;
let dragStartSvg = { x: 0, y: 0 };
let dragStartComp = { x: 0, y: 0 };
const svgNS = 'http://www.w3.org/2000/svg';

// ─── Init ───
async function init() {
  await loadBoard();
  setupEvents();
}

async function loadBoard() {
  try {
    const resp = await fetch('/api/board');
    const data = await resp.json();
    boardData = data.board;
    routedNets = data.routed_nets;
    variants = data.variants || [];
    selectedVariant = data.selected_variant || 0;
    renderAll();
    renderVariants();
  } catch (e) {
    console.error('Failed to load board:', e);
    document.getElementById('hover-info').textContent = 'Error loading board data';
  }
}

function renderVariants() {
  const section = document.getElementById('variants-section');
  const tabs = document.getElementById('variant-tabs');
  const scores = document.getElementById('variant-scores');

  if (!variants || variants.length === 0) {
    section.style.display = 'none';
    return;
  }

  section.style.display = 'block';
  tabs.innerHTML = '';
  scores.innerHTML = '';

  variants.forEach((v, i) => {
    const btn = document.createElement('button');
    btn.className = 'variant-tab' + (i === selectedVariant ? ' active' : '');
    btn.textContent = '#' + (i + 1);
    btn.title = `Score: ${v.score.composite.toFixed(0)}`;
    btn.onclick = () => selectVariant(i);
    tabs.appendChild(btn);
  });

  // Show score details for selected variant
  const v = variants[selectedVariant];
  if (v) {
    scores.innerHTML = `
      <div class="variant-score-row"><span class="label">Nets routed</span><span class="value">${v.score.nets_routed}/${v.score.total_nets}</span></div>
      <div class="variant-score-row"><span class="label">Trace length</span><span class="value">${v.score.total_trace_length.toFixed(1)}mm</span></div>
      <div class="variant-score-row"><span class="label">Vias</span><span class="value">${v.score.via_count}</span></div>
      <div class="variant-score-row"><span class="label">Score</span><span class="value" style="color:var(--accent)">${v.score.composite.toFixed(0)}</span></div>
    `;
  }
}

async function selectVariant(idx) {
  selectedVariant = idx;
  try {
    await fetch('/api/variant/' + idx + '/select', { method: 'POST' });
    await loadBoard();
  } catch (e) {
    console.error('Failed to select variant:', e);
  }
}

// ─── Rendering ───
function renderAll() {
  if (!boardData) return;
  renderComponentList();
  renderCanvas();
  renderStats();
  renderErrors();
}

function renderComponentList() {
  const list = document.getElementById('component-list');
  list.innerHTML = '';
  document.getElementById('comp-count').textContent = boardData.components.length;

  boardData.components.forEach(comp => {
    const div = document.createElement('div');
    div.className = 'comp-item' + (selectedRef === comp.ref_des ? ' selected' : '');
    div.dataset.ref = comp.ref_des;
    const fpShort = comp.footprint.split('/').pop() || comp.footprint;
    div.innerHTML = `
      <div class="comp-header">
        <span class="ref-des">${comp.ref_des}</span>
        <span class="value">${comp.value}</span>
      </div>
      <div class="footprint" title="${comp.footprint}">${fpShort}</div>
      <div class="comp-fields">
        <div class="comp-field">
          <label>X</label>
          <input type="number" step="0.1" value="${comp.x.toFixed(2)}" data-ref="${comp.ref_des}" data-field="x">
        </div>
        <div class="comp-field">
          <label>Y</label>
          <input type="number" step="0.1" value="${comp.y.toFixed(2)}" data-ref="${comp.ref_des}" data-field="y">
        </div>
        <div class="comp-field">
          <label>Rot</label>
          <input type="number" step="45" value="${comp.rotation.toFixed(1)}" data-ref="${comp.ref_des}" data-field="rotation">
        </div>
      </div>
    `;

    div.addEventListener('click', (e) => {
      if (e.target.tagName === 'INPUT') return;
      selectComponent(comp.ref_des);
    });

    // Input change handlers
    div.querySelectorAll('input').forEach(input => {
      input.addEventListener('change', async (e) => {
        const ref = e.target.dataset.ref;
        const field = e.target.dataset.field;
        const val = parseFloat(e.target.value);
        if (isNaN(val)) return;
        const body = {};
        body[field] = val;
        await updateComponent(ref, body);
      });
      input.addEventListener('click', (e) => e.stopPropagation());
    });

    list.appendChild(div);
  });
}

function selectComponent(ref) {
  selectedRef = selectedRef === ref ? null : ref;
  renderComponentList();
  highlightComponent(selectedRef);

  // Scroll to component in list
  if (selectedRef) {
    const el = document.querySelector(`.comp-item[data-ref="${selectedRef}"]`);
    if (el) el.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }
}

function highlightComponent(ref) {
  // Remove existing highlights
  document.querySelectorAll('.comp-highlight').forEach(el => el.remove());

  if (!ref || !boardData) return;
  const comp = boardData.components.find(c => c.ref_des === ref);
  if (!comp || !comp.footprint_data) return;

  const svg = document.getElementById('pcb-canvas');
  const bounds = getCompBounds(comp);

  const rect = document.createElementNS(svgNS, 'rect');
  rect.setAttribute('x', bounds.x - 0.5);
  rect.setAttribute('y', bounds.y - 0.5);
  rect.setAttribute('width', bounds.w + 1);
  rect.setAttribute('height', bounds.h + 1);
  rect.setAttribute('fill', 'none');
  rect.setAttribute('stroke', '#e94560');
  rect.setAttribute('stroke-width', '0.2');
  rect.setAttribute('stroke-dasharray', '0.5,0.3');
  rect.setAttribute('class', 'comp-highlight');
  rect.setAttribute('pointer-events', 'none');
  svg.appendChild(rect);
}

function getCompBounds(comp) {
  if (comp.footprint_data) {
    const pads = comp.footprint_data.pads;
    if (pads.length > 0) {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      pads.forEach(p => {
        const [rx, ry] = rotatePoint(p.at_x, p.at_y, comp.rotation);
        const [sw, sh] = rotateSize(p.size_w, p.size_h, comp.rotation);
        minX = Math.min(minX, rx - sw/2);
        minY = Math.min(minY, ry - sh/2);
        maxX = Math.max(maxX, rx + sw/2);
        maxY = Math.max(maxY, ry + sh/2);
      });
      return { x: comp.x + minX, y: comp.y + minY, w: maxX - minX, h: maxY - minY };
    }
  }
  return { x: comp.x - 2, y: comp.y - 2, w: 4, h: 4 };
}

function rotatePoint(x, y, degrees) {
  if (Math.abs(degrees) < 0.01) return [x, y];
  const rad = degrees * Math.PI / 180;
  return [x * Math.cos(rad) - y * Math.sin(rad), x * Math.sin(rad) + y * Math.cos(rad)];
}

function rotateSize(w, h, degrees) {
  const d = ((degrees % 360) + 360) % 360;
  if (Math.abs(d - 90) < 1 || Math.abs(d - 270) < 1) return [h, w];
  return [w, h];
}

// ─── SVG Canvas Rendering ───
function renderCanvas() {
  if (!boardData) return;
  const svg = document.getElementById('pcb-canvas');

  const margin = 5;
  viewBox = {
    x: -margin,
    y: -margin,
    w: boardData.width + margin * 2,
    h: boardData.height + margin * 2
  };
  updateViewBox();

  // Clear and rebuild
  svg.innerHTML = '';

  // Defs
  const defs = document.createElementNS(svgNS, 'defs');
  defs.innerHTML = `
    <pattern id="grid-mm" width="1" height="1" patternUnits="userSpaceOnUse">
      <path d="M 1 0 L 0 0 0 1" fill="none" stroke="rgba(255,255,255,0.04)" stroke-width="0.02"/>
    </pattern>
    <pattern id="grid-5mm" width="5" height="5" patternUnits="userSpaceOnUse">
      <rect width="5" height="5" fill="url(#grid-mm)"/>
      <path d="M 5 0 L 0 0 0 5" fill="none" stroke="rgba(255,255,255,0.08)" stroke-width="0.03"/>
    </pattern>
  `;
  svg.appendChild(defs);

  // Background
  addRect(svg, viewBox.x, viewBox.y, viewBox.w, viewBox.h, '#0d3320');
  // PCB body
  addRect(svg, 0, 0, boardData.width, boardData.height, '#1a5c36', null, null, 0.5);
  // Grid
  addRect(svg, 0, 0, boardData.width, boardData.height, 'url(#grid-5mm)');

  // Layer groups
  const gZones = createGroup(svg, 'layer-zones');
  const gBcu = createGroup(svg, 'layer-bcu');
  const gFcu = createGroup(svg, 'layer-fcu');
  const gVias = createGroup(svg, 'layer-vias');
  const gCourtyard = createGroup(svg, 'layer-courtyard');
  const gPads = createGroup(svg, 'layer-pads');
  const gSilkscreen = createGroup(svg, 'layer-silkscreen');
  const gEdge = createGroup(svg, 'layer-edge-cuts');
  const gRatsnest = createGroup(svg, 'layer-ratsnest');
  const gDragTargets = createGroup(svg, 'layer-drag-targets');

  // Render zones
  renderZones(gZones);
  // Render traces
  renderTraces(gBcu, 1, '#4444ff');
  renderTraces(gFcu, 0, '#ff3333');
  // Render vias
  renderVias(gVias);
  // Render courtyards
  renderCourtyards(gCourtyard);
  // Render pads
  renderPads(gPads);
  // Render silkscreen
  renderSilkscreen(gSilkscreen);
  // Board outline
  const outline = document.createElementNS(svgNS, 'rect');
  outline.setAttribute('x', '0'); outline.setAttribute('y', '0');
  outline.setAttribute('width', boardData.width); outline.setAttribute('height', boardData.height);
  outline.setAttribute('fill', 'none'); outline.setAttribute('stroke', '#cccc00'); outline.setAttribute('stroke-width', '0.15');
  gEdge.appendChild(outline);
  // Ratsnest
  renderRatsnest(gRatsnest);
  // Drag targets (invisible rectangles for each component to make drag easier)
  renderDragTargets(gDragTargets);

  // Board size inputs
  document.getElementById('board-width').value = boardData.width;
  document.getElementById('board-height').value = boardData.height;
  document.getElementById('board-dims').textContent = `Board: ${boardData.width}mm × ${boardData.height}mm`;

  // Restore highlight
  if (selectedRef) highlightComponent(selectedRef);
}

function createGroup(parent, id) {
  const g = document.createElementNS(svgNS, 'g');
  g.id = id;
  parent.appendChild(g);
  return g;
}

function addRect(parent, x, y, w, h, fill, stroke, strokeWidth, rx) {
  const r = document.createElementNS(svgNS, 'rect');
  r.setAttribute('x', x); r.setAttribute('y', y);
  r.setAttribute('width', w); r.setAttribute('height', h);
  r.setAttribute('fill', fill || 'none');
  if (stroke) { r.setAttribute('stroke', stroke); r.setAttribute('stroke-width', strokeWidth || '0.1'); }
  if (rx) { r.setAttribute('rx', rx); r.setAttribute('ry', rx); }
  parent.appendChild(r);
  return r;
}

function renderZones(g) {
  const hasGnd = boardData.nets.some(n => n.name === 'GND');
  const hasVcc = boardData.nets.some(n => n.name === 'VCC3V3');
  if (hasGnd) addRect(g, 0, 0, boardData.width, boardData.height, 'rgba(68,68,255,0.12)');
  if (hasVcc) addRect(g, 0, 0, boardData.width, boardData.height, 'rgba(255,68,68,0.08)');
}

function renderTraces(g, layer, color) {
  if (!routedNets) return;
  routedNets.forEach(rn => {
    rn.segments.forEach(seg => {
      if (seg.layer !== layer) return;
      const line = document.createElementNS(svgNS, 'line');
      line.setAttribute('x1', seg.start[0].toFixed(3));
      line.setAttribute('y1', seg.start[1].toFixed(3));
      line.setAttribute('x2', seg.end[0].toFixed(3));
      line.setAttribute('y2', seg.end[1].toFixed(3));
      line.setAttribute('stroke', color);
      line.setAttribute('stroke-width', seg.width.toFixed(3));
      line.setAttribute('stroke-linecap', 'round');
      line.dataset.info = `Trace: ${rn.name} | Layer: ${layer === 0 ? 'F.Cu' : 'B.Cu'} | Width: ${seg.width.toFixed(2)}mm`;
      g.appendChild(line);
    });
  });
}

function renderVias(g) {
  if (!routedNets) return;
  routedNets.forEach(rn => {
    rn.vias.forEach(via => {
      const c1 = document.createElementNS(svgNS, 'circle');
      c1.setAttribute('cx', via.x.toFixed(3)); c1.setAttribute('cy', via.y.toFixed(3));
      c1.setAttribute('r', (via.size / 2).toFixed(3));
      c1.setAttribute('fill', '#888'); c1.setAttribute('stroke', '#aaa'); c1.setAttribute('stroke-width', '0.05');
      c1.dataset.info = `Via: ${rn.name} | Drill: ${via.drill.toFixed(2)}mm`;
      g.appendChild(c1);

      const c2 = document.createElementNS(svgNS, 'circle');
      c2.setAttribute('cx', via.x.toFixed(3)); c2.setAttribute('cy', via.y.toFixed(3));
      c2.setAttribute('r', (via.drill / 2).toFixed(3));
      c2.setAttribute('fill', '#1a5c36');
      g.appendChild(c2);
    });
  });
}

function renderCourtyards(g) {
  boardData.components.forEach(comp => {
    if (!comp.footprint_data) return;
    const fp = comp.footprint_data;
    const crtyLines = fp.lines.filter(l => l.layer.includes('CrtYd') || l.layer.includes('Fab'));

    if (crtyLines.length === 0) {
      // Fallback bounding box from pads
      const bounds = getCompBounds(comp);
      addRect(g, bounds.x, bounds.y, bounds.w, bounds.h, 'none', '#666', '0.08');
    } else {
      crtyLines.forEach(line => {
        const [sx, sy] = rotatePoint(line.start[0], line.start[1], comp.rotation);
        const [ex, ey] = rotatePoint(line.end[0], line.end[1], comp.rotation);
        const l = document.createElementNS(svgNS, 'line');
        l.setAttribute('x1', (comp.x + sx).toFixed(3));
        l.setAttribute('y1', (comp.y + sy).toFixed(3));
        l.setAttribute('x2', (comp.x + ex).toFixed(3));
        l.setAttribute('y2', (comp.y + ey).toFixed(3));
        l.setAttribute('stroke', '#666'); l.setAttribute('stroke-width', '0.08');
        l.dataset.info = `${comp.ref_des} (${comp.value})`;
        g.appendChild(l);
      });
    }
  });
}

function renderPads(g) {
  // Build pin-to-net map
  const pinNetMap = {};
  boardData.nets.forEach(net => {
    net.pins.forEach(pref => {
      const comp = boardData.components.find(c => c.name === pref.component);
      if (comp) {
        const pin = comp.pins.find(p => p.name === pref.pin);
        if (pin) pinNetMap[`${comp.name}:${pin.number}`] = net.name;
      }
    });
  });

  boardData.components.forEach(comp => {
    if (!comp.footprint_data) return;
    comp.footprint_data.pads.forEach(pad => {
      if (!pad.layers.some(l => l.includes('Cu'))) return;

      const [rx, ry] = rotatePoint(pad.at_x, pad.at_y, comp.rotation);
      const px = comp.x + rx;
      const py = comp.y + ry;

      const netName = pinNetMap[`${comp.name}:${pad.number}`] || 'unconnected';
      const pinName = comp.pins.find(p => p.number === pad.number)?.name || pad.number;
      const info = `${comp.ref_des}.${pinName} (pad ${pad.number}) | Net: ${netName} | ${pad.size_w.toFixed(2)}×${pad.size_h.toFixed(2)}mm`;

      const hasWildcard = pad.layers.some(l => l === '*.Cu');
      const isFront = hasWildcard || pad.layers.some(l => l === 'F.Cu');
      const isBack = hasWildcard || pad.layers.some(l => l === 'B.Cu');
      const color = (isFront && isBack) ? '#c8a84e' : isFront ? '#d4564e' : isBack ? '#5e6ed4' : '#c8a84e';
      const [sw, sh] = rotateSize(pad.size_w, pad.size_h, comp.rotation);

      if (pad.shape === 'circle') {
        const r = Math.max(sw, sh) / 2;
        const c = document.createElementNS(svgNS, 'circle');
        c.setAttribute('cx', px.toFixed(3)); c.setAttribute('cy', py.toFixed(3));
        c.setAttribute('r', r.toFixed(3));
        c.setAttribute('fill', color); c.setAttribute('opacity', '0.85');
        c.dataset.info = info;
        g.appendChild(c);
      } else {
        const cornerR = (pad.shape === 'roundrect' || pad.shape === 'oval') ? Math.min(Math.min(sw, sh) * 0.25, 0.3) : 0.05;
        const rect = document.createElementNS(svgNS, 'rect');
        rect.setAttribute('x', (px - sw/2).toFixed(3));
        rect.setAttribute('y', (py - sh/2).toFixed(3));
        rect.setAttribute('width', sw.toFixed(3));
        rect.setAttribute('height', sh.toFixed(3));
        rect.setAttribute('rx', cornerR.toFixed(3));
        rect.setAttribute('fill', color); rect.setAttribute('opacity', '0.85');
        rect.dataset.info = info;
        g.appendChild(rect);
      }

      // Drill hole
      if (pad.drill) {
        const hole = document.createElementNS(svgNS, 'circle');
        hole.setAttribute('cx', px.toFixed(3)); hole.setAttribute('cy', py.toFixed(3));
        hole.setAttribute('r', (pad.drill / 2).toFixed(3));
        hole.setAttribute('fill', '#1a5c36');
        g.appendChild(hole);
      }
    });
  });
}

function renderSilkscreen(g) {
  boardData.components.forEach(comp => {
    if (comp.footprint_data) {
      comp.footprint_data.lines.forEach(line => {
        if (!line.layer.includes('SilkS')) return;
        const [sx, sy] = rotatePoint(line.start[0], line.start[1], comp.rotation);
        const [ex, ey] = rotatePoint(line.end[0], line.end[1], comp.rotation);
        const l = document.createElementNS(svgNS, 'line');
        l.setAttribute('x1', (comp.x + sx).toFixed(3));
        l.setAttribute('y1', (comp.y + sy).toFixed(3));
        l.setAttribute('x2', (comp.x + ex).toFixed(3));
        l.setAttribute('y2', (comp.y + ey).toFixed(3));
        l.setAttribute('stroke', 'white'); l.setAttribute('stroke-width', Math.max(line.width, 0.1).toFixed(3));
        l.setAttribute('stroke-linecap', 'round'); l.setAttribute('opacity', '0.9');
        g.appendChild(l);
      });
    }

    // Ref des label
    let labelY = comp.y - 2;
    if (comp.footprint_data) {
      const bounds = getCompBounds(comp);
      labelY = bounds.y - 0.5;
    }
    const text = document.createElementNS(svgNS, 'text');
    text.setAttribute('x', comp.x.toFixed(3));
    text.setAttribute('y', labelY.toFixed(3));
    text.setAttribute('fill', 'white'); text.setAttribute('font-size', '1.2');
    text.setAttribute('font-family', 'sans-serif'); text.setAttribute('text-anchor', 'middle');
    text.setAttribute('opacity', '0.9');
    text.dataset.info = `${comp.ref_des} (${comp.value})`;
    text.textContent = comp.ref_des;
    g.appendChild(text);
  });
}

function renderRatsnest(g) {
  if (routedNets) return; // Don't show ratsnest if already routed

  // Build pin absolute positions
  const pinPositions = {};
  boardData.components.forEach(comp => {
    comp.pins.forEach(pin => {
      let px = comp.x, py = comp.y;
      if (comp.footprint_data) {
        const pad = comp.footprint_data.pads.find(p => p.number === pin.number);
        if (pad) {
          const [rx, ry] = rotatePoint(pad.at_x, pad.at_y, comp.rotation);
          px = comp.x + rx;
          py = comp.y + ry;
        }
      }
      pinPositions[`${comp.name}:${pin.name}`] = { x: px, y: py };
    });
  });

  boardData.nets.forEach(net => {
    if (net.pins.length < 2) return;
    // Star topology from first pin
    const first = pinPositions[`${net.pins[0].component}:${net.pins[0].pin}`];
    if (!first) return;

    for (let i = 1; i < net.pins.length; i++) {
      const pos = pinPositions[`${net.pins[i].component}:${net.pins[i].pin}`];
      if (!pos) continue;
      const line = document.createElementNS(svgNS, 'line');
      line.setAttribute('x1', first.x.toFixed(3));
      line.setAttribute('y1', first.y.toFixed(3));
      line.setAttribute('x2', pos.x.toFixed(3));
      line.setAttribute('y2', pos.y.toFixed(3));
      line.setAttribute('stroke', '#ffaa00');
      line.setAttribute('stroke-width', '0.08');
      line.setAttribute('stroke-dasharray', '0.3,0.2');
      line.setAttribute('opacity', '0.6');
      line.dataset.info = `Ratsnest: ${net.name}`;
      g.appendChild(line);
    }
  });
}

function renderDragTargets(g) {
  boardData.components.forEach(comp => {
    const bounds = getCompBounds(comp);
    const rect = document.createElementNS(svgNS, 'rect');
    rect.setAttribute('x', bounds.x);
    rect.setAttribute('y', bounds.y);
    rect.setAttribute('width', bounds.w);
    rect.setAttribute('height', bounds.h);
    rect.setAttribute('fill', 'transparent');
    rect.setAttribute('stroke', 'none');
    rect.setAttribute('cursor', 'move');
    rect.dataset.ref = comp.ref_des;
    rect.dataset.info = `${comp.ref_des} (${comp.value}) — drag to move`;
    rect.addEventListener('mousedown', (e) => startDrag(e, comp.ref_des));
    g.appendChild(rect);
  });
}

// ─── Stats ───
function renderStats() {
  document.getElementById('stat-components').textContent = boardData.components.length;
  document.getElementById('stat-nets').textContent = boardData.nets.length;

  let traceCount = 0, viaCount = 0;
  if (routedNets) {
    routedNets.forEach(rn => {
      traceCount += rn.segments.length;
      viaCount += rn.vias.length;
    });
  }
  document.getElementById('stat-traces').textContent = traceCount;
  document.getElementById('stat-vias').textContent = viaCount;
}

function renderErrors() {
  const section = document.getElementById('errors-section');
  const list = document.getElementById('error-list');
  list.innerHTML = '';

  if (!routedNets) {
    section.style.display = 'block';
    const div = document.createElement('div');
    div.className = 'error-item';
    div.textContent = 'No build yet — click BUILD to route traces';
    list.appendChild(div);
    return;
  }

  // Find unrouted nets
  const routedNames = new Set(routedNets.filter(rn => rn.segments.length > 0).map(rn => rn.name));
  const unrouted = boardData.nets.filter(n => n.pins.length >= 2 && !routedNames.has(n.name));

  if (unrouted.length === 0) {
    section.style.display = 'block';
    const div = document.createElement('div');
    div.className = 'success-item';
    div.textContent = 'All nets routed successfully!';
    list.appendChild(div);
  } else {
    section.style.display = 'block';
    unrouted.forEach(net => {
      const div = document.createElement('div');
      div.className = 'error-item';
      div.textContent = `Unrouted: ${net.name} (${net.pins.length} pins)`;
      list.appendChild(div);
    });
  }
}

// ─── Pan & Zoom ───
function updateViewBox() {
  const svg = document.getElementById('pcb-canvas');
  svg.setAttribute('viewBox', `${viewBox.x} ${viewBox.y} ${viewBox.w} ${viewBox.h}`);
}

function svgCoords(e) {
  const svg = document.getElementById('pcb-canvas');
  const rect = svg.getBoundingClientRect();
  return {
    x: viewBox.x + (e.clientX - rect.left) / rect.width * viewBox.w,
    y: viewBox.y + (e.clientY - rect.top) / rect.height * viewBox.h
  };
}

function setupEvents() {
  const center = document.getElementById('center-panel');
  const svg = document.getElementById('pcb-canvas');

  // Zoom
  center.addEventListener('wheel', (e) => {
    e.preventDefault();
    const rect = svg.getBoundingClientRect();
    const mx = (e.clientX - rect.left) / rect.width;
    const my = (e.clientY - rect.top) / rect.height;
    const factor = e.deltaY > 0 ? 1.12 : 1 / 1.12;
    const newW = viewBox.w * factor;
    const newH = viewBox.h * factor;
    viewBox.x += (viewBox.w - newW) * mx;
    viewBox.y += (viewBox.h - newH) * my;
    viewBox.w = newW;
    viewBox.h = newH;
    updateViewBox();
  }, { passive: false });

  // Pan
  center.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    if (isDragging) return;
    // Check if clicking on a drag target
    if (e.target.dataset && e.target.dataset.ref) return;
    isPanning = true;
    panStart = { x: e.clientX, y: e.clientY };
    center.classList.add('panning');
  });

  window.addEventListener('mousemove', (e) => {
    // Update coordinates
    const coords = svgCoords(e);
    document.getElementById('coords-display').textContent = `X: ${coords.x.toFixed(2)}mm  Y: ${coords.y.toFixed(2)}mm`;

    if (isPanning) {
      const rect = svg.getBoundingClientRect();
      const dx = (e.clientX - panStart.x) / rect.width * viewBox.w;
      const dy = (e.clientY - panStart.y) / rect.height * viewBox.h;
      viewBox.x -= dx;
      viewBox.y -= dy;
      panStart = { x: e.clientX, y: e.clientY };
      updateViewBox();
    }

    if (isDragging) {
      handleDrag(e);
    }
  });

  window.addEventListener('mouseup', (e) => {
    if (isPanning) {
      isPanning = false;
      center.classList.remove('panning');
    }
    if (isDragging) {
      endDrag(e);
    }
  });

  // Hover info
  svg.addEventListener('mouseover', (e) => {
    const el = e.target.closest('[data-info]');
    if (el) document.getElementById('hover-info').textContent = el.dataset.info;
  });
  svg.addEventListener('mouseout', (e) => {
    const el = e.target.closest('[data-info]');
    if (el) document.getElementById('hover-info').textContent = 'Ready';
  });

  // Layer toggles
  document.querySelectorAll('.layer-toggle input[data-layer]').forEach(cb => {
    cb.addEventListener('change', () => {
      const g = document.getElementById('layer-' + cb.dataset.layer);
      if (g) g.style.display = cb.checked ? '' : 'none';
    });
  });

  // Board size inputs
  document.getElementById('board-width').addEventListener('change', async (e) => {
    const val = parseFloat(e.target.value);
    if (isNaN(val) || val < 1) return;
    await fetch('/api/board/size', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ width: val })
    });
    await loadBoard();
  });

  document.getElementById('board-height').addEventListener('change', async (e) => {
    const val = parseFloat(e.target.value);
    if (isNaN(val) || val < 1) return;
    await fetch('/api/board/size', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ height: val })
    });
    await loadBoard();
  });

  // Keyboard shortcuts
  document.addEventListener('keydown', (e) => {
    if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
    if (e.key === 'f' || e.key === 'F') {
      // Fit to view
      const margin = 5;
      viewBox = {
        x: -margin,
        y: -margin,
        w: boardData.width + margin * 2,
        h: boardData.height + margin * 2
      };
      updateViewBox();
    }
    if (e.key === 'Escape') {
      selectComponent(null);
    }
  });
}

// ─── Drag & Drop ───
function startDrag(e, ref) {
  e.preventDefault();
  e.stopPropagation();
  isDragging = true;
  dragRef = ref;
  dragStartSvg = svgCoords(e);
  const comp = boardData.components.find(c => c.ref_des === ref);
  if (comp) {
    dragStartComp = { x: comp.x, y: comp.y };
  }
  selectComponent(ref);
  document.getElementById('center-panel').classList.add('dragging-component');
}

function handleDrag(e) {
  if (!isDragging || !dragRef) return;
  const pos = svgCoords(e);
  const dx = pos.x - dragStartSvg.x;
  const dy = pos.y - dragStartSvg.y;

  let newX = dragStartComp.x + dx;
  let newY = dragStartComp.y + dy;

  // Grid snap
  const gridSnap = parseFloat(document.getElementById('grid-snap').value);
  if (gridSnap > 0) {
    newX = Math.round(newX / gridSnap) * gridSnap;
    newY = Math.round(newY / gridSnap) * gridSnap;
  }

  // Update local state immediately for smooth dragging
  const comp = boardData.components.find(c => c.ref_des === dragRef);
  if (comp) {
    comp.x = newX;
    comp.y = newY;
    renderCanvas();
  }
}

async function endDrag(e) {
  if (!isDragging || !dragRef) return;
  isDragging = false;
  document.getElementById('center-panel').classList.remove('dragging-component');

  const comp = boardData.components.find(c => c.ref_des === dragRef);
  if (comp) {
    await updateComponent(dragRef, { x: comp.x, y: comp.y });
  }
  dragRef = null;
}

// ─── API calls ───
async function updateComponent(ref, body) {
  try {
    await fetch(`/api/component/${encodeURIComponent(ref)}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    });
    // Reload to get consistent state
    await loadBoard();
  } catch (e) {
    console.error('Failed to update component:', e);
  }
}

async function triggerBuild() {
  const btn = document.getElementById('btn-build');
  const progressContainer = document.getElementById('progress-container');
  const progressBar = document.getElementById('progress-bar');
  const progressText = document.getElementById('progress-text');

  btn.disabled = true;
  btn.textContent = 'BUILDING...';
  progressContainer.classList.add('visible');
  progressBar.className = '';
  progressBar.style.width = '0%';
  progressText.textContent = 'Starting...';

  try {
    const resp = await fetch('/api/build', { method: 'POST' });

    if (resp.status === 409) {
      btn.disabled = false;
      btn.textContent = 'BUILD';
      progressText.textContent = 'Build already in progress';
      return;
    }

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop();

      for (const line of lines) {
        if (line.startsWith('data: ')) {
          try {
            const data = JSON.parse(line.substring(6));
            if (data.error) {
              progressBar.className = 'error';
              progressBar.style.width = '100%';
              progressText.textContent = `Error: ${data.error}`;
              btn.disabled = false;
              btn.textContent = 'BUILD';
              return;
            }
            if (data.progress !== undefined) {
              progressBar.style.width = data.progress + '%';
              progressText.textContent = data.step || '';
            }
            if (data.done) {
              progressBar.className = 'complete';
              progressBar.style.width = '100%';
              progressText.textContent = 'Build complete!';
              btn.textContent = 'REBUILD';
              btn.disabled = false;
              // Reload board to get routed nets
              await loadBoard();
              return;
            }
          } catch (parseErr) {
            // ignore parse errors in SSE
          }
        }
      }
    }
  } catch (e) {
    progressBar.className = 'error';
    progressText.textContent = `Error: ${e.message}`;
  }

  btn.disabled = false;
  btn.textContent = 'BUILD';
}

async function exportToml() {
  try {
    const resp = await fetch('/api/export/toml');
    const blob = await resp.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'circuit.toml';
    a.click();
    URL.revokeObjectURL(url);
    document.getElementById('hover-info').textContent = 'TOML exported!';
  } catch (e) {
    document.getElementById('hover-info').textContent = 'Export failed: ' + e.message;
  }
}

async function downloadZip() {
  try {
    const resp = await fetch('/api/export/zip');
    if (!resp.ok) {
      const data = await resp.json();
      document.getElementById('hover-info').textContent = data.error || 'Download failed';
      return;
    }
    const blob = await resp.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'jlcpcb.zip';
    a.click();
    URL.revokeObjectURL(url);
    document.getElementById('hover-info').textContent = 'ZIP downloaded!';
  } catch (e) {
    document.getElementById('hover-info').textContent = 'Download failed: ' + e.message;
  }
}

// ─── Start ───
init();
</script>
</body>
</html>"##;
