# Seeing the launcher: three harnesses, and what each one proves

Written for whoever picks this up next, human or agent. The launcher is a
graphical program and this repository is usually worked on from a container
with no compositor, so "look at it" is a question with three different answers
and they are not interchangeable.

| Harness | Renders with | Round trip | Proves |
|---|---|---|---|
| Browser surrogate (`tools/design/`) | HTML + CSS | ~2 s | layout, colour, hierarchy, state |
| wgpu in a canvas (**not built**) | the real Iced + wgpu, on WebGL | ~1 min | that Iced paints what you designed |
| VM tier (`scripts/vmtest/launcher.sh`) | the real thing on real GNOME | ~30 min | that it works on the target |

Use the cheapest one that can answer your question, and **do not let a cheap
one answer an expensive one's question**. That is the whole discipline here.

## 1. The browser surrogate — built, use it first

```sh
make design         # serve at http://127.0.0.1:8173
make design-shots   # screenshot every state, both appearances
```

`tools/design/README.md` has the details. The short version: the states come
from the real view models (`root_list::build`, `action_panel::flatten`) and the
colours and sizes come from `crates/compass-ui/src/design.rs`, both delivered
as `states.json`. The page retypes neither, so it cannot quietly disagree with
the Rust.

It does **not** prove Iced paints the same picture. It is HTML.

## 2. The wgpu path — not built, and here is exactly what it takes

This is the missing middle: the *real* widget code and the *real* renderer, in
a canvas Playwright can drive, without booting a VM.

**It is supported by the iced we already depend on.** Verified against the
vendored source rather than the changelog:

- `iced_winit 0.14` declares `wasm32` dependencies — `web-sys` with
  `HtmlCanvasElement`, and `wasm-bindgen-futures`
  (`iced_winit-0.14.0/Cargo.toml`, the `cfg(target_arch = "wasm32")` block).
- `iced 0.14` has a `webgl` feature that routes to `iced_wgpu/webgl`
  (`iced-0.14.0/Cargo.toml`), which is how wgpu reaches a browser.

So the renderer is the same wgpu pipeline, on WebGL instead of Vulkan. That is
a far smaller gap than HTML, and much smaller than the gap `iced_web` would
have.

**What blocks it, concretely.** `compass-ui` depends on `compass-core`,
`compass-xdg` and `compass-platform`, and through them on filesystem scanning,
D-Bus and process spawning — none of which builds for `wasm32`. The work is:

1. A `wasm32` entry point beside `run()` that constructs `LauncherApp` from a
   **fixed in-memory index** rather than `AppIndex::from_environment()`. The
   view models are pure, so this is a corpus and a constructor, not a port.
2. A `NullLauncher`-style no-op for the launch action — `compass-platform`
   already has the seam (ADR-0013).
3. `iced` with `features = ["webgl"]`, built with `wasm-pack` or `trunk`, into
   `tools/design/wasm/`.
4. Cargo config so `wasm32-unknown-unknown` does not try to build the crates
   that cannot: a separate thin crate depending only on `compass-ui` and the
   view models is likely cleaner than feature-gating four crates.

**Do not resurrect `iced_web`.** It is archived, targets the Iced 0.1/0.2 API,
and renders to the **DOM** through dodrio rather than through wgpu. Reviving it
would be a multi-version port *and* would paint through a renderer we do not
ship — a fidelity harness that is not faithful is worse than no harness,
because it looks authoritative.

**What it would still not prove:** that it works on GNOME under Flatpak with a
real compositor and a real icon theme. That stays the VM tier's job. WebGL is
not Vulkan and llvmpipe is neither.

## 3. The VM tier — the authority

`scripts/vmtest/launcher.sh`, driven by corral, on a real Bluefin VM. See
PLAN.md §8.9. It is the only thing that can fail for a reason the other two
cannot see, and it found #91 and #95 exactly that way.

## Playwright mechanics that will bite you

Every one of these cost a run to discover.

**The pre-installed Chromium is pinned and will not match your Playwright.**
The image ships browsers under `/opt/pw-browsers` (`chromium-1194` at the time
of writing) and `PLAYWRIGHT_BROWSERS_PATH` points there. A freshly installed
`playwright` package expects a different build number and dies with
"Executable doesn't exist". Do **not** run `npx playwright install` — point at
what is there:

```sh
CHROMIUM_PATH=/opt/pw-browsers/chromium-1194/chrome-linux/chrome \
  node tools/design/shoot.mjs
```

`shoot.mjs` reads `CHROMIUM_PATH` and falls back to Playwright's own browser
when it is unset, which is right on a developer machine.

**`fetch` does not work on `file://`.** Chromium refuses it, so a page opened
as a file loads and then sits empty forever with no error you will notice.
`shoot.mjs` runs a ten-line static server on a random port for this reason.
Inlining the JSON into the HTML would also work and is worse: it puts a second
copy of the tokens somewhere it can go stale.

**Handle the stream error, not just the pipe.** A missing file — a browser
asking for `/favicon.ico`, say — emits `error` on the `ReadStream` before any
pipe, and an unhandled `error` takes the whole process down. Attach the handler
to the stream.

**Disable animations before screenshotting.** The search caret blinks; without
`emulateMedia({ reducedMotion: "reduce" })` and an `animation: none` style tag,
half your screenshots differ from the other half for no reason anyone can read.

**Reading frames the VM tier produced.** Its artifacts are downloadable and
worth looking at rather than trusting a green tick:

```sh
# the job log, via its signed URL — grep locally rather than pulling
# thousands of lines of GL extension spam through a tool result
curl -sS -o job.log "<logs_url from get_job_logs with return_content=false>"

# the frames
curl -sS -L -o launcher.zip \
  "https://api.github.com/repos/tuna-os/compass/actions/artifacts/<id>/zip"
unzip -q launcher.zip -d art
```

**The system Python's Pillow is broken in this image** (`cannot import name
'_imaging'`). Make a venv:

```sh
python3 -m venv /tmp/venv && /tmp/venv/bin/pip install -q pillow
```

Then crop the changed region out of a before/after pair and actually look at
it. A framediff tells you *that* something changed; only the image tells you
*what*, and the difference between those two mattered: the Ctrl+B assertion
reported 2,697 changed pixels in a 107×133 box, and it took opening the frames
to know that the box contained the action panel rather than something else.
