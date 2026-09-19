// Screenshots every state in both appearances.
//
// Run: node tools/design/shoot.mjs [outdir]
//
// Animations are disabled so the caret is captured in the same phase every
// time: a screenshot that differs because of a blink is a diff nobody can
// read.

import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, extname, join, resolve } from "node:path";
import { createReadStream, existsSync, mkdirSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { execFileSync } from "node:child_process";

// Playwright is resolved rather than imported, because where it lives depends
// on the machine and the point of this harness is that it runs in seconds on
// any of them. A bare `import "playwright"` works only when it sits in a
// node_modules beside the repository, and ESM ignores NODE_PATH -- so on a CI
// image with a global install the fast path failed with ERR_MODULE_NOT_FOUND
// and the only way to look at a design change was the twenty-minute VM tier.
//
// PLAYWRIGHT_ROOT wins if it is set. Otherwise: a local install, then npm's
// global root, then the paths a couple of common images use.
async function loadChromium() {
  const candidates = [];
  if (process.env.PLAYWRIGHT_ROOT) candidates.push(process.env.PLAYWRIGHT_ROOT);
  try {
    const root = execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim();
    if (root) candidates.push(join(root, "playwright"));
  } catch {
    // No npm on PATH is not an error; the other candidates may still work.
  }
  candidates.push("/opt/node22/lib/node_modules/playwright", "/usr/lib/node_modules/playwright");

  // A global install is CommonJS, so the named export is not there and the
  // module arrives under `default`. Reading only `.chromium` yielded
  // `undefined` and the failure surfaced forty lines later as
  // "Cannot read properties of undefined (reading 'launch')" -- so a module
  // that resolves without a usable `chromium` is treated as a miss here
  // rather than passed on.
  const chromiumOf = (module) => module?.chromium ?? module?.default?.chromium;

  try {
    const found = chromiumOf(await import("playwright"));
    if (found) return found;
  } catch (error) {
    if (error?.code !== "ERR_MODULE_NOT_FOUND") throw error;
  }
  for (const candidate of candidates) {
    if (!existsSync(candidate)) continue;
    try {
      const found = chromiumOf(await import(pathToFileURL(join(candidate, "index.js")).href));
      if (found) return found;
    } catch (error) {
      if (error?.code !== "ERR_MODULE_NOT_FOUND") throw error;
    }
  }
  const looked = candidates.length ? `\n  looked in:\n    ${candidates.join("\n    ")}` : "";
  throw new Error(
    `playwright is not installed. Install it (npm i -D playwright) or point ` +
      `PLAYWRIGHT_ROOT at an existing install.${looked}`,
  );
}

const chromium = await loadChromium();

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(process.argv[2] ?? join(here, "shots"));
mkdirSync(out, { recursive: true });

const states = JSON.parse(readFileSync(join(here, "states.json"), "utf8"));

// The pre-installed Chromium is pinned to whatever build the image carries,
// which will not match every Playwright release. CHROMIUM_PATH lets a caller
// point at it rather than downloading a second browser; without it Playwright
// uses its own, which is right on a developer machine.
// Same reasoning as the resolver above: the harness is only useful if it runs
// without ceremony. If CHROMIUM_PATH is unset, a pre-installed browser at the
// path our images use is picked up automatically; failing that Playwright
// downloads and manages its own, which is right on a developer machine.
const PREINSTALLED_CHROMIUM = "/opt/pw-browsers/chromium";
const executablePath =
  process.env.CHROMIUM_PATH ||
  (existsSync(PREINSTALLED_CHROMIUM) ? PREINSTALLED_CHROMIUM : undefined);
// Served over http rather than opened as a file: Chromium refuses `fetch`
// on `file://`, so the page would load and then sit empty forever. Ten lines
// of static server beats inlining the JSON into the HTML, which would put a
// second copy of the tokens somewhere it could go stale.
const types = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".json": "application/json" };
const server = createServer((request, response) => {
  const name = request.url === "/" ? "/index.html" : request.url.split("?")[0];
  const file = join(here, name);
  if (!file.startsWith(here)) {
    response.writeHead(403).end();
    return;
  }
  const stream = createReadStream(file);
  // The error has to be handled on the stream itself: a missing file emits
  // it before any pipe, and an unhandled 'error' takes the whole process
  // down -- which is how a favicon request killed the first run.
  stream.on("error", () => response.writeHead(404).end());
  stream.on("open", () => {
    response.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
    stream.pipe(response);
  });
});
await new Promise((ready) => server.listen(0, "127.0.0.1", ready));
const origin = `http://127.0.0.1:${server.address().port}`;

const browser = await chromium.launch(executablePath ? { executablePath } : {});
const page = await browser.newPage({ viewport: { width: 1320, height: 1000 } });
await page.emulateMedia({ reducedMotion: "reduce" });
await page.addStyleTag({ content: "*, *::before, *::after { animation: none !important; transition: none !important; }" }).catch(() => {});
await page.goto(origin);
await page.waitForFunction(() => document.querySelectorAll("#state option").length > 0);

// Every state, in the default preset. This is the set that existed before
// presets did, and its filenames are unchanged so a comparison against an
// older run still lines up.
for (const appearance of states.appearances.map((a) => a.name)) {
  await page.selectOption("#appearance", appearance);
  for (const [index, state] of states.states.entries()) {
    await page.selectOption("#state", String(index));
    await page.addStyleTag({ content: "* { animation: none !important; }" });
    const screen = page.locator("#screen");
    const file = join(out, `${appearance}-${state.name}.png`);
    await screen.screenshot({ path: file });
    console.log(`${appearance}/${state.name} -> ${file}`);
  }
}

// Then every preset, in one representative state (#84).
//
// One state rather than all six: the presets differ in chrome, and six near
// identical pictures per preset is the kind of large snapshot set that gets
// rubber-stamped -- the same argument #13 makes for keeping the snapshot suite
// deliberately small. `typing` is chosen because it is the only state showing
// the field, a populated list and a selection at once, which is where every
// trait a preset varies is visible.
const REPRESENTATIVE = "typing";
const representativeIndex = states.states.findIndex((s) => s.name === REPRESENTATIVE);
if (representativeIndex < 0) {
  throw new Error(`no "${REPRESENTATIVE}" state to shoot the presets in — it was renamed or removed`);
}
if (!states.presets?.length) {
  throw new Error("states.json carries no presets — the Rust emitter changed");
}

await page.selectOption("#state", String(representativeIndex));
for (const appearance of states.appearances.map((a) => a.name)) {
  await page.selectOption("#appearance", appearance);
  for (const preset of states.presets) {
    await page.selectOption("#preset", preset.name);
    await page.addStyleTag({ content: "* { animation: none !important; }" });
    const screen = page.locator("#screen");
    const file = join(out, `${appearance}-preset-${preset.name}.png`);
    await screen.screenshot({ path: file });
    console.log(`${appearance}/preset ${preset.name} -> ${file}`);
  }
}
await page.selectOption("#preset", "gnome");

await browser.close();
server.close();
