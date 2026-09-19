// Screenshots every state in both appearances.
//
// Run: node tools/design/shoot.mjs [outdir]
//
// Animations are disabled so the caret is captured in the same phase every
// time: a screenshot that differs because of a blink is a diff nobody can
// read.

import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import { dirname, extname, join, resolve } from "node:path";
import { createReadStream, mkdirSync, readFileSync } from "node:fs";
import { createServer } from "node:http";

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(process.argv[2] ?? join(here, "shots"));
mkdirSync(out, { recursive: true });

const states = JSON.parse(readFileSync(join(here, "states.json"), "utf8"));

// The pre-installed Chromium is pinned to whatever build the image carries,
// which will not match every Playwright release. CHROMIUM_PATH lets a caller
// point at it rather than downloading a second browser; without it Playwright
// uses its own, which is right on a developer machine.
const executablePath = process.env.CHROMIUM_PATH || undefined;
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

await browser.close();
server.close();
