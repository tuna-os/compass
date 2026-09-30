/**
 * `child_process` as an extension sees it: the real module, with the six
 * functions that start a program asking {@link decide} first.
 *
 * A program the rules leave alone reaches the real function with the
 * arguments it was given, untouched. The object is a proxy over the real
 * module, so everything else on it (`fork`, `ChildProcess`, properties a
 * library adds) is the real thing.
 */

import { promisify } from "node:util";
import {
	execLike,
	execPromise,
	execSyncLike,
	FakeChild,
	type Job,
	refusedJob,
	spawnSyncLike,
} from "./fake-child";
import { type Decision, decide, type ShimContext } from "./rules";
import { join, parse, splice } from "./shell";

/** Runs what the rules cannot leave to a real child process. */
export type Jobs = {
	host: (program: string, args: string[], env: Record<string, string>) => Job;
	clipboardCopy: () => Job;
	clipboardPaste: () => Job;
};

type Plan =
	| { kind: "real"; file: string; args: string[]; shell: boolean }
	| { kind: "fake"; job: Job; file: string; args: string[] };

const envOf = (options: any): Record<string, string> =>
	Object.fromEntries(
		Object.entries(options?.env ?? {}).filter(
			([name, value]) =>
				name.startsWith("HOMEBREW_") && typeof value === "string",
		),
	) as Record<string, string>;

const fromDecision = (
	decision: Decision,
	file: string,
	args: string[],
	options: any,
	jobs: Jobs,
): Plan | undefined => {
	switch (decision.kind) {
		case "pass":
			return undefined;
		case "rewrite":
			return {
				kind: "real",
				file: decision.file,
				args: decision.args,
				shell: false,
			};
		case "host":
			return {
				kind: "fake",
				job: jobs.host(decision.program, decision.args, envOf(options)),
				file,
				args,
			};
		case "clipboard-copy":
			return { kind: "fake", job: jobs.clipboardCopy(), file, args };
		case "clipboard-paste":
			return { kind: "fake", job: jobs.clipboardPaste(), file, args };
		case "refuse":
			return { kind: "fake", job: refusedJob(decision.message), file, args };
	}
};

/** What to do with a line a shell would run. */
export const planLine = (
	line: string,
	options: any,
	context: ShimContext,
	jobs: Jobs,
): Plan | undefined => {
	if (typeof line !== "string") return undefined;
	const parsed = parse(line);
	if (parsed.simple) {
		const [head, ...rest] = parsed.words.map((word) => word.value);
		const plan = fromDecision(
			decide(head, rest, context),
			head,
			rest,
			options,
			jobs,
		);
		if (plan?.kind === "real") {
			return {
				kind: "real",
				file: join([plan.file, ...plan.args]),
				args: [],
				shell: true,
			};
		}
		return plan;
	}

	// A line that needs a shell stays one. Only its command words change,
	// and only to another command: anything that cannot run inside a shell
	// line is refused whole, by name.
	const replacements: { start: number; end: number; text: string }[] = [];
	for (const head of parsed.heads) {
		if (
			context.raycast &&
			(head.value === "open" || head.value === "/usr/bin/open")
		) {
			replacements.push({ start: head.start, end: head.end, text: "xdg-open" });
			continue;
		}
		const decision = decide(head.value, [], context);
		if (decision.kind === "pass") continue;
		if (decision.kind === "rewrite" && decision.args.length === 0) {
			replacements.push({
				start: head.start,
				end: head.end,
				text: join([decision.file]),
			});
			continue;
		}
		const message =
			decision.kind === "refuse"
				? decision.message
				: `${context.title} runs ${head.value} inside a shell command line, which Compass cannot hand to ${decision.kind === "host" ? "the host" : "its own APIs"}: ${line}`;
		return {
			kind: "fake",
			job: refusedJob(message),
			file: "/bin/sh",
			args: ["-c", line],
		};
	}
	if (replacements.length === 0) return undefined;
	return {
		kind: "real",
		file: splice(line, replacements),
		args: [],
		shell: true,
	};
};

/** What to do with `file args`, run without a shell unless `options.shell`. */
export const planFile = (
	file: string,
	args: readonly string[] | undefined,
	options: any,
	context: ShimContext,
	jobs: Jobs,
): Plan | undefined => {
	const list = Array.isArray(args) ? args.map(String) : [];
	if (options?.shell)
		return planLine([file, ...list].join(" "), options, context, jobs);
	return fromDecision(decide(file, list, context), file, list, options, jobs);
};

/** Node's optional-argument shapes, normalised. */
const splitArgs = (rest: any[]): [string[] | undefined, any, any] => {
	let [args, options, callback] = rest;
	if (typeof args === "function") return [undefined, undefined, args];
	if (!Array.isArray(args)) {
		callback = options;
		options = args;
		args = undefined;
	}
	if (typeof options === "function") return [args, undefined, options];
	return [args, options, callback];
};

