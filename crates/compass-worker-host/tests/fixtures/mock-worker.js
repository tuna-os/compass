// A worker that speaks the real wire format, for testing the host.
//
// Not a mock of the host's own framing code: the four lines that matter are
// copied from src/typescript/extension-manager/src/index.ts, so if this and
// the host disagree, one of them disagrees with the thing being ported.
//
//   const packet = Buffer.allocUnsafe(message.length + 4);
//   packet.writeUint32BE(message.length, 0);
//   message.copy(packet, 4, 0);
//   process.stdout.write(packet);
//
// It plays one extension: on `Manager/ready` it asks the host to store a
// value and then to read it back, and writes whatever came back to the file
// named in argv[2]. The host under test never learns any of that -- it sees
// an ordinary session.

const fs = require("node:fs");

const OUT = process.argv[2];
if (!OUT) {
	console.error("usage: mock-worker.js <output file>");
	process.exit(64);
}

const SESSION = "s-1";

function write(message) {
	const body = Buffer.from(JSON.stringify(message));
	const packet = Buffer.allocUnsafe(body.length + 4);
	packet.writeUint32BE(body.length, 0);
	body.copy(packet, 4, 0);
	process.stdout.write(packet);
}

function reply(id, result) {
	write({ jsonrpc: "2.0", id, result });
}

function emit(method, params) {
	write({ jsonrpc: "2.0", method, params });
}

/** Sends a tsapi call to the host, wrapped in an extensionMessage event. */
function call(id, method, params) {
	emit("Manager/extensionMessage", {
		session_id: SESSION,
		payload: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
	});
}

// A watchdog, so a host that goes silent fails the test in fifteen seconds
// instead of hanging it: the host's read blocks until this pipe closes, and
// nothing else would ever close it.
setTimeout(() => {
	console.error("mock worker: the host said nothing for 15s; giving up");
	process.exit(70);
}, 15000);

let buffer = Buffer.from("");

process.stdin.on("data", (chunk) => {
	buffer = Buffer.concat([buffer, chunk]);

	while (buffer.length >= 4) {
		const length = buffer.readUInt32BE();
		if (buffer.length - 4 < length) return;
		const packet = buffer.subarray(4, length + 4);
		buffer = buffer.subarray(length + 4);
		handle(JSON.parse(packet.toString("utf8")));
	}
});

function handle(message) {
	switch (message.method) {
		case "Manager/load":
			reply(message.id, { session_id: SESSION });
			return;

		case "Manager/ready":
			reply(message.id, true);
			// The extension starts here.
			call(1, "Storage/set", { key: "greeting", value: "hello" });
			return;

		case "Manager/messageExtension": {
			reply(message.id, true);
			const inner = JSON.parse(message.params.payload);
			if (inner.id === 1) {
				call(2, "Storage/get", { key: "greeting" });
			} else if (inner.id === 2) {
				fs.writeFileSync(OUT, JSON.stringify(inner));
				process.exit(0);
			}
			return;
		}

		default:
			// The host may send things this worker does not play; an error
			// reply is better than silence, which would hang it.
			if (message.id !== undefined) {
				write({
					jsonrpc: "2.0",
					id: message.id,
					error: `mock worker does not implement ${message.method}`,
				});
			}
	}
}
