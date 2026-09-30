# Raycast extensions written for macOS: the shim, the broker and the overrides

**Date:** 2026-09-25. Raycast extensions are written for macOS and many of them
reach past `@raycast/api` into the Mac itself: they run `/opt/homebrew/bin/brew`,
`open -a Safari`, `pbcopy`, AppleScript. Compass offers them on Linux anyway
(`compass_core::raycast_store::available_on`), so the question is what makes
them work there without a per-extension fork. Three pieces, in the order they
act:

1. **The runtime's shim** (`src/typescript/extension-manager/src/linux-shim/`):
   a generic, invisible layer between an extension and Node's `child_process`
   and `fs`, which fixes what every macOS extension gets wrong the same way.
2. **The host-command broker** (`crates/compass/src/host_commands.rs`,
   `HostCommand/run` in `figura/tsapi.fig`): the few programs an extension may
   ask the engine to run on the host, outside its sandbox, once the person
   allows it. Homebrew's `brew` first.
3. **The overrides manifest** (`extensions/raycast-linux-overrides.json`):
   curated, per-extension data for what the first two cannot do generically.

## 1. What ties Raycast extensions to macOS, measured

`scripts/suite1/macos_signals.py` downloads the prebuilt bundles of the top 300
Raycast store listings by installs (the bundles a user's install gets, as
`scripts/suite1/fetch.py` fetches them) and counts what their JavaScript does
that only works on a Mac. Measured 2026-09-25. Of the 300, 106 list macOS as
their only platform, 158 macOS and Windows, and 36 list none.

| Signal in the bundle | Extensions | of which macOS-only listings | What makes it work on Linux |
|---|---:|---:|---|
| AppleScript source (`tell application`, JXA `Application("…")`, `.applescript`) | 80 | 53 | nothing: it drives macOS applications |
| `osascript` run directly (not through `@raycast/utils`) | 50 | 37 | nothing; the shim refuses it **by name** |
| `@raycast/utils`' `runAppleScript` bundled (called or not) | 98 | 54 | it already refuses off macOS ("AppleScript is only supported on macOS") |
| `~/Library/…` paths (`Application Support`, `Preferences`, `Group Containers`, …) | 39 | 21 | a manifest `paths` entry, where a Linux counterpart exists and the sandbox grants it |
| `/Applications/…`, `….app/Contents` | 31 | 13 | nothing generic |
| `open` (`open url`, `open -a`, `open -R`) | 28 | 18 | **shim**: `xdg-open` |
| a Homebrew prefix (`/opt/homebrew`, `/usr/local/{bin/brew,Cellar,Caskroom,Homebrew}`) | 27 | 15 | **shim + broker** for `brew`; the manifest's `hostPrograms` for another Homebrew tool |
| `process.platform === "darwin"` (outside the libraries above) | 44 | 11 | nothing to do: `process.platform` stays `linux` (below) |
| Mach-O binaries shipped in the bundle | 18 | 9 | nothing: a macOS executable |
| other macOS-only programs (`sw_vers`, `screencapture`, `afplay`, `say`, `shortcuts`, `launchctl`, `plutil`, …) | 17 | 4 | the shim refuses them by name when Linux has none of that name |
| `open -a <App>` (naming an application) | 15 | 6 | shim: the target opens with its default application; `-a` alone is refused by name |
| `/usr/local/bin/<tool>` other than `brew` | 13 | 6 | left alone: Linux users install there too |
| `defaults read/write` | 7 | 4 | refused by name |
| `mdfind` / `mdls` (Spotlight) | 4 | 3 | refused by name |
| `security` (Keychain CLI) | 2 | 1 | not handled |
| `pbcopy` / `pbpaste` | **0** | 0 | shim: the runtime's clipboard (kept, it is cheap, but nothing in the top 300 uses it: `Clipboard` replaced it) |