export const wrapChildProcess = (
	real: typeof import("node:child_process"),
	context: ShimContext,
	jobs: Jobs,
): typeof import("node:child_process") => {
	const withoutShell = (options: any) =>
		options && typeof options === "object"
			? { ...options, shell: false }
			: options;

	const spawn = (file: string, ...rest: any[]) => {
		const [args, options] = splitArgs(rest);
		const plan = planFile(file, args, options, context, jobs);
		if (!plan) return (real.spawn as any)(file, ...rest);
		if (plan.kind === "fake")
			return new FakeChild(plan.job, plan.file, plan.args);
		return plan.shell
			? real.spawn(plan.file, {
					...(options ?? {}),
					shell: options?.shell ?? true,
				})
			: real.spawn(plan.file, plan.args, withoutShell(options));
	};

	const spawnSync = (file: string, ...rest: any[]) => {
		const [args, options] = splitArgs(rest);
		const plan = planFile(file, args, options, context, jobs);
		if (!plan) return (real.spawnSync as any)(file, ...rest);
		if (plan.kind === "fake") return spawnSyncLike(plan.job, options);
		return plan.shell
			? real.spawnSync(plan.file, {
					...(options ?? {}),
					shell: options?.shell ?? true,
				})
			: real.spawnSync(plan.file, plan.args, withoutShell(options));
	};

	const exec = (command: string, ...rest: any[]) => {
		const options = typeof rest[0] === "function" ? undefined : rest[0];
		const callback = typeof rest[0] === "function" ? rest[0] : rest[1];
		const plan = planLine(command, options, context, jobs);
		if (!plan) return (real.exec as any)(command, ...rest);
		if (plan.kind === "fake")
			return execLike(
				plan.job,
				command,
				plan.file,
				plan.args,
				options,
				callback,
			);
		const line = plan.shell ? plan.file : join([plan.file, ...plan.args]);
		return (real.exec as any)(line, ...rest);
	};
	Object.defineProperty(exec, promisify.custom, {
		value: (command: string, options?: any) => {
			const plan = planLine(command, options, context, jobs);
			if (plan?.kind === "fake")
				return execPromise(plan.job, command, plan.file, plan.args, options);
			const line = !plan
				? command
				: plan.shell
					? plan.file
					: join([plan.file, ...plan.args]);
			return (promisify(real.exec) as any)(line, options);
		},
	});

	const execSync = (command: string, options?: any) => {
		const plan = planLine(command, options, context, jobs);
		if (!plan) return real.execSync(command, options);
		if (plan.kind === "fake") return execSyncLike(plan.job, command, options);
		return real.execSync(
			plan.shell ? plan.file : join([plan.file, ...plan.args]),
			options,
		);
	};

	const execFile = (file: string, ...rest: any[]) => {
		const [args, options, callback] = splitArgs(rest);
		const plan = planFile(file, args, options, context, jobs);
		if (!plan) return (real.execFile as any)(file, ...rest);
		if (plan.kind === "fake")
			return execLike(
				plan.job,
				[file, ...(args ?? [])].join(" "),
				plan.file,
				plan.args,
				options,
				callback,
			);
		if (plan.shell) return (real.exec as any)(plan.file, options, callback);
		return (real.execFile as any)(
			plan.file,
			plan.args,
			withoutShell(options),
			callback,
		);
	};
	Object.defineProperty(execFile, promisify.custom, {
		value: (file: string, ...rest: any[]) => {
			const [args, options] = splitArgs(rest);
			const plan = planFile(file, args, options, context, jobs);
			if (plan?.kind === "fake")
				return execPromise(
					plan.job,
					[file, ...(args ?? [])].join(" "),
					plan.file,
					plan.args,
					options,
				);
			if (!plan) return (promisify(real.execFile) as any)(file, ...rest);
			if (plan.shell) return (promisify(real.exec) as any)(plan.file, options);
			return (promisify(real.execFile) as any)(
				plan.file,
				plan.args,
				withoutShell(options),
			);
		},
	});

	const execFileSync = (file: string, ...rest: any[]) => {
		const [args, options] = splitArgs(rest);
		const plan = planFile(file, args, options, context, jobs);
		if (!plan) return (real.execFileSync as any)(file, ...rest);
		if (plan.kind === "fake")
			return execSyncLike(plan.job, [file, ...(args ?? [])].join(" "), options);
		if (plan.shell) return real.execSync(plan.file, options);
		return real.execFileSync(plan.file, plan.args, withoutShell(options));
	};

	const wrapped: Record<string, unknown> = {
		spawn,
		spawnSync,
		exec,
		execSync,
		execFile,
		execFileSync,
	};
	return new Proxy(real, {
		get: (target, prop, receiver) =>
			typeof prop === "string" && prop in wrapped
				? wrapped[prop]
				: Reflect.get(target, prop, receiver),
	});
};
