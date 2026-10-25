// The vendor facade (PLAN-WASM-2 W2, DEC-17): assembles the CodeMirror 6
// editor around the lang-tyu module and publishes ONE entry point on
// `window.__tyu_editor`. The served artifact is an IIFE classic script
// (SRI-tagged in index.html — the JS-integrity mechanism that works; the
// app layer consumes only this facade, R-G).
//
// host contract (§6.3):
//   onRun()            — Mod-Enter: the program model (reset + submit)
//   onSessionSubmit()  — Shift-Mod-Enter: the session model (submit only)
//   getDiagnostics()   — linter source: ABI diag payloads for the LAST
//                        submission ([] when none); W2 renders them through
//                        the ASCII-identity span path (the byte↔UTF-16
//                        bridge and its property tests land in W4, and no
//                        non-ASCII fixture ships before it).

import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, dropCursor, rectangularSelection, crosshairCursor, highlightSpecialChars } from "@codemirror/view";
import { Compartment, EditorState } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { syntaxHighlighting } from "@codemirror/language";
import { linter } from "@codemirror/lint";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { tyu, tyuHighlightStyle } from "./index.mjs";

function diagnosticsLinter(host) {
  // The linter re-runs on doc changes; externally-driven diagnostic updates
  // (a submit) bump the epoch so needsRefresh forces the re-check.
  let lintedEpoch = 0;
  let epoch = 0;
  const linter_ = linter(
    (view) => {
      lintedEpoch = epoch;
      const diags = host.getDiagnostics() ?? [];
      const docLen = view.state.doc.length;
      return diags
        .map((d) => {
          // W2 span path: Tyu sources are ASCII (the shipped corpus is), so
          // byte offsets are UTF-16 offsets — asserted, never assumed: a
          // non-ASCII buffer renders nothing rather than a wrong mark.
          let from = Math.max(0, Math.min(d.span_start | 0, docLen));
          let to = Math.max(from, Math.min(from + (d.span_len | 0), docLen));
          if (!isAscii(view.state.doc)) return null;
          if (from === to) to = Math.min(docLen, from + 1);
          return {
            from,
            to,
            message: d.message,
            severity: d.severity ?? "error",
          };
        })
        .filter(Boolean);
    },
    {
      delay: 250,
      needsRefresh: () => lintedEpoch !== epoch,
    }
  );
  linter_.refreshLint = () => {
    epoch++;
  };
  return linter_;
}

function isAscii(doc) {
  const s = doc.toString();
  for (let i = 0; i < s.length; i++) {
    if (s.charCodeAt(i) > 0x7f) return false;
  }
  return true;
}

export function mount(parent, host) {
  const runKeymap = keymap.of([
    {
      key: "Mod-Enter",
      run: () => {
        host.onRun();
        return true;
      },
    },
    {
      key: "Shift-Mod-Enter",
      run: () => {
        host.onSessionSubmit();
        return true;
      },
    },
  ]);

  const lintExt = diagnosticsLinter(host);
  // Read-only is a reconfigurable compartment (native-only examples load
  // read-only, FR-25).
  const readOnly = new Compartment();
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: "",
      extensions: [
        lineNumbers(),
        highlightSpecialChars(),
        history(),
        drawSelection(),
        dropCursor(),
        EditorState.allowMultipleSelections.of(true),
        rectangularSelection(),
        crosshairCursor(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        closeBrackets(),
        readOnly.of([]),
        tyu(),
        syntaxHighlighting(tyuHighlightStyle, { fallback: true }),
        lintExt,
        runKeymap,
        // `indentWithTab` is a key BINDING (not an extension): it lives in
        // the base keymap's `of` list.
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, indentWithTab]),
      ],
    }),
  });

  return {
    setDoc(text) {
      // A document replacement is a program swap, not an edit: one atomic
      // transaction, cursor home. (Undo reaches the previous document only
      // once — standard CM6 editor-swap behavior.)
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: text },
        selection: { anchor: 0 },
        scrollIntoView: true,
      });
    },
    currentDoc() {
      return view.state.doc.toString();
    },
    jumpTo(span) {
      const docLen = view.state.doc.length;
      const from = Math.max(0, Math.min(span.start | 0, docLen));
      const to = Math.max(from, Math.min(from + (span.len | 0), docLen));
      view.dispatch({
        selection: { anchor: from, head: to },
        scrollIntoView: true,
      });
      view.focus();
    },
    focus() {
      view.focus();
    },
    // Force the linter to re-read host.getDiagnostics() (a submit happened).
    refreshLint() {
      lintExt.refreshLint();
    },
    // Native-only examples load read-only (FR-25).
    setReadOnly(v) {
      view.dispatch({
        effects: readOnly.reconfigure(EditorState.readOnly.of(v === true)),
      });
    },
  };
}

window.__tyu_editor = { mount };
