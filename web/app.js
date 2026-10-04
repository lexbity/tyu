// Tyu browser REPL (PLAN-WASM-1 P5): the static-site runtime.
//
// Boot path (§4.3): fetch the wasm artifact → verify its SHA-256 against the
// build-pinned hash (NFR-10) → instantiate → verify + install the
// `tyu-sysroot/1` blob → construct the session. A hash mismatch or boot
// failure fails the page loudly with the artifact hash + engine version
// (§10) — it never boots "best effort".
//
// No eval, no innerHTML with program content, no post-boot network (§1, NFR-10).

import { WasmRepl } from "./pkg/wasm_repl.js";
import init from "./pkg/wasm_repl.js"; // default export = the async __wbg_init
import { ARTIFACTS } from "./pkg/artifacts.js";

// ---------------------------------------------------------------------------
// DOM
// ---------------------------------------------------------------------------

const $ = (id) => document.getElementById(id);
const statusEl = $("status");
const engineInfoEl = $("engine-info");
const editorEl = $("editor");
const submitBtn = $("submit");
const stepBtn = $("step");
const fuelNoteEl = $("fuel-note");
const diagEl = $("diag");
const stackEl = $("stack");
const consoleEl = $("console");

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
  periph_read(ap, off, width, ) {
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

function renderResult(repl, result) {
  refreshStack(repl);
  const d = result.diag_code;
  if (result.status === "ok") {
    setStatus("ready", "ok");
    showDiag("");
  } else if (result.status === "exited") {
    setStatus("ready", `exited ${result.exit_code}`);
    showDiag("");
  } else if (result.status === "fuel") {
    setStatus("ready", "fuel exhausted (E9150) — suspended");
    showDiag(`E${d} @${result.diag_span_start}..${result.diag_span_end} ${result.diag_message}`);
  } else {
    setStatus("ready", `${result.status} (E${d})`);
    showDiag(`E${d} @${result.diag_span_start}..${result.diag_span_end} ${result.diag_message}`);
  }
  stepBtn.disabled = result.status !== "fuel";
  submitBtn.disabled = false;
}

function wire(repl) {
  submitBtn.disabled = false;
  submitBtn.addEventListener("click", () => {
    const result = repl.submit(editorEl.value);
    renderResult(repl, result);
  });
  editorEl.addEventListener("keydown", (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key === "Enter") {
      ev.preventDefault();
      submitBtn.click();
    }
  });
  stepBtn.addEventListener("click", () => {
    try {
      const result = repl.step(FUEL_STEP_OPS);
      renderResult(repl, result);
    } catch (e) {
      showDiag(`step failed: ${e.message ?? e}`);
    }
  });
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

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
    // the scripted devices; tutorials wire richer devices in P6).
    devices.set(0, new Uint8Array(4096));
    repl.bind_device(0);

    engineInfoEl.textContent = `engine v${repl.engine_version()} · wasm ${ARTIFACTS.wasm_sha256.slice(0, 12)}…`;
    fuelNoteEl.textContent = "fuel: 100M ops/phrase";
    setStatus("ready", "ready");
    wire(repl);
  } catch (e) {
    bootFailure(e.message ?? String(e));
  }
}

boot();