Some signal, of any kind: **153 of 300** (83 of them macOS-only listings); none:
147. AppleScript or JXA, the one thing nothing on Linux can stand in for: **90**.
Extensions whose *only* signals are ones the shim fixes (a Homebrew prefix,
`open`, `pbcopy`): **18** — `github`, `video-downloader`, `google-workspace`,
`screenshot`, `brew-services`, `aerospace`, `perplexity-api`, `shottr`,
`easy-ocr`, `aws`, `screen-saver`, `displayplacer`, `mcp`, `snippetslab`,
`numi`, `night-light`, `whisper-dictation`, `yafw` (several of which drive a
macOS application behind the `open` and are Linux-useless regardless). Of the
28 extensions Vicinae's compatibility sheet marks `impossible`, 26 carry a
signal here.

What a count is: minified bundles carry their libraries, so a pattern match is
a signal, not a proof. The script strips the known library noise
(`@raycast/utils`' runner, execa's shell list, the `open` package's `-a`
handling, a home-directory helper's `darwin` branch) and counts the runner
separately, since a bundle carries it whether or not the extension calls it.

**The Suite 1 ledger.** Suite 1's corpus deliberately excludes macOS-only and
`impossible` extensions (`scripts/suite1/select.py`), so its failures are
mostly not macOS's: 7 wait on an OAuth sign-in, 3 need input a headless run has
not got (a selection, an API key, a Warp tab config), `speedtest` cannot
execute the CLI it downloads into its own support directory, and one draws an
empty history. The one macOS failure is `visual-studio-code`'s crash, from its
`~/Library/Application Support/Code/…` paths (the compatibility sheet says the
same). A manifest `paths` entry could point it at `~/.config/Code`, but the
extension sandbox's `$HOME` allowlist does not grant that directory, so the
entry would change nothing yet; it is left for the maintainer's call on the
allowlist.

**Raycast's Brew, read.** It finds Homebrew with `execSync("brew --prefix")`
at load and, failing that, guesses `/opt/homebrew` on Apple silicon and
`/usr/local` elsewhere; every later command is `<prefix>/bin/brew …`, through
a promisified `exec` (a shell line) or `spawn`, with
`HOMEBREW_NO_AUTO_UPDATE`-style switches and a `SUDO_ASKPASS` script in its
environment. Search reads the formula and cask lists from
`formulae.brew.sh`, not from `brew`. Casks and `/Applications` only matter on
macOS; Linuxbrew reports no casks. Its `runAppleScript` is the "run in
Terminal" action, and fails by name. Nothing in it needs `process.platform` to
be `darwin`.

## 2. The runtime's shim

Installed per command, before the extension loads, as `require` overrides
(`patch-require.ts`) for `child_process`, `node:child_process`, `fs`,
`node:fs`, `fs/promises` and `node:fs/promises`. The extension gets the real
module behind a `Proxy` that replaces six functions (`spawn`, `spawnSync`,
`exec`, `execSync`, `execFile`, `execFileSync`, with `util.promisify` support
for the two that have it) and, for `fs`, only the calls that look a path up or
read it. Everything else on the module is the real thing; the runtime's own
code keeps the modules it already holds.

What it does, for every program an extension starts (`rules.ts`):

| The extension runs | The shim | For |
|---|---|---|
| a brokered program (`brew`), bare or from any Homebrew prefix: `/opt/homebrew/bin/brew`, `/usr/local/bin/brew`, `/home/linuxbrew/.linuxbrew/bin/brew`, or the prefix the host's `brew --prefix` answered | sends it to the engine (`HostCommand/run`) and hands back its output as the child process's | **every** extension |
| another program from a Homebrew prefix (`/opt/homebrew/bin/op`) | refuses it by name: Compass runs only the brokered programs for extensions | Raycast |
| `open …`, `/usr/bin/open …` | `xdg-open <target>`; `-R` opens the containing folder; `-a App target` opens the target with its default application; `-a App` alone is refused | Raycast |
| `pbcopy`, `pbpaste` | the runtime's own clipboard (`Clipboard/copy`, `Clipboard/readContent`) | Raycast |
| `osascript` | refused by name ("… needs macOS for this: it runs AppleScript (osascript) …") | Raycast |
| `defaults`, `mdfind`, `say`, `sw_vers`, … (`MACOS_ONLY`) when not on `PATH` | refused by name | Raycast |
| a program the manifest's `commands` maps | the mapped program | Raycast, per entry |
| a path under `/opt/homebrew`, `/usr/local/{Cellar,Caskroom,Homebrew}`, or a manifest `paths` prefix, as a program, an argument or an `fs` lookup | the Linux path | Raycast |

A refusal is an error with `name: "CompassRefusal"` and `code: "ENOTSUP"`,
delivered the way the call's own failures are (the `error` event, the
callback, the rejected promise, the throw), never an `ENOENT` from a missing
binary.

