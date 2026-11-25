/**
 * Render a zero-dependency, self-contained interactive HTML simulator from `ik schema`.
 *
 * Advisory structural explorer only: never parses `.ik.toml` and never evaluates guards or
 * invariants itself. The Rust `ik` runtime remains the sole execution authority.
 */
import type { MachineSchema } from "./codegen.js";

function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

/** Escape JSON safely for embedding inside a `<script>` tag (blocks `</script>` breakouts). */
function safeInlineJson(value: unknown): string {
  return JSON.stringify(value)
    .replaceAll("<", "\\u003c")
    .replaceAll(">", "\\u003e")
    .replaceAll("&", "\\u0026")
    .replaceAll("\u2028", "\\u2028")
    .replaceAll("\u2029", "\\u2029");
}

export function renderSimulatorHtml(schema: MachineSchema): string {
  const title = `${schema.machine.machine_id} v${schema.machine.version} — Desmo interactive simulator`;
  const shortHash = schema.machine.spec_hash.slice(0, 12);
  const embedded = safeInlineJson(schema);

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title>
<style>
  :root {
    --paper: #FFFFFF;
    --ink: #111315;
    --sec: #465158;
    --surf: #F2F5F6;
    --surf2: #E5EBEE;
    --line: #CBD3D7;
    --accent: #FF671D;
    --accent-ink: #9B3400;
    --accent-soft: #FFF1EA;
    --ok: #136F41;
    --ok-soft: #ECF8F1;
    --mono: ui-monospace, "IBM Plex Mono", SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    --sans: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    background: var(--paper);
    color: var(--ink);
    font-family: var(--sans);
    line-height: 1.45;
  }
  .accent-strip { height: 5px; background: var(--accent); width: 100%; }
  .wrap { max-width: 1240px; margin: 0 auto; padding: 24px 28px 48px; }
  header {
    display: flex; flex-wrap: wrap; justify-content: space-between; align-items: flex-end;
    gap: 16px; padding-bottom: 18px; border-bottom: 1.5px solid var(--line);
  }
  .kicker {
    display: inline-flex; align-items: center; gap: 10px;
    font: 600 12px/1 var(--mono); letter-spacing: 0.06em; color: var(--accent-ink); text-transform: uppercase;
  }
  .kicker::before { content: ""; width: 18px; height: 3px; background: var(--accent); display: inline-block; }
  h1 { margin: 8px 0 4px; font: 600 30px/1.1 var(--sans); letter-spacing: -0.03em; }
  .meta { font: 500 13px/1.4 var(--mono); color: var(--sec); }
  .advisory {
    margin: 16px 0 22px; padding: 11px 16px; border-radius: 8px;
    background: var(--surf); border: 1px solid var(--line);
    font: 500 12.5px/1.5 var(--mono); color: var(--sec);
  }
  .advisory strong { color: var(--ink); }
  .grid { display: grid; grid-template-columns: minmax(0, 1.55fr) minmax(320px, 1fr); gap: 22px; align-items: start; }
  @media (max-width: 960px) { .grid { grid-template-columns: 1fr; } }
  .card {
    background: var(--paper); border: 1.5px solid var(--line); border-radius: 12px; overflow: hidden;
  }
  .card-head {
    display: flex; justify-content: space-between; align-items: center; gap: 12px;
    padding: 12px 16px; background: var(--surf); border-bottom: 1px solid var(--line);
    font: 600 12px/1 var(--mono); color: var(--sec); letter-spacing: 0.04em; text-transform: uppercase;
  }
  .card-body { padding: 16px; }
  .stage { background: var(--surf); padding: 12px; overflow-x: auto; }
  svg#graph { width: 100%; height: auto; display: block; min-height: 280px; }
  .status-bar {
    display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px;
    padding: 14px 16px; border-top: 1px solid var(--line); background: var(--paper);
  }
  .stat-label { font: 600 11px/1 var(--mono); color: var(--sec); text-transform: uppercase; margin-bottom: 4px; }
  .stat-val { font: 600 16px/1.2 var(--mono); color: var(--ink); word-break: break-all; }
  .banner {
    margin: 14px 16px 0; padding: 12px 14px; border-radius: 8px; display: none;
    font: 600 13px/1.45 var(--mono);
  }
  .banner.show { display: block; }
  .banner.refused { background: var(--accent-soft); border: 1.5px solid var(--accent); color: var(--accent-ink); }
  .banner.committed { background: var(--ok-soft); border: 1.5px solid var(--ok); color: var(--ok); }
  .section-title {
    font: 600 11.5px/1 var(--mono); color: var(--sec); text-transform: uppercase;
    letter-spacing: 0.05em; margin: 14px 0 8px;
  }
  .section-title:first-child { margin-top: 0; }
  .cmd-list { display: flex; flex-direction: column; gap: 8px; }
  button.cmd {
    width: 100%; text-align: left; padding: 10px 12px; border-radius: 8px;
    cursor: pointer; font: 600 13.5px/1.3 var(--mono);
    display: flex; justify-content: space-between; align-items: center; gap: 10px;
    transition: transform 0.08s ease, background 0.12s ease;
  }
  button.cmd:active { transform: scale(0.99); }
  button.cmd.legal {
    background: var(--ink); color: var(--paper); border: 1.5px solid var(--ink);
  }
  button.cmd.legal:hover { background: #23282D; }
  button.cmd.illegal {
    background: var(--paper); color: var(--accent-ink); border: 1.5px dashed var(--accent);
  }
  button.cmd.illegal:hover { background: var(--accent-soft); }
  .cmd-sub { font: 400 11.5px/1.2 var(--mono); opacity: 0.82; }
  .toolbar { display: flex; flex-wrap: wrap; gap: 8px; }
  button.tool {
    padding: 7px 11px; border-radius: 6px; border: 1.5px solid var(--ink);
    background: var(--paper); color: var(--ink); cursor: pointer;
    font: 600 12px/1 var(--mono);
  }
  button.tool:hover { background: var(--surf); }
  button.tool.stale { border-color: var(--accent); color: var(--accent-ink); }
  button.tool.stale:hover { background: var(--accent-soft); }
  button.tool:disabled { opacity: 0.4; cursor: not-allowed; }
  .log {
    max-height: 280px; overflow-y: auto; display: flex; flex-direction: column; gap: 6px;
    font: 500 12px/1.4 var(--mono);
  }
  .log-row {
    padding: 8px 10px; border-radius: 6px; border: 1px solid var(--line); background: var(--surf);
    display: flex; justify-content: space-between; gap: 10px; align-items: baseline;
  }
  .log-row.refused { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-ink); }
  .log-row.committed { background: var(--paper); border-color: var(--line); color: var(--ink); }
  .pill {
    display: inline-block; padding: 2px 7px; border-radius: 4px;
    font: 700 10.5px/1.2 var(--mono); text-transform: uppercase;
  }
  .pill.ok { background: var(--ink); color: var(--paper); }
  .pill.bad { background: var(--accent); color: var(--paper); }
  .facts-box { font: 500 12.5px/1.5 var(--mono); color: var(--sec); }
  .facts-box code { color: var(--ink); font-weight: 600; }
  .branch-pick {
    margin-top: 8px; padding: 10px; border-radius: 8px; background: var(--surf);
    border: 1px solid var(--line); display: none;
  }
  .branch-pick.show { display: block; }
  .branch-btns { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
</style>
</head>
<body>
<div class="accent-strip"></div>
<div class="wrap">
  <header>
    <div>
      <div class="kicker">Desmo · Interactive Structural Simulator</div>
      <h1 id="machine-title">${escapeHtml(schema.machine.machine_id)}</h1>
      <div class="meta">version ${escapeHtml(schema.machine.version)} · spec_hash ${escapeHtml(shortHash)} · ${schema.states.length} states · ${schema.commands.length} commands</div>
    </div>
    <div class="toolbar">
      <button class="tool" id="btn-save-handle" type="button">Copy Capability Handle</button>
      <button class="tool stale" id="btn-fire-stale" type="button" disabled>Fire Stale Handle</button>
      <button class="tool" id="btn-reset" type="button">Reset</button>
    </div>
  </header>

  <div class="advisory">
    <strong>Advisory structural simulation</strong> derived from <code>ik schema</code> (<code>${escapeHtml(shortHash)}</code>).
    Legal source states, terminal locks, and capability generations follow the declared machine shape; guard expressions, invariants, and durable commits are evaluated authoritatively by the Rust <code>ik</code> runtime.
  </div>

  <div class="grid">
    <div class="card">
      <div class="card-head">
        <span>State Graph · Positive-Drive Rails</span>
        <span id="stale-badge">No copied handle</span>
      </div>
      <div id="banner" class="banner" role="status"></div>
      <div class="stage">
        <svg id="graph" viewBox="0 0 880 340" aria-label="Interactive state machine graph"></svg>
      </div>
      <div class="status-bar">
        <div>
          <div class="stat-label">Current State</div>
          <div class="stat-val" id="stat-state"></div>
        </div>
        <div>
          <div class="stat-label">Capability</div>
          <div class="stat-val" id="stat-cap"></div>
        </div>
        <div>
          <div class="stat-label">Terminal Lock</div>
          <div class="stat-val" id="stat-term"></div>
        </div>
      </div>
    </div>

    <div style="display:flex; flex-direction:column; gap:18px;">
      <div class="card">
        <div class="card-head">
          <span>Propose Command</span>
          <span>Click legal or illegal</span>
        </div>
        <div class="card-body">
          <div class="section-title">Legal from current state</div>
          <div class="cmd-list" id="legal-cmds"></div>
          <div class="branch-pick" id="branch-pick">
            <div class="meta" id="branch-prompt"></div>
            <div class="branch-btns" id="branch-btns"></div>
          </div>
          <div class="section-title">Illegal from current state (click to test refusal)</div>
          <div class="cmd-list" id="illegal-cmds"></div>
        </div>
      </div>

      <div class="card">
        <div class="card-head">
          <span>Schema Metadata</span>
          <span id="inv-count"></span>
        </div>
        <div class="card-body facts-box" id="facts-box"></div>
      </div>

      <div class="card">
        <div class="card-head">
          <span>Execution Timeline</span>
          <span id="log-count">0 events</span>
        </div>
        <div class="card-body">
          <div class="log" id="event-log"></div>
        </div>
      </div>
    </div>
  </div>
</div>

<script>
(() => {
  const SCHEMA = ${embedded};
  const SVG_NS = "http://www.w3.org/2000/svg";
  const initialObj = SCHEMA.states.find((s) => s.initial) || SCHEMA.states[0];
  const stateMap = new Map(SCHEMA.states.map((s) => [s.name, s]));

  let current = initialObj ? initialObj.name : "";
  let generation = 0;
  let nonce = "n0-a1f4";
  let savedHandle = null;
  let lastRefusal = null;
  let pendingBranch = null;
  let events = [];

  function nextNonce(g) {
    const tag = ((g * 2654435761) >>> 0).toString(16).slice(0, 4).padStart(4, "0");
    return "n" + g + "-" + tag;
  }

  function computeLayout() {
    const depth = new Map();
    if (initialObj) {
      depth.set(initialObj.name, 0);
      const q = [initialObj.name];
      while (q.length > 0) {
        const u = q.shift();
        const d = depth.get(u);
        for (const cmd of SCHEMA.commands) {
          if (!cmd.from.includes(u)) continue;
          for (const v of cmd.successors) {
            if (!depth.has(v)) {
              depth.set(v, d + 1);
              q.push(v);
            }
          }
        }
      }
    }
    let maxD = 0;
    for (const d of depth.values()) maxD = Math.max(maxD, d);
    for (const st of SCHEMA.states) {
      if (!depth.has(st.name)) depth.set(st.name, maxD + 1);
    }

    const cols = new Map();
    for (const st of SCHEMA.states) {
      const d = depth.get(st.name);
      if (!cols.has(d)) cols.set(d, []);
      cols.get(d).push(st);
    }
    for (const list of cols.values()) {
      // Keep non-terminal spine states higher than early terminal branches (like cancelled)
      list.sort((a, b) => Number(a.terminal) - Number(b.terminal) || a.name.localeCompare(b.name));
    }

    const colKeys = Array.from(cols.keys()).sort((a, b) => a - b);
    const maxRows = Math.max(1, ...Array.from(cols.values()).map((c) => c.length));
    const colGap = 216;
    const rowGap = 136;
    const padX = 105;
    const padY = 92;
    const width = Math.max(880, padX * 2 + Math.max(1, colKeys.length - 1) * colGap);
    const height = Math.max(290, padY * 2 + Math.max(1, maxRows - 1) * rowGap + 40);

    const pos = new Map();
    colKeys.forEach((d, ci) => {
      const list = cols.get(d);
      list.forEach((st, ri) => {
        pos.set(st.name, { x: padX + ci * colGap, y: padY + ri * rowGap });
      });
    });
    return { pos, width, height };
  }

  const LAYOUT = computeLayout();

  function el(tag, attrs = {}, text) {
    const node = document.createElementNS(SVG_NS, tag);
    for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function renderGraph() {
    const svg = document.getElementById("graph");
    svg.setAttribute("viewBox", "0 0 " + LAYOUT.width + " " + LAYOUT.height);
    while (svg.firstChild) svg.removeChild(svg.firstChild);

    const defs = el("defs");
    const mInk = el("marker", { id: "arr-ink", viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" });
    mInk.appendChild(el("path", { d: "M 0 1 L 10 5 L 0 9 z", fill: "#111315" }));
    defs.appendChild(mInk);
    const mAcc = el("marker", { id: "arr-acc", viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" });
    mAcc.appendChild(el("path", { d: "M 0 1 L 10 5 L 0 9 z", fill: "#FF671D" }));
    defs.appendChild(mAcc);
    svg.appendChild(defs);

    const NW = 122, NH = 44;
    const edgeCounts = new Map();

    function drawEdgeLabel(lx, ly, text, active) {
      const w = Math.max(30, text.length * 7.2 + 10);
      svg.appendChild(el("rect", { x: lx - w / 2, y: ly - 10, width: w, height: 15, rx: 3, fill: "#F2F5F6" }));
      svg.appendChild(el("text", { x: lx, y: ly + 1.5, "text-anchor": "middle", "font-family": "var(--mono)", "font-size": 11.5, "font-weight": 600, fill: active ? "#111315" : "#465158" }, text));
    }

    // Draw declared edges
    for (const cmd of SCHEMA.commands) {
      for (const src of cmd.from) {
        const p1 = LAYOUT.pos.get(src);
        if (!p1) continue;
        for (const dst of cmd.successors) {
          const p2 = LAYOUT.pos.get(dst);
          if (!p2) continue;
          const key = src + "->" + dst;
          const idx = edgeCounts.get(key) || 0;
          edgeCounts.set(key, idx + 1);
          const isFromCurrent = src === current;
          const stroke = isFromCurrent ? "#111315" : "#8A969E";

          if (src === dst) {
            const lx = p1.x, ly = p1.y - NH / 2;
            const d = "M " + (lx - 18) + " " + ly + " C " + (lx - 34) + " " + (ly - 54) + ", " + (lx + 34) + " " + (ly - 54) + ", " + (lx + 18) + " " + ly;
            svg.appendChild(el("path", { d, fill: "none", stroke, "stroke-width": isFromCurrent ? 2.4 : 1.7, "marker-end": "url(#arr-ink)" }));
            drawEdgeLabel(lx, ly - 44, cmd.name, isFromCurrent);
          } else {
            const dx = p2.x - p1.x, dy = p2.y - p1.y;
            const dist = Math.hypot(dx, dy) || 1;
            const ux = dx / dist, uy = dy / dist;
            const sx = p1.x + ux * (NW / 2 + 2);
            const sy = p1.y + uy * (NH / 2 + 2);
            const ex = p2.x - ux * (NW / 2 + 6);
            const ey = p2.y - uy * (NH / 2 + 6);
            const isBack = p2.x < p1.x;
            const curve = isBack ? 56 + idx * 24 : (idx > 0 ? 28 * idx : 0);
            const mx = (sx + ex) / 2 - uy * curve;
            const my = (sy + ey) / 2 + ux * curve;
            const d = curve === 0
              ? "M " + sx + " " + sy + " L " + ex + " " + ey
              : "M " + sx + " " + sy + " Q " + mx + " " + my + " " + ex + " " + ey;
            svg.appendChild(el("path", { d, fill: "none", stroke, "stroke-width": isFromCurrent ? 2.4 : 1.7, "marker-end": "url(#arr-ink)" }));
            const lx = curve === 0 ? (sx + ex) / 2 : 0.25 * sx + 0.5 * mx + 0.25 * ex;
            const isHoriz = Math.abs(dy) < 10;
            const ly = (curve === 0 ? (sy + ey) / 2 : 0.25 * sy + 0.5 * my + 0.25 * ey) + (isHoriz ? -12 : 0);
            drawEdgeLabel(lx, ly, cmd.name, isFromCurrent);
          }
        }
      }
    }

    // Draw refused dashed stop-arc if last attempt was refused
    if (lastRefusal && lastRefusal.targetState) {
      const p1 = LAYOUT.pos.get(current);
      const p2 = LAYOUT.pos.get(lastRefusal.targetState);
      if (p1 && p2) {
        const mx = (p1.x + p2.x) / 2;
        const my = Math.min(p1.y, p2.y) - 62;
        const tStop = 0.48;
        const qx = (1 - tStop) * (1 - tStop) * p1.x + 2 * (1 - tStop) * tStop * mx + tStop * tStop * p2.x;
        const qy = (1 - tStop) * (1 - tStop) * (p1.y - NH / 2) + 2 * (1 - tStop) * tStop * my + tStop * tStop * (p2.y - NH / 2);
        const d = "M " + p1.x + " " + (p1.y - NH / 2) + " Q " + ((p1.x + mx) / 2) + " " + my + " " + qx + " " + qy;
        svg.appendChild(el("path", { d, fill: "none", stroke: "#FF671D", "stroke-width": 2.6, "stroke-dasharray": "6 5" }));
        svg.appendChild(el("line", { x1: qx, y1: qy - 14, x2: qx, y2: qy + 14, stroke: "#FF671D", "stroke-width": 4.5, "stroke-linecap": "round" }));
      }
    }

    // Draw state nodes
    for (const st of SCHEMA.states) {
      const p = LAYOUT.pos.get(st.name);
      if (!p) continue;
      const isCur = st.name === current;
      const rx = p.x - NW / 2, ry = p.y - NH / 2;

      if (st.terminal) {
        svg.appendChild(el("rect", { x: rx - 5, y: ry - 5, width: NW + 10, height: NH + 10, rx: 11, fill: "none", stroke: "#111315", "stroke-width": 1.6 }));
      }
      svg.appendChild(el("rect", {
        x: rx, y: ry, width: NW, height: NH, rx: 8,
        fill: isCur ? "#111315" : "#FFFFFF",
        stroke: isCur ? "#111315" : "#465158",
        "stroke-width": isCur ? 2.4 : 1.8,
      }));
      svg.appendChild(el("text", {
        x: p.x, y: p.y + 4.5, "text-anchor": "middle",
        "font-family": "var(--mono)", "font-size": 13.5, "font-weight": 600,
        fill: isCur ? "#FFFFFF" : "#111315",
      }, st.name));

      if (st.initial) {
        svg.appendChild(el("circle", { cx: rx - 12, cy: p.y, r: 4.5, fill: "#111315" }));
      }
      if (st.terminal) {
        svg.appendChild(el("text", {
          x: p.x, y: ry + NH + 18, "text-anchor": "middle",
          "font-family": "var(--mono)", "font-size": 10, "font-weight": 700, fill: "#465158",
        }, "TERMINAL"));
      }
    }
  }

  function setBanner(kind, message) {
    const b = document.getElementById("banner");
    b.className = "banner show " + kind;
    b.textContent = message;
  }

  function clearBanner() {
    const b = document.getElementById("banner");
    b.className = "banner";
    b.textContent = "";
  }

  function commitTransition(cmd, nextState) {
    const fromState = current;
    current = nextState;
    generation += 1;
    nonce = nextNonce(generation);
    lastRefusal = null;
    pendingBranch = null;
    events.unshift({
      kind: "committed",
      label: cmd.name + ": " + fromState + " → " + nextState,
      detail: "gen " + generation + (cmd.plans_effect ? " · effect planned" : ""),
    });
    setBanner("committed", "✓ COMMITTED: " + cmd.name + " (" + fromState + " → " + nextState + ") · capability advanced to gen " + generation);
    renderAll();
  }

  function refuseCommand(cmd, code, detail) {
    lastRefusal = { command: cmd.name, code, targetState: cmd.successors[0] || null };
    pendingBranch = null;
    events.unshift({
      kind: "refused",
      label: cmd.name + " refused (" + code + ")",
      detail: "stays " + current + " · gen " + generation,
    });
    setBanner("refused", "✕ REFUSED (" + code + "): " + detail + " Canonical state stays " + current + " (gen " + generation + ").");
    renderAll();
  }

  function onClickCommand(cmd) {
    const curState = stateMap.get(current);
    if (curState && curState.terminal) {
      refuseCommand(cmd, "IK_ILLEGAL_STATE", "state " + current + " is terminal; no further commands may commit.");
      return;
    }
    if (!cmd.from.includes(current)) {
      refuseCommand(cmd, "IK_ILLEGAL_STATE", "command " + cmd.name + " is legal only from [" + cmd.from.join(", ") + "], not " + current + ".");
      return;
    }
    if (cmd.successors.length > 1) {
      pendingBranch = cmd;
      renderAll();
      return;
    }
    commitTransition(cmd, cmd.successors[0] || current);
  }

  function renderCommands() {
    const legalEl = document.getElementById("legal-cmds");
    const illegalEl = document.getElementById("illegal-cmds");
    legalEl.textContent = "";
    illegalEl.textContent = "";

    const curState = stateMap.get(current);
    const isTerm = Boolean(curState && curState.terminal);

    for (const cmd of SCHEMA.commands) {
      const legal = !isTerm && cmd.from.includes(current);
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "cmd " + (legal ? "legal" : "illegal");

      const left = document.createElement("span");
      left.textContent = cmd.name + (cmd.args.length ? "(" + cmd.args.map((a) => a.name).join(", ") + ")" : "()");
      const right = document.createElement("span");
      right.className = "cmd-sub";
      right.textContent = legal
        ? "→ " + cmd.successors.join(" | ") + (cmd.plans_effect ? " · effect" : "")
        : "from [" + cmd.from.join(", ") + "]";
      btn.appendChild(left);
      btn.appendChild(right);
      btn.addEventListener("click", () => onClickCommand(cmd));
      (legal ? legalEl : illegalEl).appendChild(btn);
    }

    if (!legalEl.firstChild) {
      const empty = document.createElement("div");
      empty.className = "meta";
      empty.textContent = isTerm ? "Terminal state reached — all further commands are refused." : "No outgoing commands declared.";
      legalEl.appendChild(empty);
    }

    const pick = document.getElementById("branch-pick");
    const btns = document.getElementById("branch-btns");
    btns.textContent = "";
    if (pendingBranch) {
      pick.className = "branch-pick show";
      document.getElementById("branch-prompt").textContent =
        "Command " + pendingBranch.name + " has guarded outcomes (evaluated by ik at runtime). Pick a successor to explore:";
      for (const dst of pendingBranch.successors) {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "tool";
        b.textContent = "→ " + dst;
        b.addEventListener("click", () => commitTransition(pendingBranch, dst));
        btns.appendChild(b);
      }
    } else {
      pick.className = "branch-pick";
    }
  }

  function renderMeta() {
    document.getElementById("stat-state").textContent = current;
    document.getElementById("stat-cap").textContent = "gen " + generation + " · " + nonce;
    const isTerm = Boolean(stateMap.get(current) && stateMap.get(current).terminal);
    document.getElementById("stat-term").textContent = isTerm ? "LOCKED (terminal)" : "Open";

    const badge = document.getElementById("stale-badge");
    const fireBtn = document.getElementById("btn-fire-stale");
    if (!savedHandle) {
      badge.textContent = "No copied handle";
      fireBtn.disabled = true;
    } else if (savedHandle.generation === generation) {
      badge.textContent = "Copied handle: gen " + savedHandle.generation + " (still current — advance state first)";
      fireBtn.disabled = false;
    } else {
      badge.textContent = "Copied handle: gen " + savedHandle.generation + " (STALE vs gen " + generation + ")";
      fireBtn.disabled = false;
    }

    const factsBox = document.getElementById("facts-box");
    factsBox.textContent = "";
    const fLine = document.createElement("div");
    fLine.textContent = SCHEMA.facts.length
      ? "Facts: " + SCHEMA.facts.map((f) => f.name + ": " + f.type + " (" + f.policy + ")").join(" · ")
      : "Facts: none declared";
    factsBox.appendChild(fLine);
    const iLine = document.createElement("div");
    iLine.style.marginTop = "6px";
    iLine.textContent = SCHEMA.invariants.length
      ? "Invariants: " + SCHEMA.invariants.join(", ")
      : "Invariants: structural transition rules only";
    factsBox.appendChild(iLine);
    document.getElementById("inv-count").textContent = SCHEMA.invariants.length + " invariant(s)";

    const logEl = document.getElementById("event-log");
    logEl.textContent = "";
    document.getElementById("log-count").textContent = events.length + " event(s)";
    if (events.length === 0) {
      const empty = document.createElement("div");
      empty.className = "meta";
      empty.textContent = "Click any legal or illegal command to test the lifecycle boundary.";
      logEl.appendChild(empty);
    } else {
      for (const ev of events) {
        const row = document.createElement("div");
        row.className = "log-row " + ev.kind;
        const left = document.createElement("span");
        const pill = document.createElement("span");
        pill.className = "pill " + (ev.kind === "committed" ? "ok" : "bad");
        pill.textContent = ev.kind === "committed" ? "COMMIT" : "REFUSED";
        left.appendChild(pill);
        left.appendChild(document.createTextNode(" " + ev.label));
        const right = document.createElement("span");
        right.textContent = ev.detail;
        row.appendChild(left);
        row.appendChild(right);
        logEl.appendChild(row);
      }
    }
  }

  function renderAll() {
    renderGraph();
    renderCommands();
    renderMeta();
  }

  document.getElementById("btn-save-handle").addEventListener("click", () => {
    savedHandle = { state: current, generation, nonce };
    renderMeta();
  });

  document.getElementById("btn-fire-stale").addEventListener("click", () => {
    if (!savedHandle) return;
    const candidate = SCHEMA.commands.find((c) => c.from.includes(savedHandle.state)) || SCHEMA.commands[0];
    if (!candidate) return;
    if (savedHandle.generation !== generation) {
      refuseCommand(
        candidate,
        "IK_CAPABILITY_STALE",
        "copied handle held gen " + savedHandle.generation + " (" + savedHandle.nonce + "), but current aggregate is at gen " + generation + ".",
      );
    } else {
      setBanner("committed", "Copied handle is still at current gen " + generation + ". Advance the state first, then fire the copied handle to observe IK_CAPABILITY_STALE.");
    }
  });

  document.getElementById("btn-reset").addEventListener("click", () => {
    current = initialObj ? initialObj.name : "";
    generation = 0;
    nonce = nextNonce(0);
    savedHandle = null;
    lastRefusal = null;
    pendingBranch = null;
    events = [];
    clearBanner();
    renderAll();
  });

  renderAll();
})();
</script>
</body>
</html>
`;
}
