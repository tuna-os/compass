/**
 * A child process that is not one: the shape `child_process` returns, for a
 * program the runtime answers itself (`pbcopy` from its clipboard), the
 * engine runs on the host (`brew`), or the shim refuses by name.
 *
 * Only the parts extensions use: the streams, `exit`, `close` and `error`,
 * the callback and promise forms of `exec` and `execFile`, and the sync
 * forms' return values and errors.
 */

import { EventEmitter } from "node:events";
import { PassThrough, Writable } from "node:stream";

export type RunResult = {
	/** The exit status; `null` with `error`. */
	code: number | null;
	stdout: string;
	stderr: string;
	/** Set when it never ran: a refusal, or the engine's answer was an error. */
	error?: Error;
};

export type Job = {
	/** Whether it waits for its standard input to end (`pbcopy` does). */
	readsStdin: boolean;
	/** Set when it is refused outright: it never starts. */
	refused?: Error;
	run: (input: string) => Promise<RunResult>;
	runSync: (input: string) => RunResult;
};

/** The error a refused program fails with. */
export const refusal = (message: string): Error =>
	Object.assign(new Error(message), {
		name: "CompassRefusal",
		code: "ENOTSUP",
	});

/** A job that only ever refuses. */
export const refusedJob = (message: string): Job => {
	const error = refusal(message);
	const result = (): RunResult => ({
		code: null,
		stdout: "",
		stderr: "",
		error,
	});
	return {
		readsStdin: false,
		refused: error,
		run: async () => result(),
		runSync: result,
	};
};

type Encoding = BufferEncoding | "buffer" | null | undefined;

const encode = (text: string, encoding: Encoding): string | Buffer =>
	encoding === "buffer" || encoding === null || encoding === undefined
		? Buffer.from(text)
		: Buffer.from(text).toString(encoding);

const inputOf = (options: any): string => {
	const input = options?.input;
	if (input === undefined || input === null) return "";
	return typeof input === "string" ? input : Buffer.from(input).toString();
};

export class FakeChild extends EventEmitter {
	readonly stdin: Writable;
	readonly stdout = new PassThrough();
	readonly stderr = new PassThrough();
	readonly stdio: [Writable, PassThrough, PassThrough];
	pid: number | undefined = undefined;
	exitCode: number | null = null;
	signalCode: NodeJS.Signals | null = null;
	killed = false;
	connected = false;
	spawnfile: string;
	spawnargs: string[];

	constructor(
		job: Job,
		file: string,
		args: string[],
		onDone?: (result: RunResult) => void,
	) {
		super();
		this.spawnfile = file;
		this.spawnargs = [file, ...args];
		const chunks: Buffer[] = [];
		this.stdin = new Writable({
			write(chunk, _encoding, callback) {
				chunks.push(Buffer.from(chunk));
				callback();
			},
		});
		this.stdio = [this.stdin, this.stdout, this.stderr];

		const start = () =>
			job
				.run(Buffer.concat(chunks).toString())
				.catch(
					(error: Error): RunResult => ({
						code: null,
						stdout: "",
						stderr: "",
						error,
					}),
				)
				.then((result) => this.finish(result, onDone));

		if (job.refused) {
			const error = job.refused;
			process.nextTick(() =>
				this.finish({ code: null, stdout: "", stderr: "", error }, onDone),
			);
			return;
		}
		process.nextTick(() => this.emit("spawn"));
		if (job.readsStdin) this.stdin.on("finish", start);
		else process.nextTick(start);
	}

	private finish(result: RunResult, onDone?: (result: RunResult) => void) {
		if (result.error) {
			this.stdout.end();
			this.stderr.end();
			this.emit("error", result.error);
			this.emit("close", null, null);
			onDone?.(result);
			return;
		}
		this.exitCode = result.code;
		this.stdout.end(result.stdout);
		this.stderr.end(result.stderr);
		this.emit("exit", result.code, null);
		let open = 2;
		const closed = () => {
			if (--open === 0) this.emit("close", result.code, null);
		};
		this.stdout.on("end", closed);
		this.stderr.on("end", closed);
		onDone?.(result);
		// Nobody may read them, and draining is what lets `end` fire; a
		// reader that attaches on `exit` still gets everything first.
		setImmediate(() => {
			for (const stream of [this.stdout, this.stderr]) {
				if (stream.readableFlowing === null) stream.resume();
			}
		});
	}

	kill(): boolean {
		this.killed = true;
		return false;
	}

	ref() {}
	unref() {}
	disconnect() {}
}

/** The error `exec`'s callback and a sync form's throw carry. */
export const failure = (command: string, result: RunResult): Error => {
	if (result.error) return result.error;
	return Object.assign(
		new Error(
			`Command failed: ${command}${result.stderr ? `\n${result.stderr}` : ""}`,
		),
		{
			code: result.code,
			status: result.code,
			killed: false,
			signal: null,
			cmd: command,
			stdout: result.stdout,
			stderr: result.stderr,
		},
	);
};

/** `exec` and `execFile`: the callback, and the child they return. */
export const execLike = (
	job: Job,
	command: string,
	file: string,
	args: string[],
	options: any,
	callback?: (error: Error | null, stdout: any, stderr: any) => void,
): FakeChild => {
	const encoding: Encoding =
		options?.encoding === undefined ? "utf8" : options.encoding;
	const child = new FakeChild(job, file, args, (result) => {
		if (!callback) return;
		const stdout = encode(result.stdout, encoding);
		const stderr = encode(result.stderr, encoding);
		const failed = result.error !== undefined || result.code !== 0;
		const error = failed
			? Object.assign(failure(command, result), { stdout, stderr })
			: null;
		callback(error, stdout, stderr);
	});
	// As with the real `exec`, a failure to start reaches the callback, not
	// an `error` event nobody listens for.
	child.on("error", () => {});
	return child;
};

/** `util.promisify(exec)` and `util.promisify(execFile)`. */
export const execPromise = (
	job: Job,
	command: string,
	file: string,
	args: string[],
	options: any,
): Promise<{ stdout: any; stderr: any }> & { child: FakeChild } => {
	let child!: FakeChild;
	const promise = new Promise<{ stdout: any; stderr: any }>(
		(resolve, reject) => {
			child = execLike(
				job,
				command,
				file,
				args,
				options,
				(error, stdout, stderr) =>
					error ? reject(error) : resolve({ stdout, stderr }),
			);
		},
	);
	return Object.assign(promise, { child });
};

/** `execSync` and `execFileSync`. */
export const execSyncLike = (job: Job, command: string, options: any) => {
	const result = job.runSync(inputOf(options));
	if (result.error || result.code !== 0) {
		const encoding: Encoding = options?.encoding ?? "buffer";
		throw Object.assign(failure(command, result), {
			stdout: encode(result.stdout, encoding),
			stderr: encode(result.stderr, encoding),
			output: [
				null,
				encode(result.stdout, encoding),
				encode(result.stderr, encoding),
			],
		});
	}
	return encode(result.stdout, options?.encoding ?? "buffer");
};

/** `spawnSync`. */
export const spawnSyncLike = (job: Job, options: any) => {
	const result = job.runSync(inputOf(options));
	const encoding: Encoding = options?.encoding ?? "buffer";
	const stdout = encode(result.stdout, encoding);
	const stderr = encode(result.stderr, encoding);
	return {
		pid: 0,
		output: [null, stdout, stderr],
		stdout,
		stderr,
		status: result.error ? null : result.code,
		signal: null,
		...(result.error ? { error: result.error } : {}),
	};
};
