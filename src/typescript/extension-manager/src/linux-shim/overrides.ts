/**
 * `extensions/raycast-linux-overrides.json`, the curated per-extension
 * overrides; `docs/rust-engine/RAYCAST-LINUX-SHIM.md` documents it. The
 * engine reads the same file (`compass_core::raycast_overrides`) for the
 * host programs and redirects, which only it may act on.
 */

import manifest from "../../../../../extensions/raycast-linux-overrides.json";

export type Patch = {
	file: string;
	find: string;
	replace: string;
	why: string;
};

export type Entry = {
	why: string;
	hostPrograms?: string[];
	paths?: Record<string, string>;
	commands?: Record<string, string>;
	patches?: Patch[];
	redirect?: {
		store: string;
		author: string;
		name: string;
		why: string;
	} | null;
};

export type Manifest = {
	version: number;
	hostPrograms?: string[];
	extensions?: Record<string, Entry>;
};

export const SHIPPED: Manifest = manifest as Manifest;

/** The id the engine installs a store extension under. */
export const extensionId = (name: string, raycast: boolean) =>
	raycast ? `store.raycast.${name}` : name;

export const entryFor = (
	id: string,
	from: Manifest = SHIPPED,
): Entry | undefined => from.extensions?.[id];

/** Every program `id` may ask the engine to run on the host. */
export const hostProgramsFor = (id: string, from: Manifest = SHIPPED) =>
	new Set([
		...(from.hostPrograms ?? []),
		...(entryFor(id, from)?.hostPrograms ?? []),
	]);

/** The entry's path maps, `~` expanded, longest first. */
export const pathsFor = (
	entry: Entry | undefined,
	home: string,
): [string, string][] =>
	Object.entries(entry?.paths ?? {})
		.map(([from, to]): [string, string] => [
			from.replace(/^~(?=\/|$)/, home),
			to.replace(/^~(?=\/|$)/, home),
		])
		.sort((a, b) => b[0].length - a[0].length);