**Shell lines.** `exec("…")` and `shell: true` hand a line to a shell. A
simple command (words and quotes only) is taken apart (`shell.ts`) and
treated like the argument form, and runs without a shell when it is brokered.
A line that needs the shell (a pipe, a redirection, an expansion) stays one:
only its command words are changed, in place (`open` → `xdg-open`), and a line
that would need a brokered or answered program inside it (`brew list | grep x`)
is refused by name rather than run half-sandboxed.

**Blocking calls.** `execSync("brew --prefix")` has to be answered while the
extension's own thread is blocked, and API answers arrive on that thread's
message port. `sync-rpc.ts` sends the call, then takes messages off the port
itself (`worker_threads.receiveMessageOnPort`) until the answer is among them,
and delivers everything else it took, in order, once the caller has its
answer — the way a real `execSync` holds back every event until the child
exits.

**`process.platform` stays `linux`.** Spoofing `darwin` gains Brew nothing (its
only `darwin` check is `@raycast/utils`' AppleScript guard), and the 44
extensions that check it mostly choose a macOS path or program by it; the
libraries bundled with them (execa, `open`, clipboard helpers) would start
spawning macOS binaries. A truthful platform makes them take their non-macOS
branch, which is at worst the Windows one.

**Scope.** The macOS rewrites apply to extensions from the Raycast store only
(`is_raycast`); brokering applies to every extension, since the sandbox that
stops Raycast's Brew from running `brew` stops the Vicinae store's `linuxbrew`
extension the same way.

## 3. The host-command broker

The extension sandbox (`crates/compass-sandbox`, `extension_runner::policy_in`)
grants execute on `/usr`, `/bin`, `/lib`, `/app`, Node and the extension's own
directory. `/home/linuxbrew/.linuxbrew` is none of them, and inside the Flatpak
it is not even visible (the app has `--filesystem=home:ro`, and `/home/linuxbrew`
is not under the user's home). Rather than widen either, `brew` runs as the
engine's child, on the host, when the person says so:

1. The shim sends `HostCommand/run` with the program's **name**, its
   arguments, optional standard input and the extension's environment
   switches. The call always defers (`compass_worker_host::host_command_service`),
   and a program that is a path, an option or anything but a plain name is
   refused before anyone is asked: the extension does not get to pick the
   binary the person is agreeing to.
2. The engine refuses a program no extension is known to need: the manifest's
   top-level `hostPrograms` (`brew`) for every extension, plus an entry's own.
3. Allowed "always" (the grants file) or "once" already in this run: it runs.
   Otherwise the person is asked in the command's view — **"Allow Brew to run
   brew?"**, "Brew wants to run brew on your computer, outside its sandbox and
   with your permissions. Allow Once lasts until you close Brew. Always Allow
   is remembered, and Script Permissions can take it back." — with **Enter:
   Allow Once**, **Ctrl+Enter: Always Allow**, **Esc: Deny**. Calls made while
   the question is open wait for its one answer. A command with no view has
   nowhere to ask and is refused by name.
4. It runs `env PATH=<search path> [HOMEBREW_NO_…=…] brew <args>`: directly
   outside the Flatpak, through `flatpak-spawn --host`
   (`compass_platform_linux::host_command`) inside it. The search path is the
   engine's `PATH` (the host's system directories inside the Flatpak) followed
   by `/home/linuxbrew/.linuxbrew/{bin,sbin}` and `~/.linuxbrew/bin`, so a
   `brew` not on `PATH` is found in the Linuxbrew prefix. Of the extension's
   environment only Homebrew's switches cross (`HOMEBREW_NO_*`,
   `HOMEBREW_COLOR`, `HOMEBREW_VERBOSE`, `HOMEBREW_DOWNLOAD_CONCURRENCY`):
   never a variable that names a program or a path (`SUDO_ASKPASS`,
   `HOMEBREW_BROWSER`, `LD_PRELOAD`, `PATH`), since the program runs unconfined.
