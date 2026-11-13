"use strict";
// Shared by the web page designer (designer.html) and the display designer (screens.html).
// A page defines `render()` (redraws everything from `schema`) and calls `startPolling()`.
const $ = (id) => document.getElementById(id);
const COLOR_PARAMS = ["color", "back", "bg"];
let types = {}, schema = null, selected = null;
let dragging = false;   // a pointer or HTML drag is in progress: the background refresh must not redraw
let epoch = 0, lastSchema = "", pointerDown = false;
document.addEventListener("pointerdown", () => { pointerDown = true; }, true);
document.addEventListener("pointerup", () => { pointerDown = false; }, true);
document.addEventListener("pointercancel", () => { pointerDown = false; }, true);

async function api(name, args = {}) {
  const r = await fetch("/api/" + name, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(args),
  });
  const j = await r.json();
  if (!j.ok) throw new Error(j.error);
  return j.result;
}
function say(text, err) {
  const s = $("status");
  s.textContent = "";
  const span = document.createElement("span");
  if (err) span.className = "err";
  span.textContent = text;
  s.appendChild(span);
}
async function act(fn) {
  try { await fn(); } catch (e) { say(e.message, true); }
  await reload();
}
async function reload() {
  await freshSchema();
  render();
}
// the schema as the server has it now (ids and orders are chosen from it, not from a stale copy)
async function freshSchema() {
  epoch++;
  schema = await api("get_schema");
  lastSchema = JSON.stringify(schema);
}

const blocksOf = () => (schema && schema.logic.block) || [];
const val = (b, p) => (b[p.name] !== undefined && b[p.name] !== null ? b[p.name] : p.default);
const num = (v, d) => (typeof v === "number" ? v : d);
// Param of a block; a param left at its default is absent from the block, so the type's default is used.
function bp(b, name, d) {
  if (b[name] !== undefined && b[name] !== null) return b[name];
  const p = (types[b.type]?.params || []).find((x) => x.name === name);
  return p && p.default !== undefined && p.default !== null ? p.default : d;
}
function uniqueId(base) {
  const used = new Set(blocksOf().map((b) => b.id));
  let n = 1, id = base;
  while (used.has(id)) id = base + ++n;
  return id;
}
// Loads the library block types the page needs: `filters` are name filters, `accept(t)` keeps a type.
async function loadTypes(filters, accept) {
  for (const f of filters) {
    for (const t of await api("list_block_types", { filter: f })) if (accept(t)) types[t.type] = t;
  }
}

// ---- widget palette (left edge): click adds, drag onto the canvas adds at the drop point ----
const PALETTE_MIME = "application/x-crab-widget";
const GLYPHS = {
  button: "▶", chart: "∿", clock: "◷", gauge: "◔", icon: "★", image: "▣", input: "⌨", label: "T", led: "●",
  net: "⇄", shape: "◆", slider: "⇔", switch: "⏻", tank: "▥", text: "Aa", text_input: "✎", value: "12",
  disp_text: "Aa", disp_value: "12", disp_bar: "▰", disp_led: "●", disp_icon: "★", item: "≡", submenu: "☰",
};
// items: [{ key, name }]; onAdd(key) is called on a click. Rebuilt only when the item list changes.
function buildPalette(items, onAdd, title) {
  const el = $("palette");
  el.hidden = !items.length;
  const sig = JSON.stringify([title, items]);
  if (el.dataset.sig === sig) return;
  el.dataset.sig = sig;
  el.textContent = "";
  const h = document.createElement("h2");
  h.textContent = title;
  el.appendChild(h);
  for (const it of items) {
    const btn = document.createElement("button");
    btn.className = "pw";
    btn.draggable = true;
    btn.title = `Добавить: ${it.name}. Щелчок — на свободное место, перетаскивание — в нужную точку.`;
    const g = document.createElement("i");
    g.textContent = GLYPHS[it.key.replace(/^web_/, "")] || "▫";
    btn.append(g, document.createTextNode(it.name));
    btn.onclick = () => onAdd(it.key);
    btn.ondragstart = (ev) => { ev.dataTransfer.setData(PALETTE_MIME, it.key); dragging = true; };
    btn.ondragend = () => { dragging = false; };
    el.appendChild(btn);
  }
}
// Makes `el` accept palette drops: onDrop(key, ev). Other drags (by `text/plain`) go to `other(ev)` if given.
function acceptDrops(el, onDrop, other) {
  const kinds = (ev) => [...(ev.dataTransfer?.types || [])];
  el.addEventListener("dragover", (ev) => {
    if (kinds(ev).includes(PALETTE_MIME) || (other && kinds(ev).includes("text/plain"))) {
      ev.preventDefault();
      el.classList.add("drop");
    }
  });
  el.addEventListener("dragleave", () => el.classList.remove("drop"));
  el.addEventListener("drop", (ev) => {
    el.classList.remove("drop");
    dragging = false;
    const key = ev.dataTransfer.getData(PALETTE_MIME);
    if (key) { ev.preventDefault(); onDrop(key, ev); } else if (other) { ev.preventDefault(); other(ev); }
  });
}

