/**
 * The runtime's shim for extensions written for macOS, and the one path to
 * the engine's host-command broker.
 *
 * Installed per command, before the extension loads, as `require` overrides
 * for `child_process` and `fs` (and their `node:` and `/promises` names):
 * the extension gets the real modules behind a proxy, and the runtime's own
 * code keeps the ones it already holds. `process.platform` stays `linux`.
 *
 * `docs/rust-engine/RAYCAST-LINUX-SHIM.md` says what is covered and why.
 */

import * as path from "node:path";
import { wrapChildProcess, type Jobs } from "./child-process";
import { type Job, type RunResult, refusal } from "./fake-child";
import { wrapFs, wrapFsPromises } from "./fs";
import {
	entryFor,
	extensionId,
	hostProgramsFor,
	type Manifest,
	pathsFor,
	SHIPPED,
} from "./overrides";
import { installPatches } from "./patches";
import { LINUXBREW_PREFIX, type ShimContext } from "./rules";
import { callSync, type SyncPort } from "./sync-rpc";

// The real modules, taken when the runtime loads and before any extension
// does: the objects the proxies stand in front of.
const childProcess: typeof import("node:child_process") = require("node:child_process");
const fs: typeof import("node:fs") = require("node:fs");
const fsPromises: typeof import("node:fs/promises") = require("node:fs/promises");

/** How long a blocking call waits: long enough for a person to answer. */
const SYNC_TIMEOUT_MS = 5 * 60 * 1000;

export type HostCommandRequest = {
	program: string;
	args: string[];
	input?: string;
	env: { name: string; value: string }[];
};

export type HostCommandResult = {
	exitCode: number;
	stdout: string;
	stderr: string;
};

export type ShimOptions = {
	extensionName: string;
	/** The installed id, when the manager was given it. */
	extensionId?: string;
	raycast: boolean;
	/** Where the command's file is; its directory is the extension's. */
	entrypoint: string;
	/** `HostCommand/run` through the API client. */
	runOnHost: (request: HostCommandRequest) => Promise<HostCommandResult>;
	/** The clipboard through the API client. */
	copy: (text: string) => Promise<unknown>;
	readText: () => Promise<string | undefined>;
	/** For the blocking forms (`execSync`). */
	port: SyncPort;
	log: (message: string) => void;
	manifest?: Manifest;
	env?: NodeJS.ProcessEnv;
	home?: string;
	isFile?: (file: string) => boolean;
};

const errorResult = (error: unknown): RunResult => ({
	code: null,
	stdout: "",
	stderr: "",
	error: refusal(
		typeof error === "string"
			? error
			: error instanceof Error
				? error.message
				: String(error),
	),
});

/** The shim's context for one command. */
export const contextFor = (options: ShimOptions): ShimContext => {
	const env = options.env ?? process.env;
	const home = options.home ?? env.HOME ?? "";
	const isFile =
		options.isFile ??
		((file: string) => {
			try {
				return fs.statSync(file).isFile();
			} catch {
				return false;
			}
		});
	const id =
		options.extensionId ?? extensionId(options.extensionName, options.raycast);
	const manifest = options.manifest ?? SHIPPED;
	const entry = entryFor(id, manifest);
	const homebrewPrefix = [
		env.HOMEBREW_PREFIX,
		LINUXBREW_PREFIX,
		home && `${home}/.linuxbrew`,
	]
		.filter(
			(prefix): prefix is string =>
				typeof prefix === "string" && prefix.length > 0,
		)
		.find((prefix) => isFile(`${prefix}/bin/brew`));
	const pathDirs = (env.PATH ?? "").split(":").filter(Boolean);
	return {
		title: options.extensionName,
		raycast: options.raycast,
		hostPrograms: hostProgramsFor(id, manifest),
		commands: options.raycast ? (entry?.commands ?? {}) : {},
		paths: options.raycast ? pathsFor(entry, home) : [],
		homebrewPrefix,
		learntPrefixes: new Set(),
		onPath: (name) => pathDirs.some((dir) => isFile(path.join(dir, name))),
	};
};

export const jobsFor = (options: ShimOptions, context?: ShimContext): Jobs => {
	const host = (
		program: string,
		args: string[],
		env: Record<string, string>,
	): Job => {
		const request = (input: string): HostCommandRequest => ({
			program,
			args,
			env: Object.entries(env).map(([name, value]) => ({ name, value })),
			...(input ? { input } : {}),
		});
		const result = (answer: HostCommandResult): RunResult => {
			const prefix = answer.stdout.trim();
			if (
				context &&
				program === "brew" &&
				args.length === 1 &&
				args[0] === "--prefix" &&
				answer.exitCode === 0 &&
				path.isAbsolute(prefix)
			) {
				context.learntPrefixes.add(prefix);
			}
			return {
				code: answer.exitCode,
				stdout: answer.stdout,
				stderr: answer.stderr,
			};
		};
		return {
			readsStdin: false,
			run: (input) =>
				options.runOnHost(request(input)).then(result, errorResult),
			runSync: (input) => {
				try {
					return result(
						callSync<HostCommandResult>(
							options.port,
							"HostCommand/run",
							{ request: request(input) },
							SYNC_TIMEOUT_MS,
						),
					);
				} catch (error) {
					return errorResult(error);
				}
			},
		};
	};
	const done: RunResult = { code: 0, stdout: "", stderr: "" };
	return {
		host,
		clipboardCopy: () => ({
			readsStdin: true,
			run: (input) => options.copy(input).then(() => done, errorResult),
			runSync: (input) => {
				try {
					callSync(
						options.port,
						"Clipboard/copy",
						{ content: { text: input }, options: { concealed: false } },
						SYNC_TIMEOUT_MS,
					);
					return done;
				} catch (error) {
					return errorResult(error);
				}
			},
		}),
		clipboardPaste: () => ({
			readsStdin: false,
			run: () =>
				options
					.readText()
					.then(
						(text): RunResult => ({ code: 0, stdout: text ?? "", stderr: "" }),
						errorResult,
					),
			runSync: () => {
				try {
					const content = callSync<{ text?: string }>(
						options.port,
						"Clipboard/readContent",
						{},
						SYNC_TIMEOUT_MS,
					);
					return { code: 0, stdout: content?.text ?? "", stderr: "" };
				} catch (error) {
					return errorResult(error);
				}
			},
		}),
	};
};

/**
 * The `require` overrides that install the shim for one command, after
 * applying the manifest's load-time patches for it.
 */
export const installShim = (
	options: ShimOptions,
): Record<string, () => unknown> => {
	const context = contextFor(options);
	const jobs = jobsFor(options, context);
	const entry = entryFor(
		options.extensionId ?? extensionId(options.extensionName, options.raycast),
		options.manifest ?? SHIPPED,
	);
	if (options.raycast && entry?.patches?.length) {
		installPatches(
			path.dirname(options.entrypoint),
			entry.patches,
			options.log,
		);
	}

	const shimmed = wrapChildProcess(childProcess, context, jobs);
	const overrides: Record<string, () => unknown> = {
		child_process: () => shimmed,
		"node:child_process": () => shimmed,
	};
	if (context.raycast && (context.homebrewPrefix || context.paths.length > 0)) {
		const shimmedFs = wrapFs(fs, context);
		const shimmedPromises = wrapFsPromises(fsPromises, context);
		Object.assign(overrides, {
			fs: () => shimmedFs,
			"node:fs": () => shimmedFs,
			"fs/promises": () => shimmedPromises,
			"node:fs/promises": () => shimmedPromises,
		});
	}
	return overrides;
};