5. The answer is the exit status and output. Denied: "You did not allow Brew to
   run brew on your computer". Not installed: "brew is not installed on this
   computer: Compass looked for it on PATH and in the Linuxbrew prefix, …".

**Where "Always Allow" is kept:** `$XDG_CONFIG_HOME/compass/host-command-grants.json`,
beside the Rhai scripts' `script-grants.json`:

```json
{ "extensions": { "store.raycast.brew": ["brew"] } }
```

Script Permissions (`commands:script-permissions`) lists each extension's
grants beside the Rhai scripts', as `host.run:brew` / "run brew on your
computer, outside the extension sandbox", and Enter revokes them. On the wire
that is `ListScriptGrants` and `RevokeScriptGrant { id }` with an extension's
id, and the alert's third answer is `ExtensionAlert::remember_text` with
`Request::ExtensionAlertRemember` (IPC v22). "Allow Once" is held in memory for
the run of the command and never written.

What running `brew` this way does not do: stream (the output arrives when it
exits, so a long `brew upgrade` shows nothing until it ends), cancel (an
`AbortSignal` or `kill()` does not reach the host process), or answer a
`sudo` prompt (`SUDO_ASKPASS` does not cross; Linuxbrew does not need `sudo`).

**The Qt engine** refuses `HostCommand/run` by name
(`src/server/src/extension/api/host-command-service.hpp`): the IDL is shared,
the broker is the Rust engine's.

## 4. The overrides manifest

`extensions/raycast-linux-overrides.json`, read by the runtime (bundled into
`runtime.js`) and by the engine (`compass_core::raycast_overrides`, embedded
with `include_str!`). Unknown fields are refused, so a misspelt one fails the
engine's tests instead of being ignored.

```jsonc
{
  "version": 1,
  // Programs every extension may ask the engine to run on the host.
  "hostPrograms": ["brew"],
  "extensions": {
    // The installed id: store.raycast.<name>.
    "store.raycast.example": {
      "why": "What the extension does on macOS that needs this entry.",   // required
      "hostPrograms": ["op"],        // more programs it may ask to run on the host
      "paths": {                     // path prefixes, macOS to Linux; ~ is the home directory
        "~/Library/Application Support/Code": "~/.config/Code"
      },
      "commands": { "gdate": "date" },  // a program run in place of another
      "patches": [                   // literal replacements when a bundled file loads
        { "file": "index.js", "find": "exact text", "replace": "new text", "why": "…" }
      ],
      "redirect": {                  // install a Linux-capable extension instead
        "store": "vicinae", "author": "domainus", "name": "linuxbrew",
        "why": "Why this one cannot be made to work."
      }
    }
  }
}
```

- `hostPrograms` is the only field that widens anything, and only as far as a
  question to the person: the program still runs only when they allow it. The
  engine enforces it; the runtime reads it to know what to send.
- `paths` and `commands` are the shim's, for Raycast extensions.
- `patches` are applied as the file loads (`module.registerHooks`, Node 22.15+),
  never to the installed file, so a store update replaces the file and the
  patch applies to the new one — or, if its text is gone, is skipped and
  logged. Patch the minified bundle's text, and keep `find` as short as is
  unambiguous.
- `redirect` makes the engine install the named Vicinae store extension when
  the person installs the Raycast one (`Request::StoreInstall`).

