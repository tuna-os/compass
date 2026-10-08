import { execFile } from "node:child_process";

export type VicinaeClientOptions = {
	/** The engine's socket; `COMPASS_SOCKET` and the CLI's default otherwise. */
	socketPath?: string;
	timeoutMs?: number;
};

type Invocation = { file: string; args: string[] };

// How to run the `compass` CLI: `COMPASS_BIN` when set, then `compass` on
// PATH, then the Flatpak. The engine's wire format is postcard-encoded and
// versioned with the engine, so the SDK does not speak it itself: the CLI is
// always the engine's own version and says which one it speaks.
const invocations = (args: string[]): Invocation[] => {
	if (process.env.COMPASS_BIN) return [{ file: process.env.COMPASS_BIN, args }];
	return [
		{ file: "compass", args },
		{ file: "flatpak", args: ["run", "org.tunaos.compass", ...args] },
	];
};

const run = (
	{ file, args }: Invocation,
	timeoutMs: number,
): Promise<{ stdout: string; stderr: string }> =>
	new Promise((resolve, reject) => {
		execFile(file, args, { timeout: timeoutMs }, (error, stdout, stderr) => {
			if (error) {
				const failure = Object.assign(error, { stdout, stderr });
				reject(failure);
				return;
			}
			resolve({ stdout, stderr });
		});
	});

export class VicinaeClient {
	constructor(private readonly options: VicinaeClientOptions = {}) {}

	/** Whether a Compass engine is answering. */
	async ping(): Promise<void> {
		await this.compass(["ping"]);
	}

	refreshDevSession(extensionId: string): Promise<void> {
		return this.deeplink(
			`compass://api/extensions/develop/refresh?id=${encodeURIComponent(extensionId)}`,
		);
	}

	startDevSession(extensionId: string): Promise<void> {
		return this.deeplink(
			`compass://api/extensions/develop/start?id=${encodeURIComponent(extensionId)}`,
		);
	}

	stopDevSession(extensionId: string): Promise<void> {
		return this.deeplink(
			`compass://api/extensions/develop/stop?id=${encodeURIComponent(extensionId)}`,
		);
	}

	private async deeplink(url: string): Promise<void> {
		await this.compass(["deeplink", url]);
	}

	private async compass(args: string[]): Promise<string> {
		const timeoutMs = this.options.timeoutMs ?? 5000;
		const socket = this.options.socketPath ?? process.env.COMPASS_SOCKET;
		const full = socket ? ["--socket", socket, ...args] : args;
		let missing: Error | undefined;
		for (const invocation of invocations(full)) {
			try {
				return (await run(invocation, timeoutMs)).stdout;
			} catch (error) {
				const failure = error as NodeJS.ErrnoException & { stderr?: string };
				if (failure.code === "ENOENT") {
					missing = failure;
					continue;
				}
				const reason = failure.stderr?.trim() || failure.message;
				throw new Error(reason);
			}
		}
		throw new Error(
			`Could not find the compass command (${missing?.message ?? "not found"}). Install Compass, or set COMPASS_BIN to its path.`,
		);
	}
}
