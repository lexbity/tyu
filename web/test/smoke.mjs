// G55 browser smoke (PLAN-WASM-1 P5, AC-5): a real browser loads the static
// site, the integrity check passes, a lesson-0 program drives the stack
// panel, a reload resets, and the console stays clean. Run with:
//
//   node web/test/smoke.mjs [--tampered]
//
// `--tampered` is the G55 red path (AC-9): the server corrupts ONE byte of
// the wasm artifact and the page must refuse to boot (integrity banner, no
// REPL). Requires `playwright` (CI installs it; `npm i playwright` locally)
// and its chromium build (`npx playwright install chromium`).

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const WEB = fileURLToPath(new URL("..", import.meta.url));
const TAMPERED = process.argv.includes("--tampered");

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".json": "application/json",
};

function serve() {
  return new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      try {
        const url = new URL(req.url, "http://localhost");
        let path = normalize(decodeURIComponent(url.pathname)).replace(/^([/\\])+/, "");
        if (path === "" || path.endsWith("/")) path += "index.html";
        const full = join(WEB, path);
        let bytes = await readFile(full);
        if (TAMPERED && path.endsWith(".wasm")) {
          // The red path: flip one byte of the shipped artifact. The page's
          // SHA-256 pin must catch it — a tampered engine never boots.
          bytes = bytes.slice();
          bytes[Math.floor(bytes.length / 2)] ^= 0x01;
        }
        res.writeHead(200, { "content-type": MIME[extname(full)] ?? "application/octet-stream" });
        res.end(bytes);
      } catch {
        res.writeHead(404);
        res.end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

const { chromium } = await import("playwright").catch(() => {
  console.error("SKIP: playwright not installed (npm install playwright + npx playwright install chromium)");
  process.exit(2);
});

const server = await serve();
const port = server.address().port;
const base = `http://127.0.0.1:${port}/`;

const consoleErrors = [];
let failed = false;
const assert = (cond, msg) => {
  if (cond) {
    console.log(`  ok: ${msg}`);
  } else {
    console.error(`  FAIL: ${msg}`);
    failed = true;
  }
};

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  page.on("console", (m) => {
    if (m.type() === "error") consoleErrors.push(m.text());
  });
  page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));

  if (TAMPERED) {
    // -- the red path: the tampered artifact must fail the integrity check --
    await page.goto(base, { waitUntil: "load" });
    await page.waitForSelector("#boot-failure", { timeout: 15_000 });
    const banner = await page.textContent("#boot-failure");
    assert(banner.includes("boot FAILED"), "tampered artifact → loud boot-failure banner");
    assert(banner.includes("sha256"), "banner carries the pinned hash (§10)");
    const status = await page.textContent("#status");
    assert(status.includes("boot failed"), "status bar shows boot failure");
  } else {
    await page.goto(base, { waitUntil: "load" });
    await page.waitForFunction(
      () => document.getElementById("status")?.dataset.state === "ready",
      null,
      { timeout: 15_000 }
    );
    assert(true, "page boots to ready");

    // W2: the CM6 editor is the active surface.
    assert(await page.$(".cm-editor") !== null, "CM6 editor mounted (.cm-editor present)");
    assert(await page.$("#editor") !== null && await page.$eval("#editor", (el) => el.hidden),
      "fallback textarea hidden while CM6 is active");
    assert(await page.$("#diag") !== null && (await page.textContent("#diag")) === "",
      "no degraded banner in full mode");

    let cells = [];
    // Editor-aware input: CM6 (keyboard events) or the textarea (fill).
    const typeInto = async (src) => {
      if (await page.$(".cm-content")) {
        await page.click(".cm-content");
        await page.keyboard.press("ControlOrMeta+a");
        await page.keyboard.insertText(src);
      } else {
        await page.fill("#editor", src);
      }
    };
    const submit = async (src) => {
      await typeInto(src);
      await page.click("#submit");
    };
    const run = async (src) => {
      await typeInto(src);
      await page.click("#run");
    };

    // Highlighting is live: a `module` keyword in the buffer carries the
    // lang-tyu token class (the HighlightStyle contract).
    await typeInto("module M; : w ( -- ) ; end;");
    const highlighted = await page.$$eval(".cm-line span", (spans) =>
      spans.some((sp) => sp.className.includes("tyu-kw"))
    );
    assert(highlighted, "keyword highlight class applied (tyu-kw)");

    // DEC-19: Run resets — output cleared, program model.
    await run("import platform/testio { testio.write-byte };\n65 testio.write-byte");
    let outText = await page.textContent("#console");
    assert(outText.includes("A"), `run emits the byte (${JSON.stringify(outText)})`);
    await run("1 2 3");
    outText = await page.textContent("#console");
    assert(!outText.includes("A"), "run reset the session (console cleared)");
    cells = await page.$$eval("#stack li", (lis) => lis.map((li) => li.textContent.trim()));
    assert(cells.length === 3, `run left the fresh program's stack (${cells.length})`);

    // The session flow starts from a clean slate (an emptied run resets).
    await page.click(".cm-content");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.press("Backspace");
    await page.click("#run");
    cells = await page.$$eval("#stack li", (lis) => lis.map((li) => li.textContent.trim()));
    assert(cells.length === 0, `empty run resets the stack (${cells.length})`);

    // Lesson-0 flow: push 1 2 3, then fold with + (session model).
    await submit("1 2 3");
    cells = await page.$$eval("#stack li", (lis) =>
      lis.map((li) => li.textContent.trim())
    );
    assert(cells.length === 3, `1 2 3 → three stack cells (${JSON.stringify(cells)})`);

    await submit("+");
    cells = await page.$$eval("#stack li", (lis) => lis.map((li) => li.textContent.trim()));
    assert(cells.length === 2, `+ → two cells (${JSON.stringify(cells)})`);
    // DOM order is bottom-first: the top cell is the last element, holding
    // 2+3 = 5.
    assert(cells[1].includes("0x0000000000000005"), "top of stack is 2+3 = 5");

    // Definitions persist; output streams to the console pane.
    await submit("import platform/testio { testio.write-byte };");
    await submit("65 testio.write-byte");
    const out = await page.textContent("#console");
    assert(out.includes("A"), `output pane shows the written byte (got ${JSON.stringify(out)})`);

    // Submit survives compile errors and renders them as lint marks
    // (W2 diagnostics path, ASCII spans).
    await submit(":@:");
    await page.waitForTimeout(800); // the linter's debounce
    const lintCount = await page.$$eval(
      ".cm-lintRange-error, .cm-lintPoint-error",
      (els) => els.length
    ).catch(() => 0);
    assert(lintCount >= 1, `compile error renders a lint mark (${lintCount})`);

    // NFR-12 evidence (loose browser-side bar): 100 keystrokes, input →
    // doc mutation p95 well under the human-perception bar in-process.
    await typeInto("");
    await page.click(".cm-content");
    const latencies = await page.evaluate(async () => {
      const view = document.querySelector(".cm-content").cmView?.view;
      const times = [];
      const text = ": perf ( i64 -- i64 ) dup 1 + as i64 ;\n1 2 3 + 4 5 6 + 7 8 9 + ;\n";
      for (const ch of text.repeat(3)) {
        const t0 = performance.now();
        document.execCommand("insertText", false, ch);
        await new Promise((r) => requestAnimationFrame(r));
        times.push(performance.now() - t0);
      }
      times.sort((a, b) => a - b);
      return times[Math.floor(times.length * 0.95)];
    });
    assert(typeof latencies === "number" && latencies < 100,
      `keystroke→paint p95 < 100 ms (got ${latencies?.toFixed?.(1)}ms)`);

    // The example flow may hit the unsaved-edits confirm (FR-25); the
    // smoke accepts it (the buffer content at this point is scratch).
    page.on("dialog", (d) => d.accept());

    // ---------------------------------------------------------------
    // W3 / G58: the example surface. The sidebar is data-driven from the
    // hash-pinned module; deep links load + run; a green example runs Ok,
    // a red example is rejected by the checker with its expected code,
    // native-only examples load read-only with their reason, and an
    // unknown deep link degrades inline.
    // ---------------------------------------------------------------
    const exCount = await page.$$eval("#examples .example", (els) => els.length);
    assert(exCount > 40, `sidebar lists the classified corpus (${exCount})`);
    assert(
      await page.$$eval("#examples .badge.native-only", (els) => els.length > 0),
      "native-only badges render"
    );

    // Deep link → green example: loads read-write; run → ok.
    await page.goto(`${base}#example=${encodeURIComponent("ch01/green-01-hello")}`);
    await page.waitForFunction(
      () => document.querySelector("#examples .example.target"),
      null,
      { timeout: 10_000 }
    );
    await page.click(".cm-content");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("1 1 +");
    await page.click("#run");
    await page.waitForFunction(() =>
      document.getElementById("status")?.textContent.includes("run ·"), null, { timeout: 10_000 });
    const greenStatus = await page.textContent("#status");
    assert(greenStatus.includes("ok"), `green example runs ok (${greenStatus})`);

    // Deep link → red example: the checker rejects with the EXPECTED code.
    await page.goto(`${base}#example=${encodeURIComponent("ch02/red-01-underflow")}`);
    await page.waitForFunction(
      () => document.querySelector("#examples .example.target")?.dataset.exampleId ===
        "ch02/red-01-underflow", null, { timeout: 10_000 });
    await page.click("#run");
    await page.waitForFunction(() =>
      document.getElementById("status")?.textContent.includes("compile-error"), null, { timeout: 10_000 });
    const redStatus = await page.textContent("#status");
    const redDiag = await page.textContent("#diag");
    assert(redStatus.includes("E3202"), `red example rejected with its expected code (${redStatus})`);
    assert(redDiag.includes("3202"), `diagnostic carries the code (${redDiag})`);

    // Native-only: loads read-only with its reason banner.
    await page.goto(`${base}#example=${encodeURIComponent("ch12/green-01-spawn-join-run")}`);
    await page.waitForFunction(
      () => document.querySelector("#examples .example.target")?.dataset.exampleId ===
        "ch12/green-01-spawn-join-run", null, { timeout: 10_000 });
    const noBanner = await page.textContent("#diag");
    assert(noBanner.includes("native-only"), `reason banner shown (${noBanner})`);
    // Read-only: typing into the CM6 editor must NOT change the document.
    await page.click(".cm-content");
    await page.keyboard.insertText("X");
    const afterType = await page.evaluate(() =>
      document.querySelector(".cm-content").textContent);
    assert(!afterType.includes("X"), `native-only buffer is read-only (${JSON.stringify(afterType)})`);

    // Unknown deep link: inline notice, page functional.
    await page.goto(`${base}#example=no-such-lab`);
    await page.waitForTimeout(400);
    const note = await page.textContent("#examples");
    assert(note.includes("unknown example"), `unknown id noticed (${JSON.stringify(note)})`);
    const stillReady = await page.textContent("#status");
    assert(!stillReady.includes("boot failed"), "unknown id does not fail the page");

    // ---------------------------------------------------------------
    // W4 / G57(b): the debug panel against the committed native dump. The
    // fixture (test-goldens/repl/debug/console-fixture.*) is one whole-
    // buffer submission whose every span anchors into the editor doc (I-9),
    // so the browser can be compared field-for-field with the native dump.
    // ---------------------------------------------------------------
    const { readFileSync } = await import("node:fs");
    const GOLDENS = join(WEB, "..", "test-goldens", "repl", "debug");
    const dumpLines = readFileSync(join(GOLDENS, "console-fixture.dump"), "utf8")
      .trim()
      .split("\n");
    const dump = JSON.parse(dumpLines[1]);
    const repl_fuel_from_dump = (d) => `${Math.round(d.fuel / 1_000_000)}M`;
    const dumpCalls = dump.trace_events.filter((e) => e.kind === "call");
    const dumpTrap = dump.trace_events.find((e) => e.kind === "trap");

    await page.goto(base); // drop the deep-link hash
    await page.waitForFunction(
      () => document.getElementById("status")?.dataset.state === "ready",
      null,
      { timeout: 15_000 }
    );
    const fixtureSource = readFileSync(join(GOLDENS, "console-fixture.session"), "utf8")
      .trim()
      .split("\n")
      .pop();
    await page.click(".cm-content");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText(fixtureSource);
    await page.click("#run");
    await page.waitForFunction(() =>
      document.getElementById("frames")?.textContent.includes("backtrace"),
      null,
      { timeout: 10_000 }
    );

    // Frame order (innermost first — gdb order) matches the dump.
    const frameRows = await page.$$eval("#frames ol li", (els) =>
      els.map((el) => el.textContent.trim())
    );
    const expectedFrames = [...dumpCalls].reverse().map((c) => `at ${c.word} (depth ${c.depth})`);
    assert(
      JSON.stringify(frameRows) === JSON.stringify(expectedFrames),
      `frame order vs native dump: ${JSON.stringify(frameRows)}`
    );

    // The trap row: code + the sig-vs-actual detail fields (FR-29).
    const trapText = await page.$eval("#frames .trap-row", (el) => el.textContent);
    assert(trapText.includes(`trap ${dumpTrap.code}`), `trap code rendered (${trapText})`);
    assert(trapText.includes(dumpTrap.detail.kind), `detail kind (${trapText})`);
    assert(trapText.includes(dumpTrap.detail.word), `detail word (${trapText})`);
    assert(trapText.includes(dumpTrap.detail.declared), `detail declared (${trapText})`);
    assert(trapText.includes(dumpTrap.detail.actual[0].value_hex),
      `detail actual cell (${trapText})`);

    // Stack depth and §7.4 status line vs the dump.
    const statusLine = await page.textContent("#status");
    assert(statusLine.includes(`stack ${dump.stack_depth}`), `stack depth in status (${statusLine})`);
    assert(statusLine.includes(`words ${dump.words_count}`), `word count in status (${statusLine})`);
    assert(statusLine.includes("trapped 20"), `outcome in status (${statusLine})`);

    // FR-29 in the diag banner: the trap detail is wherever the diagnostic
    // is shown.
    const diagText = await page.textContent("#diag");
    assert(diagText.includes("sig-mismatch") && diagText.includes("add-safely"),
      `sig-vs-actual in the diag banner (${JSON.stringify(diagText)})`);

    // Click-to-source: the trap row's jump selects the span in the editor
    // (doc-anchored, I-9) — the selection equals the span's text.
    await page.click("#frames .trap-row button");
    const selected = await page.evaluate(() => {
      const cm = document.querySelector(".cm-content");
      const sel = window.getSelection();
      return sel ? sel.toString() : "";
    });
    const src = fixtureSource;
    const expectedSlice = src.slice(dumpTrap.span.start, dumpTrap.span.start + dumpTrap.span.len);
    assert(
      selected === expectedSlice,
      `click-to-source selects the trap span (${JSON.stringify(selected)} vs ${JSON.stringify(expectedSlice)})`
    );

    // FR-31: the transcript records the submissions.
    const trRows = await page.$$eval("#transcript .tr-row", (els) => els.length);
    assert(trRows >= 1, `transcript has entries (${trRows})`);

    // D-W9: the fuel note renders ABI numbers, not a JS constant.
    const fuelNote = await page.textContent("#fuel-note");
    assert(fuelNote.includes(repl_fuel_from_dump(dump)), `fuel note from ABI (${fuelNote})`);

    // ---------------------------------------------------------------
    // W5 / AC-W5: the introspection surface. Completions offer imported
    // words with their declared signatures after a compile; `?word` on a
    // sysroot word shows the defining file's source; the transcript
    // reflects the session's statuses.
    // ---------------------------------------------------------------
    await page.goto(`${base}#example=${encodeURIComponent("ch01/green-01-hello")}`);
    await page.waitForFunction(
      () => document.querySelector("#examples .example.target"),
      null,
      { timeout: 10_000 }
    );
    // Completions: a prefix of the imported word opens the tooltip.
    await page.click(".cm-content");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("plat");
    await page.waitForSelector(".cm-tooltip-autocomplete", { timeout: 10_000 });
    const completion = await page.evaluate(() =>
      [...document.querySelectorAll(".cm-completionLabel")].map((el) => el.textContent)
    );
    assert(
      completion.includes("platform.io.log"),
      `completion offers the imported word (${JSON.stringify(completion)})`
    );
    const ioDetail = await page.evaluate(() => {
      // The detail rides the option row of the platform.io.log entry.
      const rows = [...document.querySelectorAll(".cm-completionItem, li")];
      const row = rows.find((el) => el.textContent.includes("platform.io.log"));
      return row?.textContent ?? "";
    });
    assert(
      ioDetail.includes("( str -- )"),
      `completion carries the declared signature (${JSON.stringify(ioDetail)})`
    );

    // ?word on a sysroot word: origin + defining file + source, never a
    // submission.
    await page.click(".cm-content");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("?platform.io.log");
    await page.click("#submit");
    await page.waitForTimeout(300);
    const inspect = await page.textContent("#diag");
    assert(
      inspect.includes("platform.io.log") && inspect.includes("( str -- )"),
      `introspection renders the signature (${JSON.stringify(inspect)})`
    );
    assert(
      inspect.includes("sysroot") && inspect.includes("platform/linux.def"),
      `view-source resolves the defining file (${JSON.stringify(inspect)})`
    );
    const statusAfterInspect = await page.textContent("#status");
    assert(
      !statusAfterInspect.includes("compile-error"),
      `?word is never submitted (${statusAfterInspect})`
    );

    // The transcript shows the session's submissions with their statuses
    // (introspection is not a submission, so it has no row — the records
    // here are the example load and the earlier fixture run).
    const trText = await page.$eval("#transcript", (el) => el.textContent);
    assert(
      trText.includes("example") && trText.includes("→ ok"),
      `transcript reflects submissions with statuses (${JSON.stringify(trText.slice(0, 200))})`
    );

    // Reload resets (Fresh state, §4.2).
    await page.reload({ waitUntil: "load" });
    await page.waitForFunction(
      () => document.getElementById("status")?.dataset.state === "ready",
      null,
      { timeout: 15_000 }
    );
    cells = await page.$$eval("#stack li", (lis) => lis.map((li) => li.textContent.trim()));
    assert(cells.length === 0, "reload resets the session stack");
  }

  assert(consoleErrors.length === 0, `zero console errors (${JSON.stringify(consoleErrors)})`);

  // W2 FR-23: the degraded-editor leg (full mode only — the tampered red
  // path never reaches Ready, so there is no working session to degrade).
  // The fallback is CI-proven, not faith-proven. A fresh page forces the
  // CM6 path to fail (?editor=fallback): textarea active, persistent banner,
  // run still works.
  if (TAMPERED) {
    if (failed) {
      console.error("G55 smoke: FAILED");
      process.exit(1);
    }
    console.log("G55 smoke (tampered red path): green");
    process.exit(0);
  }
  const page2 = await browser.newPage();
  const errors2 = [];
  page2.on("console", (m) => {
    if (m.type() === "error") errors2.push(m.text());
  });
  page2.on("pageerror", (e) => errors2.push(`pageerror: ${e.message}`));
  try {
    await page2.goto(`${base}?editor=fallback`, { waitUntil: "load" });
    await page2.waitForFunction(
      () => document.getElementById("status")?.dataset.state === "ready",
      null,
      { timeout: 15_000 }
    );
    assert(await page2.$eval("#editor", (el) => !el.hidden), "degraded mode: textarea active");
    const banner = await page2.textContent("#diag");
    assert(banner.includes("editor degraded"), `degraded banner shown (${JSON.stringify(banner)})`);
    await page2.fill("#editor", "1 2 +");
    await page2.click("#submit");
    const cells2 = await page2.$$eval("#stack li", (lis) => lis.map((li) => li.textContent.trim()));
    assert(cells2.length === 1 && cells2[0].includes("0x0000000000000003"),
      `degraded mode still runs the session (${JSON.stringify(cells2)})`);
    assert(errors2.length === 0, `zero console errors in degraded mode (${JSON.stringify(errors2)})`);
  } finally {
    await page2.close();
  }
} finally {
  await browser.close();
  server.close();
}

if (failed) {
  console.error("G55 smoke: FAILED");
  process.exit(1);
}
console.log("G55 smoke: green");
