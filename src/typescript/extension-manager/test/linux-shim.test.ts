// The runtime's macOS shim, against a fake Homebrew in a temporary prefix.
// Nothing here runs the machine's own `brew`, reads the real home directory
// or touches the real clipboard: the "engine" is a function in this file
// that runs the fake `brew`, as the real engine would run Linuxbrew's.

import * as assert from "node:assert/strict";
import * as childProcess from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { after, before, describe, it } from "node:test";
import { promisify } from "node:util";
import { planLine, wrapChildProcess } from "../src/linux-shim/child-process";
import { wrapFs } from "../src/linux-shim/fs";
import {
	contextFor,
	type HostCommandRequest,
	type HostCommandResult,
	jobsFor,
	type ShimOptions,
} from "../src/linux-shim/index";
import { hostProgramsFor, SHIPPED } from "../src/linux-shim/overrides";
import { applyPatches, installPatches } from "../src/linux-shim/patches";
import { decide, mapPath, type ShimContext } from "../src/linux-shim/rules";
import { parse } from "../src/linux-shim/shell";
import { callSync, type SyncPort } from "../src/linux-shim/sync-rpc";

let root: string;
let prefix: string;
let bin: string;
const installed = JSON.stringify({
	formulae: [
		{
			name: "wget",
			desc: "Internet file retriever",
			installed: [{ version: "1.25.0" }],
		},
	],
	casks: [],
});

/** Runs the fake `brew` the way the engine's broker runs the real one. */
const engine = (request: HostCommandRequest): HostCommandResult => {
	const run = childProcess.spawnSync(
		path.join(bin, request.program),
		request.args,
		{
			input: request.input,
			encoding: "utf8",
			env: { PATH: `${bin}:/usr/bin:/bin`, FAKE_PREFIX: prefix },
		},
	);
	return { exitCode: run.status ?? -1, stdout: run.stdout, stderr: run.stderr };
};

/** A worker port whose engine answers `HostCommand/run` with {@link engine}. */
const enginePort = (seen: string[] = []): SyncPort => {
	const inbox: string[] = [];
	return {
		post: (message) => {
			const call = JSON.parse(message);
			seen.push(call.method);
			// Something unrelated arrives first, as it can on a real port.
			inbox.push(
				JSON.stringify({
					jsonrpc: "2.0",
					method: "Lifecycle/other",
					params: {},
				}),
			);
			const answer =
				call.method === "HostCommand/run"
					? { jsonrpc: "2.0", id: call.id, result: engine(call.params.request) }
					: {
							jsonrpc: "2.0",
							id: call.id,
							result: { text: "from the clipboard" },
						};
			inbox.push(
				JSON.stringify({
					jsonrpc: "2.0",
					id: 1,
					method: "Lifecycle/send_message",
					params: { msg: JSON.stringify(answer) },
				}),
			);
		},
		receive: () => inbox.shift(),
		deliver: () => {},
		sleep: () => {},
		now: () => 0,
	};
};

const options = (overrides: Partial<ShimOptions> = {}): ShimOptions => ({
	extensionName: "brew",
	raycast: true,
	entrypoint: path.join(root, "ext", "installed.js"),
	runOnHost: async (request) => engine(request),
	copy: async () => {},
	readText: async () => "clipboard text",
	port: enginePort(),
	log: () => {},
	env: { PATH: `${root}/path`, HOMEBREW_PREFIX: prefix },
	home: path.join(root, "home"),
	...overrides,
});

const shimmed = (overrides: Partial<ShimOptions> = {}) => {
	const opts = options(overrides);
	const context = contextFor(opts);
	return wrapChildProcess(childProcess, context, jobsFor(opts, context));
};

before(() => {
	root = fs.mkdtempSync(path.join(os.tmpdir(), "linux-shim-"));
	prefix = path.join(root, "linuxbrew");
	bin = path.join(prefix, "bin");
	fs.mkdirSync(bin, { recursive: true });
	fs.mkdirSync(path.join(root, "path"));
	fs.mkdirSync(path.join(root, "home"));
	fs.writeFileSync(path.join(root, "installed.json"), installed);
	fs.writeFileSync(
		path.join(bin, "brew"),
		[
			"#!/bin/sh",
			'case "$1" in',
			'  --prefix) echo "$FAKE_PREFIX" ;;',
			'  --cache) echo "$FAKE_PREFIX/cache" ;;',
			`  info) cat "${path.join(root, "installed.json")}" ;;`,
			'  list) echo wget; echo "$HOMEBREW_NO_AUTO_UPDATE" ;;',
			'  *) echo "Error: Unknown command: $1" >&2; exit 1 ;;',
			"esac",
			"",
		].join("\n"),
		{ mode: 0o755 },
	);
});

