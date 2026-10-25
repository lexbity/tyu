// Tyu browser REPL (PLAN-WASM-1 P5, extended by PLAN-WASM-2 W2): the
// static-site runtime.
//
// Boot path (§4.3): fetch the wasm artifact → verify its SHA-256 against the
// build-pinned hash (NFR-10) → instantiate → verify + install the
// `tyu-sysroot/1` blob → construct the session. A hash mismatch or boot
// failure fails the page loudly with the artifact hash + engine version
// (§10) — it never boots "best effort".
//
// Editor (W2, DEC-17/DEC-26): the CodeMirror 6 stack arrives as the SRI-pinned
// `pkg/vendor/editor.js` classic script (window.__tyu_editor); this module
// drives it exclusively through `editor_app.mjs` (R-G) and degrades to the
// textarea — loudly — on any editor failure.
//
// Run semantics (DEC-19): Run (button / Mod-Enter) = reset + submit — the
// program model; "submit" (button / Shift-Mod-Enter) = session submit — the
// accumulating model. The status bar names the mode of the last action.
//
// No eval, no innerHTML with program content, no post-boot network (§1, NFR-10).

import { WasmRepl } from "./pkg/wasm_repl.js";
import init from "./pkg/wasm_repl.js"; // default export = the async __wbg_init
import { ARTIFACTS } from "./pkg/artifacts.js";
import { createEditor } from "./editor_app.mjs";
import { buildSpanMap } from "./spanmap.mjs";

// ---------------------------------------------------------------------------
// DOM
// ---------------------------------------------------------------------------

const $ = (id) => document.getElementById(id);
const statusEl = $("status");
const engineInfoEl = $("engine-info");
const editorPaneEl = $("editor-pane");
const editorEl = $("editor");
const submitBtn = $("submit");
const runBtn = $("run");
const stepBtn = $("step");
const fuelNoteEl = $("fuel-note");
const diagEl = $("diag");
const stackEl = $("stack");
const consoleEl = $("console");
const examplesEl = $("examples");
const framesEl = $("frames");
const transcriptEl = $("transcript");
const debugPaneEl = $("debug-pane");

function setStatus(state, text) {
  statusEl.dataset.state = state;
  statusEl.textContent = text;
}

function showDiag(text) {
  if (text === "") {
    diagEl.classList.add("hidden");
    diagEl.textContent = "";
  } else {
    diagEl.classList.remove("hidden");
    diagEl.textContent = text;
  }
}

function bootFailure(reason) {
  setStatus("error", "boot failed");
  // §10: page-fail paths render the artifact hash + engine version for bug
  // reports. The engine version lives in the pinned manifest. The UI shell
  // stays up (disabled and inert) so the status bar remains readable.
  const layout = $("layout");
  layout.hidden = true;
  const el = document.createElement("pre");
  el.id = "boot-failure";
  el.textContent =
    `REPL boot FAILED: ${reason}\n` +
    `artifact sha256 mismatch or decode failure — refusing to boot.\n` +
    `engine: v${ARTIFACTS.engine_version} · wasm sha256 ${ARTIFACTS.wasm_sha256}\n` +
    `sysroot: v${ARTIFACTS.sysroot_version} · sha256 ${ARTIFACTS.sysroot_sha256}`;
  document.body.insertBefore(el, layout);
}

// ---------------------------------------------------------------------------
// Integrity (NFR-10): SHA-256 via WebCrypto, compared against the manifest
// that ci/wasm.sh generated at build time.
// ---------------------------------------------------------------------------

async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

async function verified(kind, path, pinned) {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`${kind}: fetch ${path} → HTTP ${res.status}`);
  const bytes = new Uint8Array(await res.arrayBuffer());
  const actual = await sha256Hex(bytes);
  if (actual !== pinned) {
    throw new Error(
      `${kind}: sha256 mismatch — want ${pinned}, got ${actual} (${bytes.length} bytes)`
    );
  }
  return bytes;
}

// ---------------------------------------------------------------------------
// The §6.2 imported surface: the page's first-party device layer. Devices
// are plain functions of their own state (§Q6 determinism); only apertures
// the page binds (repl.bind_device) are reachable — the engine rejects any
// other access with E9152 inside wasm, so a JS exception can never become a
// wasm trap (§6.2 error discipline).
// ---------------------------------------------------------------------------