// ---- properties form helpers ----
function sources(inputType) {
  // all block outputs of a compatible type (the schema view lists the active ports)
  const out = [];
  for (const [id, v] of Object.entries(schema.view || {})) {
    for (const o of v.outputs || []) {
      if (o.type === inputType || o.type === "any" || inputType === "any") out.push(`${id}.${o.name}`);
    }
  }
  return out;
}

function bindingsSection(el, b) {
  // bindings: inputs take a source from the schema, outputs show where they go
  const view = (schema.view || {})[b.id] || { inputs: [], outputs: [] };
  if (view.inputs.length) {
    const h2 = document.createElement("h2");
    h2.textContent = "Источник сигнала";
    el.appendChild(h2);
    for (const inp of view.inputs) {
      const cur = ((schema.logic.link || []).find((l) => l.to === `${b.id}.${inp.name}`) || {}).from || "";
      const sel = document.createElement("select");
      sel.appendChild(new Option("— не подключено —", ""));
      for (const s of sources(inp.type)) sel.appendChild(new Option(s, s));
      if (cur && ![...sel.options].some((o) => o.value === cur)) sel.appendChild(new Option(cur, cur));
      sel.value = cur;
      sel.onchange = () => act(async () => {
        if (!sel.value) await api("disconnect", { to: `${b.id}.${inp.name}` });
        else await api("connect", { from: sel.value, to: `${b.id}.${inp.name}` });
      });
      const lab = document.createElement("label");
      lab.className = "f";
      lab.textContent = `Вход «${inp.label || inp.name}» (${inp.type})`;
      el.append(lab, sel);
    }
  }
  if (view.outputs.length) {
    const h2 = document.createElement("h2");
    h2.textContent = "Куда идёт сигнал";
    el.appendChild(h2);
    for (const o of view.outputs) {
      const to = (schema.logic.link || []).filter((l) => l.from === `${b.id}.${o.name}`).map((l) => l.to);
      const p = document.createElement("div");
      p.className = "hint";
      p.textContent = `${b.id}.${o.name} → ${to.length ? to.join(", ") : "никуда (соедините в схеме)"}`;
      el.appendChild(p);
    }
  }
}

function paramField(b, p) {
  const wrap = document.createElement("div");
  const lab = document.createElement("label");
  lab.className = "f";
  lab.textContent = p.label || p.name;
  lab.title = p.description || "";
  const v = val(b, p);
  let inp;
  if (p.kind === "bool") {
    inp = document.createElement("input");
    inp.type = "checkbox";
    inp.checked = !!v;
    inp.onchange = () => set(b, p.name, inp.checked);
  } else if (p.choices && p.choices.length) {
    inp = document.createElement("select");
    for (const c of p.choices) inp.appendChild(new Option(c.label, c.value));
    inp.value = v;
    const numeric = typeof p.choices[0].value === "number";
    inp.onchange = () => set(b, p.name, numeric ? Number(inp.value) : inp.value);
  } else if (COLOR_PARAMS.includes(p.name)) {
    // a colour is a CSS string; empty means "default", the picker is a shortcut
    inp = document.createElement("div");
    inp.className = "row";
    const txt = document.createElement("input");
    const pick = document.createElement("input");
    pick.type = "color";
    pick.style.width = "44px";
    txt.value = v || "";
    if (/^#[0-9a-f]{6}$/i.test(txt.value)) pick.value = txt.value;
    txt.onchange = () => set(b, p.name, txt.value.trim());
    pick.onchange = () => set(b, p.name, pick.value);
    inp.append(txt, pick);
  } else {
    inp = document.createElement("input");
    inp.type = p.kind === "string" ? "text" : "number";
    if (p.kind === "float") inp.step = "any";
    inp.value = v === null || v === undefined ? "" : v;
    inp.onchange = () => {
      if (p.kind === "string") return set(b, p.name, inp.value);
      const n = Number(inp.value);
      set(b, p.name, p.kind === "int" ? Math.round(n) : n);
    };
  }
  wrap.append(lab, inp);
  return wrap;
}
function set(b, name, value) {
  return act(async () => { await api("set_params", { id: b.id, params: { [name]: value } }); });
}

// The editor may change blocks too: pick up its changes. Never while the user drags or edits a field.
function startPolling() {
  setInterval(async () => {
    if (document.hidden || dragging || pointerDown || document.activeElement?.closest?.("#side")) return;
    try {
      const seen = epoch;
      const next = await api("get_schema");
      if (seen !== epoch || dragging || pointerDown) return;   // an edit happened meanwhile: this copy is stale
      const text = JSON.stringify(next);
      if (text !== lastSchema) { lastSchema = text; schema = next; render(); }
    } catch (e) { /* the server may be busy; try again */ }
  }, 2000);
  // keeps the server alive while the page is open
  setInterval(() => fetch("/ping").catch(() => {}), 10000);
}