after(() => fs.rmSync(root, { recursive: true, force: true }));

describe("shell lines", () => {
	it("takes a simple command apart into its words", () => {
		const parsed = parse(`"/opt/homebrew/bin/brew" info --json=v2 'a b' c\\ d`);
		assert.ok(parsed.simple);
		assert.deepEqual(
			parsed.words.map((w) => w.value),
			["/opt/homebrew/bin/brew", "info", "--json=v2", "a b", "c d"],
		);
	});

	it("leaves a line that needs a shell to the shell, and finds its commands", () => {
		for (const line of [
			"brew list | grep x",
			"echo $HOME",
			"ls *.txt",
			"A=1 brew",
			"brew > out",
		]) {
			assert.equal(parse(line).simple, false, line);
		}
		const parsed = parse("echo hi | pbcopy && open 'x y'");
		assert.ok(!parsed.simple);
		assert.deepEqual(
			parsed.heads.map((w) => w.value),
			["echo", "pbcopy", "open"],
		);
	});
});

describe("the rules", () => {
	const context = (overrides: Partial<ShimContext> = {}): ShimContext => ({
		title: "Brew",
		raycast: true,
		hostPrograms: new Set(["brew"]),
		commands: {},
		paths: [],
		homebrewPrefix: "/home/linuxbrew/.linuxbrew",
		learntPrefixes: new Set(),
		onPath: () => false,
		...overrides,
	});

	it("sends brew to the host wherever the extension thinks it is", () => {
		for (const file of [
			"brew",
			"/opt/homebrew/bin/brew",
			"/usr/local/bin/brew",
			"/home/linuxbrew/.linuxbrew/bin/brew",
		]) {
			assert.deepEqual(decide(file, ["list"], context()), {
				kind: "host",
				program: "brew",
				args: ["list"],
			});
		}
	});

	it("brokers brew for extensions not written for macOS too, and nothing else", () => {
		const vicinae = context({ raycast: false });
		assert.equal(decide("brew", [], vicinae).kind, "host");
		assert.equal(decide("open", ["x"], vicinae).kind, "pass");
		assert.equal(decide("osascript", [], vicinae).kind, "pass");
	});

	it("refuses another Homebrew program by name rather than failing to run it", () => {
		const decision = decide(
			"/opt/homebrew/bin/op",
			["item", "list"],
			context(),
		);
		assert.equal(decision.kind, "refuse");
		assert.match((decision as { message: string }).message, /Homebrew/);
		assert.equal(decide("/usr/local/bin/op", [], context()).kind, "pass");
	});

	it("opens with xdg-open", () => {
		assert.deepEqual(decide("open", ["https://brew.sh"], context()), {
			kind: "rewrite",
			file: "xdg-open",
			args: ["https://brew.sh"],
		});
		assert.deepEqual(
			decide("/usr/bin/open", ["-a", "Safari", "/tmp/x"], context()),
			{
				kind: "rewrite",
				file: "xdg-open",
				args: ["/tmp/x"],
			},
		);
		assert.deepEqual(decide("open", ["-R", "/tmp/dir/file"], context()), {
			kind: "rewrite",
			file: "xdg-open",
			args: ["/tmp/dir"],
		});
		assert.equal(decide("open", ["-a", "Terminal"], context()).kind, "refuse");
	});

	it("answers the clipboard tools itself and refuses AppleScript by name", () => {
		assert.equal(decide("pbcopy", [], context()).kind, "clipboard-copy");
		assert.equal(decide("pbpaste", [], context()).kind, "clipboard-paste");
		const applescript = decide("osascript", ["-e", "beep"], context());
		assert.equal(applescript.kind, "refuse");
		assert.match((applescript as { message: string }).message, /AppleScript/);
	});

	it("refuses a macOS-only program only when Linux does not have one of that name", () => {
		assert.equal(decide("defaults", ["read"], context()).kind, "refuse");
		assert.equal(
			decide("say", ["hi"], context({ onPath: (n) => n === "say" })).kind,
			"pass",
		);
	});

	it("maps Homebrew's macOS paths, and the manifest's", () => {
		const ctx = context({
			paths: [
				["/Users/me/Library/Application Support/Code", "/home/me/.config/Code"],
			],
		});
		assert.equal(
			mapPath("/opt/homebrew/Cellar/wget", ctx),
			"/home/linuxbrew/.linuxbrew/Cellar/wget",
		);
		assert.equal(
			mapPath("/usr/local/Caskroom", ctx),
			"/home/linuxbrew/.linuxbrew/Caskroom",
		);
		assert.equal(mapPath("/usr/local/lib/x", ctx), undefined);
		assert.equal(
			mapPath(
				"/Users/me/Library/Application Support/Code/User/settings.json",
				ctx,
			),
			"/home/me/.config/Code/User/settings.json",
		);
		assert.equal(
			mapPath("/opt/homebrew/x", context({ raycast: false })),
			undefined,
		);
	});

	it("takes a program the manifest names in place of another", () => {
		assert.deepEqual(
			decide("gdate", ["+%s"], context({ commands: { gdate: "date" } })),
			{
				kind: "rewrite",
				file: "date",
				args: ["+%s"],
			},
		);
	});
});

