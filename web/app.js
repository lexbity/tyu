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

const FUEL_STEP_OPS = 100_000_000n; // the resume budget for the step button

function refreshStack(repl) {
  stackEl.replaceChildren(
    ...repl.stack().map((cell) => {
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

// The last submission's diagnostics, in the editor-linter's shape (I-8:
// ABI data verbatim — code, span, message; severity from the status kind).
let lastDiagnostics = [];

function noteDiagnostics(result) {
  if (result.status === "ok" || result.status === "exited") {
    lastDiagnostics = [];
    return;
  }
  lastDiagnostics = [
    {
      span_start: result.diag_span_start,
      span_len: Math.max(0, result.diag_span_end - result.diag_span_start),
      message: `E${result.diag_code} ${result.diag_message}`,
      severity: "error",
    },
  ];
}

// Set in wire(): forces the editor linter to re-read the diagnostics.
let bumpLint = () => {};

function renderResult(repl, result, mode) {
  refreshStack(repl);
  bumpLint();
  noteDiagnostics(result);
  const d = result.diag_code;
  if (result.status === "ok") {
    setStatus("ready", `${mode} · ok`);
    showDiag("");
  } else if (result.status === "exited") {
    setStatus("ready", `${mode} · exited ${result.exit_code}`);
    showDiag("");
  } else if (result.status === "fuel") {
    setStatus("ready", `${mode} · fuel exhausted (E9150) — suspended`);
    showDiag(`E${d} @${result.diag_span_start}..${result.diag_span_end} ${result.diag_message}`);
  } else {
    setStatus("ready", `${mode} · ${result.status} (E${d})`);
    showDiag(`E${d} @${result.diag_span_start}..${result.diag_span_end} ${result.diag_message}`);
  }
  stepBtn.disabled = result.status !== "fuel";
  submitBtn.disabled = false;
  runBtn.disabled = false;
}

function wire(repl) {
  // The editor mounts FIRST (the handlers below read its buffer);
  // createEditor itself degrades to the textarea on any CM6 failure
  // (DEC-26) — it never throws.
  const editor = createEditor(editorPaneEl, editorEl, {
    onRun: () => runProgram(),
    onSessionSubmit: () => submitSession(),
    getDiagnostics: () => lastDiagnostics,
  });

  const submitSession = () => {
    // The session model (DEC-19): submit without reset — the accumulating
    // module + persistent stack.
    const result = repl.submit(editor.currentDoc());
    renderResult(repl, result, "session");
  };
  const runProgram = () => {
    // The program model (DEC-19): reset + submit the whole buffer — every
    // run a deterministic, shareable demo.
    repl.reset();
    consoleEl.textContent = "";
    const result = repl.submit(editor.currentDoc());
    renderResult(repl, result, "run");
  };

  submitBtn.disabled = false;
  runBtn.disabled = false;
  runBtn.addEventListener("click", runProgram);
  submitBtn.addEventListener("click", submitSession);
  stepBtn.addEventListener("click", () => {
    try {
      const result = repl.step(FUEL_STEP_OPS);
      renderResult(repl, result, "session");
    } catch (e) {
      showDiag(`step failed: ${e.message ?? e}`);
    }
  });

  bumpLint = () => editor.refreshLint?.();
  return editor;
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
