// The app-side editor boundary (PLAN-WASM-2 W2, DEC-26, R-G): the ONLY
// module that knows both the vendor facade and the app's session state.
// app.js never touches CM6 symbols — it consumes this module's `createEditor`
// and, on ANY failure (facade missing because the SRI check rejected the
// script, a build bug, a browser quirk), gets the proven textarea editor
// back with a degraded banner. A blank page is the one forbidden outcome.

// The textarea fallback editor (the PLAN-WASM-1 editor, preserved as the
// degraded path): same facade as the CM6 editor — { setDoc, currentDoc,
// jumpTo, focus }.
function fallbackEditor(textareaEl, host) {
  textareaEl.addEventListener("keydown", (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key === "Enter") {
      ev.preventDefault();
      if (ev.shiftKey) host.onSessionSubmit();
      else host.onRun();
    }
  });
  return {
    setDoc(text) {
      textareaEl.value = text;
    },
    currentDoc() {
      return textareaEl.value;
    },
    // Degraded mode keeps spans honest by selection: the textarea API is
    // byte-faithful for ASCII (the corpus), which is the only content the
    // W2 diagnostics path marks anyway (the general bridge lands in W4).
    // UTF-16 positions (the app translates engine byte spans; DEC-21).
    jumpTo(fromPos, toPos) {
      textareaEl.focus();
      const len = textareaEl.value.length;
      const start = Math.max(0, Math.min(fromPos | 0, len));
      const end = Math.max(start, Math.min(toPos | 0, len));
      textareaEl.setSelectionRange(start, end);
    },
    focus() {
      textareaEl.focus();
    },
    setReadOnly(v) {
      textareaEl.readOnly = v === true;
    },
    refreshLint() {}, // the textarea path renders diagnostics in #diag only
    degraded: true,
  };
}

// W2 diagnostics span path: the corpus is ASCII, so byte offsets are
// UTF-16 offsets — asserted by the caller's data only ever being produced
// from an ASCII buffer check (see diagnosticsFor).
function cm6Editor(parentEl, textareaEl, host) {
  if (new URLSearchParams(window.location.search).get("editor") === "fallback") {
    // The CI leg: force the degraded path so the fallback is proven by a
    // real browser run, never by faith (FR-23).
    throw new Error("editor degraded by request (?editor=fallback)");
  }
  const facade = window.__tyu_editor;
  if (!facade || typeof facade.mount !== "function") {
    throw new Error("editor facade missing (vendor bundle failed SRI or did not load)");
  }
  const api = facade.mount(parentEl, host);
  textareaEl.hidden = true;
  return {
    setDoc: (t) => api.setDoc(t),
    currentDoc: () => api.currentDoc(),
    jumpTo: (span) => api.jumpTo(span),
    focus: () => api.focus(),
    refreshLint: () => api.refreshLint(),
    setReadOnly: (v) => api.setReadOnly(v),
    jumpTo: (fromPos, toPos) => api.jumpTo(fromPos, toPos),
    degraded: false,
  };
}

/**
 * Create the page editor inside `parentEl` (the CM6 mount) with `textareaEl`
 * as the degraded fallback. `host` supplies { onRun, onSessionSubmit,
 * getDiagnostics } (§6.3). Returns the editor facade; throws only if BOTH
 * paths are impossible (the caller turns that into a boot failure).
 */
export function createEditor(parentEl, textareaEl, host) {
  try {
    return cm6Editor(parentEl, textareaEl, host);
  } catch (e) {
    const api = fallbackEditor(textareaEl, host);
    api.degradedReason = e.message ?? String(e);
    return api;
  }
}
