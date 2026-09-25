/**
 * `fs` as a Raycast extension sees it: the real module, with the functions
 * that look a path up or read it seeing the Linux path for a macOS one
 * ({@link mapPath}): `existsSync("/opt/homebrew/bin/brew")` asks about the
 * Linuxbrew prefix, and a path the overrides manifest maps is read from where
 * Linux keeps it.
 *
 * Only lookups and reads. Writes keep the path they were given: an extension
 * that writes where it thinks macOS keeps something should not end up
 * writing somewhere else.
 */

import { mapPath, type ShimContext } from "./rules";

/** Take a path first and only look or read. */
const LOOKUPS = [
	"existsSync",
	"statSync",
	"lstatSync",
	"accessSync",
	"realpathSync",
	"readFileSync",
	"readdirSync",
	"opendirSync",
	"stat",
	"lstat",
	"access",
	"exists",
	"realpath",
	"readFile",
	"readdir",
	"opendir",
	"createReadStream",
] as const;

const PROMISE_LOOKUPS = [
	"stat",
	"lstat",
	"access",
	"realpath",
	"readFile",
	"readdir",
	"opendir",
] as const;

const mapFirst =
	(fn: (...args: any[]) => any, context: ShimContext) =>
	(first: unknown, ...rest: unknown[]) =>
		fn(
			typeof first === "string" ? (mapPath(first, context) ?? first) : first,
			...rest,
		);

const overlay = <T extends object>(
	real: T,
	names: readonly string[],
	context: ShimContext,
	extra: Record<string, unknown> = {},
): T => {
	const wrapped: Record<string, unknown> = { ...extra };
	for (const name of names) {
		const fn = (real as Record<string, unknown>)[name];
		if (typeof fn !== "function") continue;
		const mapped = mapFirst(fn.bind(real) as (...args: any[]) => any, context);
		// `fs.exists` and `fs.realpath` carry their own promisified forms.
		for (const key of Object.getOwnPropertySymbols(fn)) {
			(mapped as any)[key] = (fn as any)[key];
		}
		if (typeof (fn as any).native === "function") {
			(mapped as any).native = mapFirst((fn as any).native, context);
		}
		wrapped[name] = mapped;
	}
	return new Proxy(real, {
		get: (target, prop, receiver) =>
			typeof prop === "string" && prop in wrapped
				? wrapped[prop]
				: Reflect.get(target, prop, receiver),
	});
};

export const wrapFsPromises = (
	real: typeof import("node:fs/promises"),
	context: ShimContext,
): typeof import("node:fs/promises") => overlay(real, PROMISE_LOOKUPS, context);

export const wrapFs = (
	real: typeof import("node:fs"),
	context: ShimContext,
): typeof import("node:fs") =>
	overlay(real, LOOKUPS, context, {
		promises: wrapFsPromises(real.promises, context),
	});