const devices = new Map(); // aperture (number) → Uint8Array (RAM backing)

globalThis.tyu = {
  periph_read(ap, off, width) {
    const dev = devices.get(Number(ap));
    if (!dev) {
      console.warn(`tyu: read on unbound aperture ${ap} (engine should gate this)`);
      return 0n;
    }
    let v = 0n;
    const base = Number(off);
    for (let i = 0; i < Number(width); i++) {
      v |= BigInt(dev[base + i] ?? 0) << BigInt(8 * i);
    }
    return v;
  },

  periph_write(ap, off, width, val) {
    const dev = devices.get(Number(ap));
    if (!dev) {
      console.warn(`tyu: write on unbound aperture ${ap} (engine should gate this)`);
      return;
    }
    const base = Number(off);
    for (let i = 0; i < Number(width); i++) {
      dev[base + i] = Number((val >> BigInt(8 * i)) & 0xffn);
    }
  },

  emit_output(bytes) {
    // Stream to the console pane (the session retains its own copy).
    consoleEl.textContent += new TextDecoder().decode(bytes);
    consoleEl.scrollTop = consoleEl.scrollHeight;
  },

  now_ms() {
    // UI timing only; the engine never calls this (§Q6, G55-a).
    return performance.now();
  },
};

// ---------------------------------------------------------------------------
// UI behavior
// ---------------------------------------------------------------------------

function refreshStack(repl) {
  // §Q9 render cap: the top 200 cells + a visible "+N deeper" counter —
  // the ABI payload is complete; the DOM is not.
  const MAX = RENDER_CAPS.stackCells;
  const cells = repl.stack();
  const shown = cells.slice(-MAX);
  const overflow = cells.length - shown.length;
  stackEl.replaceChildren();
  if (overflow > 0) {
    const note = document.createElement("li");
    note.className = "overflow-note";
    note.textContent = `+${overflow} deeper (showing top ${MAX})`;
    stackEl.append(note);
  }
  stackEl.append(
    ...shown.map((cell) => {
      const li = document.createElement("li");
      const ty = document.createElement("span");
      ty.className = "ty";
      ty.textContent = cell.ty_name;
      const val = document.createElement("span");
      val.textContent = cell.value_hex;
      li.append(ty, val);
      return li;
    })
  );
}

// ---------------------------------------------------------------------------
// The example corpus (W3, DEC-18): hash-verified BEFORE import (I-7). The
// module's only exports are data constants; a hash-mismatched module is
// never instantiated. Failure degrades the PANE, never the page.
// ---------------------------------------------------------------------------

let EXAMPLES = null;

async function loadExamples() {
  try {
    const res = await fetch("./pkg/examples.js");
    if (!res.ok) throw new Error(`fetch → HTTP ${res.status}`);
    const bytes = new Uint8Array(await res.arrayBuffer());
    const actual = await sha256Hex(bytes);
    if (actual !== ARTIFACTS.examples_sha256) {
      throw new Error(`sha256 mismatch — want ${ARTIFACTS.examples_sha256}, got ${actual}`);
    }
    const mod = await import("./pkg/examples.js");
    return mod.EXAMPLES;
  } catch (e) {
    console.warn(`tyu: examples unavailable (integrity): ${e.message ?? e}`);
    return null;
  }
}

