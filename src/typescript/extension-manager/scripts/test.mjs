// Runs the runtime's unit tests: bundles each `test/*.test.ts` with the
// esbuild the runtime is already built with, and hands the bundles to Node's
// own test runner. No test framework to install, and nothing the offline
// builds (Nix, Flatpak) would have to fetch.
import * as esbuild from "esbuild";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");
const testDir = join(root, "test");
const entryPoints = readdirSync(testDir)
	.filter((name) => name.endsWith(".test.ts"))
	.map((name) => join(testDir, name));

const outdir = mkdtempSync(join(tmpdir(), "extension-manager-tests-"));
let status = 1;
try {
	await esbuild.build({
		entryPoints,
		bundle: true,
		outdir,
		format: "cjs",
		platform: "node",
		logLevel: "warning",
	});
	const files = readdirSync(outdir)
		.filter((name) => name.endsWith(".js"))
		.map((name) => join(outdir, name));
	status =
		spawnSync(process.execPath, ["--test", ...files], { stdio: "inherit" })
			.status ?? 1;
} finally {
	rmSync(outdir, { recursive: true, force: true });
}
process.exit(status);
