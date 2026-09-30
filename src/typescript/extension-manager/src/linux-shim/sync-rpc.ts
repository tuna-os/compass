/**
 * A call to the engine that blocks, for `execSync("brew --prefix")`.
 *
 * Extension API calls are messages on the worker's port, answered on the
 * same port, so a call made from synchronous code cannot be answered while
 * that code runs: the event loop that would deliver the answer is the one it
 * is blocking. This sends the call and then takes messages off the port
 * itself (`receiveMessageOnPort`) until the answer is among them. Everything
 * else it takes is delivered, in order, once the caller has its answer, the
 * way a real `execSync` holds back every event until the child exits.
 */

export type SyncPort = {
	/** Sends a message to the engine, as the API client does. */
	post: (message: string) => void;
	/** Takes the next message the worker was sent, if there is one. */
	receive: () => string | undefined;
	/** Hands a message to the worker's own router, as its port would have. */
	deliver: (message: string) => void;
	/** Waits a little for more messages. */
	sleep: (ms: number) => void;
	now: () => number;
};

/** Well past anything the generated client numbers its own calls with. */
let nextId = 1_000_000_000;

const answerIn = (message: string, id: number) => {
	try {
		const outer = JSON.parse(message);
		const inner =
			typeof outer?.params?.msg === "string"
				? JSON.parse(outer.params.msg)
				: undefined;
		if (inner?.id === id && inner.method === undefined) return inner;
	} catch {}
	return undefined;
};

export const callSync = <T>(
	port: SyncPort,
	method: string,
	params: Record<string, unknown>,
	timeoutMs: number,
): T => {
	const id = nextId++;
	port.post(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
	const held: string[] = [];
	const deadline = port.now() + timeoutMs;
	try {
		for (;;) {
			const message = port.receive();
			if (message === undefined) {
				if (port.now() > deadline) {
					throw new Error(
						`${method} got no answer from Compass within ${Math.round(timeoutMs / 1000)} s`,
					);
				}
				port.sleep(5);
				continue;
			}
			held.push(message);
			const answer = answerIn(message, id);
			if (answer === undefined) continue;
			if (answer.error !== undefined) {
				throw new Error(
					typeof answer.error === "string"
						? answer.error
						: JSON.stringify(answer.error),
				);
			}
			return answer.result as T;
		}
	} finally {
		if (held.length > 0) {
			queueMicrotask(() => {
				for (const message of held) port.deliver(message);
			});
		}
	}
};

/** A port on the worker's own `parentPort`. */
export const workerPort = (deliver: (message: string) => void): SyncPort => {
	const { parentPort, receiveMessageOnPort } =
		require("node:worker_threads") as typeof import("node:worker_threads");
	const cell = new Int32Array(new SharedArrayBuffer(4));
	return {
		post: (message) => parentPort?.postMessage(message),
		receive: () =>
			parentPort ? receiveMessageOnPort(parentPort)?.message : undefined,
		deliver,
		sleep: (ms) => {
			Atomics.wait(cell, 0, 0, ms);
		},
		now: () => Date.now(),
	};
};