function renderExamples(examples) {
  examplesEl.replaceChildren();
  if (!examples) {
    const note = document.createElement("div");
    note.className = "examples-note";
    note.textContent = "examples unavailable (integrity)";
    examplesEl.append(note);
    return;
  }
  // Chapter groups, collapsible; labels and badges via textContent (I-7).
  const byChapter = new Map();
  for (const ex of examples) {
    if (!byChapter.has(ex.chapter)) byChapter.set(ex.chapter, []);
    byChapter.get(ex.chapter).push(ex);
  }
  for (const [chapter, exs] of byChapter) {
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = `chapter ${chapter}`;
    details.append(summary);
    for (const ex of exs) {
      const btn = document.createElement("button");
      btn.className = "example";
      btn.dataset.exampleId = ex.id;
      const title = document.createElement("span");
      title.textContent = ex.title;
      const badge = document.createElement("span");
      badge.className = `badge ${ex.kind}`;
      if (ex.kind === "runnable") {
        badge.textContent =
          ex.expect?.outcome === "diag" ? "red" : "green";
        badge.classList.add("ok");
      } else {
        badge.textContent = "native-only";
        badge.classList.add("native-only");
        btn.title = ex.reason ?? "";
      }
      btn.append(title, badge);
      btn.addEventListener("click", () => loadExample(ex));
      details.append(btn);
    }
    examplesEl.append(details);
  }
}

// FR-25: loading sets the buffer and resets the session; unsaved edits are
// confirmed away; native-only examples load read-only with their reason.
let loadedSource = null;

function bufferIsDirty() {
  const doc = editor.currentDoc();
  return doc !== loadedSource && doc !== "";
}

async function loadExample(ex) {
  if (bufferIsDirty() && !window.confirm("discard the current buffer?")) {
    return;
  }
  replInstance.reset();
  consoleEl.textContent = "";
  lastDiagnostics = [];
  editor.setDoc(ex.source);
  editor.setReadOnly(ex.kind === "native-only");
  loadedSource = ex.source;
  const result = replInstance.submit(ex.source);
  renderResult(replInstance, result, `example ${ex.id.split("/")[1]}`);
  if (ex.kind === "native-only") {
    showDiag(`native-only — ${ex.reason ?? ""}`);
  } else {
    showDiag("");
  }
  examplesEl.querySelectorAll(".example.target").forEach((el) => el.classList.remove("target"));
  const target = examplesEl.querySelector(`[data-example-id="${CSS.escape(ex.id)}"]`);
  if (target) {
    target.closest("details")?.setAttribute("open", "");
    target.classList.add("target");
    target.scrollIntoView({ block: "nearest" });
  }
}

