// The editor artifact build (PLAN-WASM-2 W2, DEC-17): generated parser
// (lezer) + bundled vendor facade (esbuild IIFE) → web/pkg/vendor/. The
// served site references ONLY the generated artifact (R-F); nothing here
// runs at page time.
//
// Determinism (NFR-14): fixed input order, no timestamps — two builds are
// byte-identical (the G55 reproducibility leg proves it). The output
// manifest carries sizes + the SRI sha384 that build-web pins into
// index.html.

import { build } from "esbuild";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const GEN_DIR = join(HERE, "lang-tyu", "src");
const OUT_DIR = join(HERE, "..", "pkg", "vendor");

await main();

async function main() {
  mkdirSync(GEN_DIR, { recursive: true });
  mkdirSync(OUT_DIR, { recursive: true });

  // 1. Generate the parser (the CLI path is the one that resolves the
  //    external stack-comment tokenizer and emits its terms file).
  const gen = join(HERE, "node_modules", ".bin", "lezer-generator");
  execFileSync(
    process.execPath,
    [gen, join(HERE, "lang-tyu", "tyu.grammar"), "-o", join(GEN_DIR, "parser.js")],
    { stdio: ["ignore", "inherit", "inherit"] }
  );

  // 2. Bundle the facade: one IIFE classic script (SRI-taggable) exposing
  //    window.__tyu_editor, plus the stylesheet.
  const result = await build({
    entryPoints: [join(GEN_DIR, "vendor.mjs")],
    bundle: true,
    minify: true,
    format: "iife",
    target: "es2022",
    platform: "browser",
    outfile: join(OUT_DIR, "editor.js"),
    charset: "utf8",
    logLevel: "silent",
  });
  if (result.errors.length) {
    console.error(result.errors);
    process.exit(1);
  }
  copyFileSync(join(HERE, "editor.css"), join(OUT_DIR, "editor.css"));

  // 3. The manifest fragment: sizes + the SRI sha384 that build-web pins to
  //    strength for JS). Consumed by ci/wasm.sh build-web; never hand-edited.
  const js = readFileSync(join(OUT_DIR, "editor.js"));
  const css = readFileSync(join(OUT_DIR, "editor.css"));
  const sri = "sha384-" + createHash("sha384").update(js).digest("base64");
  const manifest = {
    editor_js_bytes: js.length,
    editor_css_bytes: css.length,
    editor_sri384: sri,
  };
  writeFileSync(
    join(OUT_DIR, "manifest.json"),
    JSON.stringify(manifest, null, 2) + "\n"
  );
  console.log(`editor bundle: ${js.length}B js + ${css.length}B css, sri ${sri}`);
}