**Adding an entry.** Find why the extension fails (the Suite 1 report's
detail, the extension's `support/<id>/.vicinae/stderr.txt`, or
`macos_signals.py`'s signals for it); prefer the smallest field that fixes it
(a `paths` or `commands` entry over a patch, a patch over a redirect); write
`why`; run `cargo test -p compass-core raycast_overrides` and the runtime's
`npm test`; and, if the extension is in Suite 1's corpus, record its new
outcome in `scripts/suite1/expected.json` with the reason.

The shipped manifest has one entry, `store.raycast.brew`, which names `brew`
explicitly although every extension may ask for it: it is the extension the
broker was built for and proven on.

## 5. Brew, proven

- `crates/compass/tests/engine_end_to_end.rs`
  `raycasts_brew_runs_homebrew_on_the_host_once_the_person_allows_it`: a
  Raycast-store stand-in doing exactly what Brew's Show Installed does
  (`execSync("brew --prefix")` at load, then the promisified `exec` of
  `<prefix>/bin/brew info --json=v2 --installed`), behind the real sandbox,
  with a fake `brew` in a temporary prefix. It asserts the consent prompt and
  its three answers, the list drawn from the host's `brew`, the grants file,
  Script Permissions' entry, no question on the second run, revocation, and a
  denial reaching the extension by name.
- The runtime's own tests (`src/typescript/extension-manager/test/`, `npm test`)
  run the shim against a fake `brew` in a temporary prefix: Brew's start-up
  sequence, promisified `exec`, `spawn` streaming, failures, refusals,
  `pbcopy`/`pbpaste`, `xdg-open`, the `fs` mapping, the blocking-call port, and
  load-time patches.
- **Raycast's real Brew bundle** (the store's build of 2026-09-25), run by hand
  through `compass conformance` behind the sandbox with a fake `brew` first on
  `PATH` and "Always Allow" recorded: **Show Installed: rendered, Search:
  rendered**; the fake saw `brew --prefix`, `brew --cache`,
  `brew info --json=v2 --installed` (and `brew --version` from Search), and the
  extension cached the two formulae the fake reported. Without the grant, a
  headless run times out on the unanswered question and `brew` never runs.

Brew is not in Suite 1's corpus (the corpus excludes macOS-only listings), so
the ledger does not record it.

## 6. The sandbox, and what was not widened

No policy was changed. Had `brew` been run from inside the sandbox instead,
it would have needed, for the Brew extension only:

- **read and execute** on `/home/linuxbrew/.linuxbrew` (Homebrew's Ruby,
  its `Library`, the kegs a formula's own programs are), for listing,
  searching and `info`;
- **write** on `/home/linuxbrew/.linuxbrew` for install, upgrade, uninstall
  and cleanup, and read and write on `~/.cache/Homebrew` (downloads and the
  API cache) — which the `$HOME` allowlist (`compass_sandbox::home`) grants
  nothing of;
- inside the Flatpak, a new `--filesystem=/home/linuxbrew/.linuxbrew` hole in
  the manifest, for every extension, since a Flatpak permission cannot be
  scoped to one extension or asked for at run time.

There is no per-extension grant mechanism in the policy today:
`compass_sandbox::Allowlist` (read, write, execute lists parsed from an
extension's manifest) exists but nothing builds a policy from it;
`extension_runner::policy_in` gives every extension the same one. The broker
makes that question moot for `brew`: the extension's policy stays as it is,
and the grant that matters — this extension may run this program — is the
person's, per extension and revocable.

## 7. What is left

- AppleScript and JXA (90 of the top 300) and the Mach-O binaries (18) cannot
  be shimmed; they are refused by name, and the compatibility sheet should
  keep calling them `impossible`.
- `~/Library` paths (39) need a manifest `paths` entry per extension *and* a
  sandbox grant for the Linux directory; the `$HOME` allowlist is the
  maintainer's call (VS Code's `~/.config/Code` is the first candidate).
- Other Homebrew tools extensions run (`op`, `bw`, `aws`, `yt-dlp`, `ffmpeg`
  in the measured set) can be brokered per extension with `hostPrograms`; none
  is listed until someone proves one.
- The broker returns output when the program exits: no streaming, no
  cancellation. Brew's install and upgrade progress parsers see it all at
  once.
- A command with no view cannot ask; the person has to open one of the
  extension's views first.
- `redirect` has no entry yet: Raycast's Brew works, so nothing is replaced.