// FR-37: `#example=<id>` deep links — at boot and on hashchange; an unknown
// id renders an inline notice without failing the page.
function applyDeepLink() {
  const m = window.location.hash.match(/^#example=(.+)$/);
  if (!m) return;
  const id = decodeURIComponent(m[1]);
  const ex = EXAMPLES?.find((e) => e.id === id);
  const existing = examplesEl.querySelector(".examples-note.deep-link");
  if (existing) existing.remove();
  if (ex) {
    loadExample(ex);
  } else {
    const note = document.createElement("div");
    note.className = "examples-note deep-link";
    note.textContent = "unknown example";
    examplesEl.append(note);
  }
}

// The last submission's diagnostics, POSITIONED through the span bridge
// (DEC-21/I-9: the ABI hands doc-relative BYTE spans; this is the one
// translator). Render caps per §Q9: the data is complete, the DOM is not.
let lastDiagnostics = [];
let spanMap = buildSpanMap("");

function noteDiagnostics(result) {
  if (result.status === "ok" || result.status === "exited") {
    lastDiagnostics = [];
    return;
  }
  const { from, to } = spanMap.toUtf16(
    result.diag_span_start,
    Math.max(0, result.diag_span_end - result.diag_span_start)
  );
  lastDiagnostics = [
    {
      from,
      to,
      message: `E${result.diag_code} ${result.diag_message}`,
      severity: "error",
    },
  ];
}

// Set in wire(): forces the editor linter to re-read the diagnostics.
let bumpLint = () => {};

function renderResult(repl, result, mode) {
  refreshStack(repl);
  noteDiagnostics(result);
  bumpLint();
  renderDebugPanes(repl, result);
  pushTranscript(mode, result);
  // §7.4: one line, every field ABI data — mode · outcome · stack · words ·
  // fuel. No JS-side inference.
  const summary = repl.last_status();
  const fuelM = Number(repl.fuel_default());
  const fuelLabel =
    fuelM >= 1_000_000 ? `${Math.round(fuelM / 1_000_000)}M` : `${fuelM}`;
  const parts = [
    mode,
    summary?.status ?? result.status,
    `stack ${repl.stack().length}`,
    `words ${repl.words().length}`,
    `fuel ${fuelLabel}`,
  ];
  setStatus("ready", parts.join(" · "));

  const d = result.diag_code;
  if (result.status === "ok" || result.status === "exited") {
    showDiag("");
  } else if (result.status === "fuel") {
    showDiag(`E${d} @${result.diag_span_start}..${result.diag_span_end} ${result.diag_message}`);
  } else {
    const detail = result.detail;
    let detailText = "";
    if (detail) {
      const actual = detail.actual
        .map((c) => `${c.ty_name}:${c.value_hex}`)
        .join(" ");
      detailText = `\n  ${detail.kind}: ${detail.word} declared ${detail.declared} — actual: ${actual}`;
    }
    showDiag(`E${d} @${result.diag_span_start}..${result.diag_span_end} ${result.diag_message}${detailText}`);
  }
  stepBtn.disabled = result.status !== "fuel";
  submitBtn.disabled = false;
  runBtn.disabled = false;
}

// ---------------------------------------------------------------------------
// The debug console (W4, §Q4–§Q11): every pixel is ABI data (I-8). The UI
// selects the frame source by the last status — LIVE frames while a phrase
// is suspended, the POST-MORTEM backtrace from trace_events when it trapped
// (§Q8/§7.2) — and renders caps per §Q9 (truncation visible, never silent).
// ---------------------------------------------------------------------------

const RENDER_CAPS = {
  stackCells: 200, // top N + a "+N deeper" line
  traceEvents: 512, // first N + a counter
};

function renderDebugPanes(repl, result) {
  renderFrames(repl, result);
}

// §Q8/§7.2: live frames vs post-mortem backtrace, joined on nothing —
// the UI picks ONE source by the last status.
function renderFrames(repl, result) {
  framesEl.replaceChildren();
  // `frames_snapshot` is the LIVE-frames source: non-empty only while a
  // phrase is suspended (§Q8 — after a completed or rolled-back submission
  // the engine has no frames).
  const live = repl.frames_snapshot();
  if (live.length > 0) {
    const head = document.createElement("h3");
    head.textContent = "call stack (live — phrase suspended)";
    framesEl.append(head);
    const ol = document.createElement("ol");
    for (const f of live) {
      ol.append(frameRow(f, false));
    }
    framesEl.append(ol);
    return;
  }
  if (result.status === "trapped") {
    const head = document.createElement("h3");
    head.textContent = "backtrace (post-mortem — state rolled back)";
    framesEl.append(head);
    const events = repl.trace_events();
    const calls = events.filter((e) => e.kind === "call");
    const shown = calls.slice(-RENDER_CAPS.traceEvents);
    const ol = document.createElement("ol");
    // Innermost first: reverse emission order (gdb `bt` order). As-built,
    // Call events carry word+depth only — the span and the click-to-source
    // live on the trap row.
    for (const c of [...shown].reverse()) {
      const li = document.createElement("li");
      li.className = "bt-row";
      li.textContent = `at ${c.word} (depth ${c.depth})`;
      ol.append(li);
    }
    framesEl.append(ol);
    if (calls.length > shown.length) {
      const note = document.createElement("div");
      note.className = "overflow-note";
      note.textContent = `+${calls.length - shown.length} earlier calls hidden`;
      framesEl.append(note);
    }
    // The trap row: frozen code + the §Q6 sig-vs-actual (FR-29).
    const trap = events.find((e) => e.kind === "trap");
    if (trap) {
      const row = document.createElement("div");
      row.className = "trap-row";
      const line = document.createElement("div");
      line.textContent = `trap ${trap.code}`;
      row.append(line);
      const pos = spanMap.toUtf16(trap.span_start, trap.span_len);
      const jump = document.createElement("button");
      jump.className = "bt-row";
      jump.textContent = `@${trap.span_start}..${trap.span_start + trap.span_len} — show source`;
      jump.addEventListener("click", () => editor.jumpTo(pos.from, pos.to));
      row.append(jump);
      if (trap.detail) {
        const detail = document.createElement("div");
        const actual = trap.detail.actual.map((c) => `${c.ty_name}:${c.value_hex}`).join(" ");
        detail.textContent = `${trap.detail.kind}: ${trap.detail.word} declared ${trap.detail.declared} — actual: ${actual}`;
        row.append(detail);
      }
      framesEl.append(row);
    }
    return;
  }
  // Neither suspended nor trapped: an empty pane with the honest reason.
  const empty = document.createElement("div");
  empty.className = "overflow-note";
  empty.textContent = "no live frames (the phrase completed or rolled back)";
  framesEl.append(empty);
}

function frameRow(f, clickable) {
  const li = document.createElement("li");
  const line = document.createElement("span");
  line.textContent = `#${f.index} ${f.word} ${f.sig}`;
  li.append(line);
  if (clickable && f.call_span_end > f.call_span_start) {
    const btn = document.createElement("button");
    btn.className = "bt-row";
    btn.textContent = "show call site";
    const pos = spanMap.toUtf16(f.call_span_start, f.call_span_end - f.call_span_start);
    btn.addEventListener("click", () => editor.jumpTo(pos.from, pos.to));
    li.append(btn);
  }
  return li;
}

// ---------------------------------------------------------------------------
// The transcript (FR-31, §Q11): per-submission records — source, mode,
// status, diagnostics, output range, stack depth — FIFO at 100 (§Q9),
// exportable from ?debug=1.
// ---------------------------------------------------------------------------

const TRANSCRIPT_CAP = 100;
const transcript = [];

function pushTranscript(mode, result) {
  transcript.push({
    n: transcript.length + 1,
    mode,
    source: editor.currentDoc(),
    status: result.status,
    diag_code: result.diag_code,
    diag_message: result.diag_message,
    output: replInstance
      .output_total()
      .slice(result.output_start, result.output_end)
      .toString(),
    stack_depth: replInstance.stack().length,
  });
  while (transcript.length > TRANSCRIPT_CAP) transcript.shift();
  renderTranscript();
}

function renderTranscript() {
  transcriptEl.replaceChildren();
  for (const t of transcript.slice(-30)) {
    const div = document.createElement("div");
    div.className = "tr-row";
    const head = document.createElement("div");
    head.className = "tr-head";
    head.textContent = `[${t.n}] ${t.mode} → ${t.status} (stack ${t.stack_depth})`;
    const src = document.createElement("div");
    src.className = "tr-src";
    src.textContent = t.source.length > 80 ? `${t.source.slice(0, 80)}…` : t.source;
    div.append(head, src);
    transcriptEl.append(div);
  }
  if (transcript.length > 30) {
    const note = document.createElement("div");
    note.className = "overflow-note";
    note.textContent = `+${transcript.length - 30} older entries (export from ?debug=1)`;
    transcriptEl.append(note);
  }
}

function wire(repl) {
  // The editor mounts FIRST (the handlers below read its buffer);
  // createEditor itself degrades to the textarea on any CM6 failure
  // (DEC-26) — it never throws.
  const localEditor = createEditor(editorPaneEl, editorEl, {
    onRun: () => runProgram(),
    onSessionSubmit: () => submitSession(),
    getDiagnostics: () => lastDiagnostics,
    // FR-30: signature-labeled completions from the session dictionary
    // (empty before the first successful compile).
    getCompletions: () =>
      repl.words().map((w) => ({ label: w.name, detail: w.sig })),
  });

  // D-W9: the fuel budget is ABI data — the resume budget is the default's
  // §6.5 step fraction, and the note renders the ABI number.
  // wasm-bindgen surfaces u64 as BigInt; the budget fits an f64 and every
  // arithmetic below is Number-space (coerced once, at the boundary).
  const fuelDefault = Number(repl.fuel_default());
  const stepBudget = BigInt(Math.max(1_000, Math.floor(fuelDefault / 100)));
  fuelNoteEl.textContent = `fuel: ${fuelDefault >= 1_000_000 ? `${Math.round(fuelDefault / 1_000_000)}M` : fuelDefault} ops/phrase · step ${stepBudget}`;

  // DEC-21: the span bridge is built ONCE per buffer, at submit time —
  // every span translation below goes through it. Submit controls are
  // disabled for the (synchronous) run window (FR-26).
  const beginSubmit = () => {
    spanMap = buildSpanMap(localEditor.currentDoc());
    submitBtn.disabled = true;
    runBtn.disabled = true;
  };

  // FR-30: `?word` introspection — never a submission. Session words jump
  // to their definition; sysroot words show the blob source.
  const introspect = () => {
    const name = localEditor.currentDoc().trim().slice(1);
    const info = repl.words().find((w) => w.name === name);
    if (!info) {
      showDiag(`?${name}: not in the dictionary`);
      return;
    }
    const src = repl.word_source(name);
    let text = `${info.name} ${info.sig}  effects: ${info.effects}  (${info.origin})`;
    if (src.origin === "sysroot") {
      text += `\nsource: ${src.path}\n${src.source}`;
    } else if (src.origin === "session") {
      const pos = spanMap.toUtf16(src.def_span_start, src.def_span_len);
      text += "\n(definition selected in the editor)";
      localEditor.jumpTo(pos.from, pos.to);
    }
    showDiag(text);
  };

  const submitSession = () => {
    if (localEditor.currentDoc().trimStart().startsWith("?")) {
      introspect();
      submitBtn.disabled = false;
      runBtn.disabled = false;
      return;
    }
    // The session model (DEC-19): submit without reset — the accumulating
    // module + persistent stack.
    beginSubmit();
    const result = repl.submit(localEditor.currentDoc());
    renderResult(repl, result, "session");
  };
  const runProgram = () => {
    // The program model (DEC-19): reset + submit the whole buffer — every
    // run a deterministic, shareable demo.
    beginSubmit();
    repl.reset();
    consoleEl.textContent = "";
    const result = repl.submit(localEditor.currentDoc());
    renderResult(repl, result, "run");
  };

  submitBtn.disabled = false;
  runBtn.disabled = false;
  runBtn.addEventListener("click", runProgram);
  submitBtn.addEventListener("click", submitSession);
  stepBtn.addEventListener("click", () => {
    try {
      const result = repl.step(stepBudget);
      renderResult(repl, result, "session");
    } catch (e) {
      showDiag(`step failed: ${e.message ?? e}`);
    }
  });

  bumpLint = () => localEditor.refreshLint?.();
  return localEditor;
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

let editor = null;
let replInstance = null;

async function boot() {
  try {
    setStatus("booting", "verifying artifact…");
    const wasmBytes = await verified("engine", "./pkg/wasm_repl_bg.wasm", ARTIFACTS.wasm_sha256);
    const blobBytes = await verified("sysroot", "./pkg/tyu-sysroot-1", ARTIFACTS.sysroot_sha256);

    // ASYNC instantiation (the CSP's `wasm-unsafe-eval` covers WebAssembly
    // compilation; sync `new WebAssembly.Module` requires the broader
    // `unsafe-eval`, which this page refuses — no eval, NFR-10). The bytes
    // are the integrity-verified ones from above.
    await init({ module_or_path: wasmBytes });
    const repl = new WasmRepl(blobBytes);

    // A demo board device: one RAM-backed aperture (the browser analogue of
    // the scripted devices; tutorials wire richer devices in W3's labs).
    devices.set(0, new Uint8Array(4096));
    repl.bind_device(0);

    engineInfoEl.textContent = `engine v${repl.engine_version()} · wasm ${ARTIFACTS.wasm_sha256.slice(0, 12)}…`;
    fuelNoteEl.textContent = "fuel: 100M ops/phrase";
    setStatus("ready", "ready");

    // wire mounts the editor (CM6, or the textarea fallback with a degraded
    // reason — DEC-26).
    editor = wire(repl);
    replInstance = repl;
    if (editor.degraded) {
      showDiag(`editor degraded — basic mode: ${editor.degradedReason}`);
    }

    // The example corpus (hash-gated) and its deep links (FR-37).
    EXAMPLES = await loadExamples();
    renderExamples(EXAMPLES);
    window.addEventListener("hashchange", applyDeepLink);
    applyDeepLink();
  } catch (e) {
    bootFailure(e.message ?? String(e));
  }
}

boot();
