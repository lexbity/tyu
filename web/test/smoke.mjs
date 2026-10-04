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
  ".css": "text/css",
  ".wasm": "application/wasm",
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

    // Lesson-0 flow: push 1 2 3, then fold with +.
    const submit = async (src) => {
      await page.fill("#editor", src);
      await page.click("#submit");
    };
    await submit("1 2 3");
    let cells = await page.$$eval("#stack li", (lis) =>
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
} finally {
  await browser.close();
  server.close();
}

if (failed) {
  console.error("G55 smoke: FAILED");
  process.exit(1);
}
console.log("G55 smoke: green");
