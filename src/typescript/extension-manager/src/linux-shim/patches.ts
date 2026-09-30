/**
 * The overrides manifest's source patches, applied as the extension's own
 * files load. The installed files stay as the store served them: an update
 * replaces them and the patch applies again, or, if the text it looks for is
 * gone, is skipped and says so.
 */

import * as path from "node:path";
import { fileURLToPath } from "node:url";
import type { Patch } from "./overrides";

export type Applied = { file: string; count: number };

/** `source` with `patches` for `file` applied, and how many matched. */
export const applyPatches = (
	source: string,
	file: string,
	patches: readonly Patch[],
	log: (message: string) => void = () => {},
): string => {
	let out = source;
	for (const patch of patches) {
		if (patch.file !== file) continue;
		const pieces = out.split(patch.find);
		if (pieces.length === 1) {
			log(
				`override patch for ${file} not applied: its text is not there (${patch.why})`,
			);
			continue;
		}
		out = pieces.join(patch.replace);
	}
	return out;
};

/**
 * Applies `patches` to files under `extensionDir` as Node loads them, through
 * `module.registerHooks`. Returns false when this Node has no such hook.
 */
export const installPatches = (
	extensionDir: string,
	patches: readonly Patch[],
	log: (message: string) => void,
): boolean => {
	if (patches.length === 0) return true;
	const Module = require("node:module") as {
		registerHooks?: (hooks: {
			load: (
				url: string,
				context: unknown,
				next: (url: string, context: unknown) => { source?: unknown },
			) => { source?: unknown };
		}) => unknown;
	};
	if (typeof Module.registerHooks !== "function") {
		log(
			"this Node has no module.registerHooks; the extension's override patches are not applied",
		);
		return false;
	}
	const root = path.resolve(extensionDir);
	Module.registerHooks({
		load: (url, context, next) => {
			const result = next(url, context);
			if (!url.startsWith("file:") || result.source === undefined)
				return result;
			const file = fileURLToPath(url);
			if (!file.startsWith(`${root}${path.sep}`)) return result;
			const relative = path.relative(root, file).split(path.sep).join("/");
			if (!patches.some((patch) => patch.file === relative)) return result;
			return {
				...result,
				source: applyPatches(String(result.source), relative, patches, log),
			};
		},
	});
	return true;
};
