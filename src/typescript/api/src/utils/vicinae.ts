import * as net from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { Client, type PingResponse, RpcTransport } from "../proto/ipc.js";

export type VicinaeClientOptions = {
	socketPath?: string;
	timeoutMs?: number;
};

// The engine's socket: `$XDG_RUNTIME_DIR/compass/ipc.sock`, or
// `/tmp/compass-$USER/ipc.sock` without a runtime directory, as
// crates/compass-ipc/src/path.rs resolves it. `COMPASS_SOCKET` overrides it,
// as it does for the `compass` CLI.
const runtimeDir = (): string => {
	const runtime =
		process.platform === "darwin"
			? process.env.TMPDIR
			: process.env.XDG_RUNTIME_DIR;
	if (runtime) return path.join(runtime, "compass");
	return path.join("/tmp", `compass-${os.userInfo().username}`);
};

export const serverSocketPath = (): string => {
	if (process.env.COMPASS_SOCKET) return process.env.COMPASS_SOCKET;
	if (process.platform === "win32")
		return `\\\\.\\pipe\\compass-${os.userInfo().username}`;
	return path.join(runtimeDir(), "ipc.sock");
};

export class VicinaeClient {
	constructor(private readonly options: VicinaeClientOptions = {}) {}

	ping(): Promise<PingResponse> {
		return this.withConnection((client) => client.Ipc.ping());
	}

	refreshDevSession(extensionId: string): Promise<void> {
		return this.deeplink(
			`compass://api/extensions/develop/refresh?id=${extensionId}`,
		);
	}

	startDevSession(extensionId: string): Promise<void> {
		return this.deeplink(
			`compass://api/extensions/develop/start?id=${extensionId}`,
		);
	}

	stopDevSession(extensionId: string): Promise<void> {
		return this.deeplink(
			`compass://api/extensions/develop/stop?id=${extensionId}`,
		);
	}

	private async deeplink(url: string): Promise<void> {
		const response = await this.withConnection((client) =>
			client.Ipc.deeplink({ url }),
		);

		if (response.error) throw new Error(response.error);
	}

	private withConnection<T>(run: (client: Client) => Promise<T>): Promise<T> {
		const socketPath = this.options.socketPath ?? serverSocketPath();
		const timeoutMs = this.options.timeoutMs ?? 5000;

		return new Promise<T>((resolve, reject) => {
			const socket = net.createConnection({ path: socketPath });
			let data = Buffer.alloc(0);
			let settled = false;

			const fail = (error: Error) => {
				if (settled) return;
				settled = true;
				socket.destroy();
				reject(error);
			};

			const succeed = (value: T) => {
				if (settled) return;
				settled = true;
				socket.end();
				resolve(value);
			};

			const client = new Client(
				new RpcTransport({
					send: (payload: string) => {
						const body = Buffer.from(payload);
						const frame = Buffer.alloc(4 + body.length);

						frame.writeUInt32LE(body.length, 0);
						body.copy(frame, 4);
						socket.write(frame);
					},
				}),
			);

			socket.setTimeout(timeoutMs, () => {
				fail(new Error("Timed out waiting for a response from Vicinae"));
			});

			socket.on("error", (error: NodeJS.ErrnoException) => {
				if (error.code === "ENOENT" || error.code === "ECONNREFUSED") {
					fail(
						new Error(
							`Could not connect to Vicinae at ${socketPath}. Is Vicinae running?`,
						),
					);
					return;
				}
				fail(error);
			});

			socket.on("close", () => {
				fail(new Error("Connection closed before a response was received"));
			});

			socket.on("connect", () => {
				run(client).then(succeed, (error: unknown) => {
					fail(error instanceof Error ? error : new Error(String(error)));
				});
			});

			socket.on("data", (chunk) => {
				data = Buffer.concat([data, chunk]);

				while (data.length >= 4) {
					const size = data.readUInt32LE(0);
					if (data.length < 4 + size) break;

					try {
						client.route(data.subarray(4, 4 + size).toString());
					} catch (error) {
						fail(new Error(`Received a malformed response: ${error}`));
						return;
					}

					data = data.subarray(4 + size);
				}
			});
		});
	}
}