describe("child_process, shimmed", () => {
	it("does what Raycast's Brew does when it starts: finds the prefix, then the cache", () => {
		const cp = shimmed();
		const found = cp.execSync("brew --prefix", { encoding: "utf8" }).trim();
		assert.equal(found, prefix);
		const cache = cp
			.execSync(`"${path.join(found, "bin", "brew")}" --cache`, {
				encoding: "utf8",
			})
			.trim();
		assert.equal(cache, `${prefix}/cache`);
	});

	it("learns the prefix the host's brew reports, for a prefix the sandbox cannot see", () => {
		const seen: string[] = [];
		const cp = shimmed({
			env: { PATH: `${root}/path` },
			port: enginePort(seen),
		});
		const found = cp.execSync("brew --prefix", { encoding: "utf8" }).trim();
		assert.equal(found, prefix);
		cp.execSync(`"${found}/bin/brew" --cache`);
		assert.deepEqual(seen, ["HostCommand/run", "HostCommand/run"]);
	});

	it("lists what is installed through promisified exec, as Show Installed does", async () => {
		const exec = promisify(shimmed().exec);
		const { stdout } = await exec(
			"/opt/homebrew/bin/brew info --json=v2 --installed",
			{
				env: { ...process.env, HOMEBREW_NO_AUTO_UPDATE: "1" },
			},
		);
		assert.deepEqual(JSON.parse(stdout), JSON.parse(installed));
	});

	it("streams a spawned brew and forwards only Homebrew's switches", async () => {
		const seen: HostCommandRequest[] = [];
		const cp = shimmed({
			runOnHost: async (request) => {
				seen.push(request);
				return engine(request);
			},
		});
		const child = cp.spawn("/opt/homebrew/bin/brew", ["list"], {
			env: { HOMEBREW_NO_AUTO_UPDATE: "1", LD_PRELOAD: "/evil.so" },
			stdio: ["ignore", "pipe", "pipe"],
		});
		let out = "";
		child.stdout?.on("data", (chunk) => {
			out += chunk;
		});
		const code = await new Promise((resolve) => child.on("close", resolve));
		assert.equal(code, 0);
		assert.equal(out, "wget\n\n");
		assert.deepEqual(seen[0].env, [
			{ name: "HOMEBREW_NO_AUTO_UPDATE", value: "1" },
		]);
	});

	it("reports a failing brew the way exec does", async () => {
		const error = await new Promise<any>((resolve) =>
			shimmed().execFile("brew", ["frobnicate"], (err) => resolve(err)),
		);
		assert.equal(error.code, 1);
		assert.match(error.stderr, /Unknown command/);
		assert.throws(
			() => shimmed().execSync("brew frobnicate"),
			(e: any) => e.status === 1,
		);
	});

	it("carries the engine's refusal to the extension by name, not as ENOENT", async () => {
		const cp = shimmed({
			runOnHost: async () => {
				throw "You did not allow brew to run brew on your computer";
			},
		});
		const error = await promisify(cp.exec)("brew list").catch((e) => e);
		assert.equal(error.code, "ENOTSUP");
		assert.match(error.message, /did not allow/);
	});

	it("refuses AppleScript by name in every form", async () => {
		const cp = shimmed();
		const viaExec = await promisify(cp.exec)(
			`osascript -e 'tell application "Finder" to beep'`,
		).catch((e) => e);
		assert.equal(viaExec.name, "CompassRefusal");
		const viaSpawn = cp.spawn("osascript", ["-e", "beep"]);
		const error = await new Promise<any>((resolve) =>
			viaSpawn.on("error", resolve),
		);
		assert.match(error.message, /AppleScript/);
		assert.equal(
			cp.spawnSync("/usr/bin/osascript", ["-e", "beep"]).error?.name,
			"CompassRefusal",
		);
	});

	it("copies and pastes through the runtime's clipboard", async () => {
		const copied: string[] = [];
		const cp = shimmed({ copy: async (text) => copied.push(text) });
		const child = cp.spawn("pbcopy");
		child.stdin?.end("hello");
		await new Promise((resolve) => child.on("close", resolve));
		assert.deepEqual(copied, ["hello"]);
		const { stdout } = await promisify(cp.exec)("pbpaste");
		assert.equal(stdout, "clipboard text");
		assert.equal(
			cp.execSync("pbpaste", { encoding: "utf8" }),
			"from the clipboard",
		);
	});

	it("opens with xdg-open, and leaves everything else alone", () => {
		const context = contextFor(options());
		const jobs = jobsFor(options());
		assert.deepEqual(planLine("open https://brew.sh", {}, context, jobs), {
			kind: "real",
			file: "xdg-open https://brew.sh",
			args: [],
			shell: true,
		});
		assert.deepEqual(planLine("echo x | open -f", {}, context, jobs), {
			kind: "real",
			file: "echo x | xdg-open -f",
			args: [],
			shell: true,
		});
		assert.equal(planLine("echo hello", {}, context, jobs), undefined);
		assert.equal(
			shimmed().execSync("echo hello", { encoding: "utf8" }),
			"hello\n",
		);
	});

	it("refuses brew inside a shell pipeline by name rather than running it sandboxed", () => {
		assert.throws(
			() => shimmed().execSync("brew list | grep wget"),
			(e: any) =>
				e.name === "CompassRefusal" && /shell command line/.test(e.message),
		);
	});
});

describe("fs, shimmed", () => {
	it("finds Homebrew where macOS keeps it", () => {
		const shimmedFs = wrapFs(fs, contextFor(options()));
		assert.equal(shimmedFs.existsSync("/opt/homebrew/bin/brew"), true);
		assert.equal(shimmedFs.statSync("/opt/homebrew/bin").isDirectory(), true);
		assert.equal(shimmedFs.existsSync("/opt/homebrew/bin/nothing"), false);
		assert.equal(
			shimmedFs.readFileSync,
			shimmedFs.readFileSync,
			"stable identity",
		);
		assert.equal(
			shimmedFs.constants,
			fs.constants,
			"the rest is the real module",
		);
	});

	it("maps the promise API too", async () => {
		const shimmedFs = wrapFs(fs, contextFor(options()));
		await shimmedFs.promises.access("/opt/homebrew/bin/brew");
	});
});

describe("blocking calls", () => {
	it("hands back everything it held, in order, once it has its answer", async () => {
		const delivered: string[] = [];
		const port = { ...enginePort(), deliver: (m: string) => delivered.push(m) };
		const result = callSync<HostCommandResult>(
			port,
			"HostCommand/run",
			{
				request: { program: "brew", args: ["--prefix"], env: [] },
			},
			1000,
		);
		assert.equal(result.stdout.trim(), prefix);
		assert.equal(
			delivered.length,
			0,
			"nothing is delivered while the caller runs",
		);
		await Promise.resolve();
		assert.equal(delivered.length, 2);
		assert.match(delivered[0], /Lifecycle\/other/);
	});

	it("gives up with a sentence rather than hanging", () => {
		let now = 0;
		const silent: SyncPort = {
			post: () => {},
			receive: () => undefined,
			deliver: () => {},
			sleep: () => {
				now += 100;
			},
			now: () => now,
		};
		assert.throws(
			() => callSync(silent, "HostCommand/run", {}, 1000),
			/no answer from Compass/,
		);
	});
});

describe("the overrides manifest", () => {
	it("brokers brew for every extension", () => {
		assert.ok(hostProgramsFor("store.raycast.brew").has("brew"));
		assert.ok(hostProgramsFor("store.vicinae.linuxbrew").has("brew"));
		assert.ok(!hostProgramsFor("store.raycast.brew").has("sh"));
		assert.equal(SHIPPED.version, 1);
	});

	it("patches a file's text, and skips a patch whose text is gone", () => {
		const logged: string[] = [];
		const patches = [
			{ file: "a.js", find: "darwin", replace: "linux", why: "w" },
			{ file: "a.js", find: "absent", replace: "x", why: "gone" },
			{ file: "b.js", find: "darwin", replace: "nope", why: "other file" },
		];
		assert.equal(
			applyPatches("darwin darwin", "a.js", patches, (m) => logged.push(m)),
			"linux linux",
		);
		assert.equal(logged.length, 1);
		assert.match(logged[0], /gone/);
	});

	it("applies patches as the extension's files load", () => {
		const dir = path.join(root, "patched");
		fs.mkdirSync(dir);
		fs.writeFileSync(path.join(dir, "cmd.js"), 'module.exports = "darwin";\n');
		assert.ok(
			installPatches(
				dir,
				[{ file: "cmd.js", find: "darwin", replace: "linux", why: "w" }],
				() => {},
			),
		);
		assert.equal(require(path.join(dir, "cmd.js")), "linux");
	});
});
