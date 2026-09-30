/**
 * What the shim does with one program an extension runs: nothing, run
 * another one in its place, send it to the engine to run on the host, answer
 * it from the runtime's own APIs, or refuse it by name.
 *
 * Pure: everything it needs to know about the machine comes in through
 * {@link ShimContext}, so the rules are tested without one.
 */

import * as path from "node:path";

/** Homebrew's prefix on Linux, where Bluefin and the installer put it. */
export const LINUXBREW_PREFIX = "/home/linuxbrew/.linuxbrew";

/** Where macOS extensions expect Homebrew: Apple silicon, then Intel. */
export const MACOS_HOMEBREW_PREFIXES = ["/opt/homebrew", "/usr/local"];

/** Directories under `/usr/local` that only Homebrew puts there. */
const USR_LOCAL_HOMEBREW = ["Homebrew", "Cellar", "Caskroom"];

/**
 * macOS programs with no Linux counterpart. Refused by name when they are
 * not on `PATH`, rather than failing with `ENOENT`.
 */
export const MACOS_ONLY = new Set([
	"osascript",
	"defaults",
	"mdfind",
	"mdls",
	"sw_vers",
	"system_profiler",
	"screencapture",
	"afplay",
	"say",
	"shortcuts",
	"plutil",
	"launchctl",
	"networksetup",
	"pmset",
	"lsappinfo",
	"sips",
	"textutil",
	"scutil",
	"diskutil",
	"ioreg",
]);

export type Decision =
	| { kind: "pass" }
	| { kind: "rewrite"; file: string; args: string[] }
	| { kind: "host"; program: string; args: string[] }
	| { kind: "clipboard-copy" }
	| { kind: "clipboard-paste" }
	| { kind: "refuse"; message: string };

export type ShimContext = {
	/** The extension's title, for messages. */
	title: string;
	/**
	 * Whether it was written for Raycast, and so for macOS. The macOS
	 * rewrites apply only then; brokering applies to every extension.
	 */
	raycast: boolean;
	/** Programs the engine runs on the host for this extension. */
	hostPrograms: ReadonlySet<string>;
	/** Programs run in place of others, from the overrides manifest. */
	commands: Readonly<Record<string, string>>;
	/** Path prefixes mapped macOS to Linux, longest first, `~` expanded. */
	paths: ReadonlyArray<readonly [string, string]>;
	/** The Linuxbrew prefix, when there is one. */
	homebrewPrefix?: string;
	/**
	 * Homebrew prefixes learnt from what `brew --prefix` answered on the
	 * host, which inside the Flatpak is the only way to know one.
	 */
	learntPrefixes: Set<string>;
	/** Whether a program of this name is on `PATH`. */
	onPath: (name: string) => boolean;
};

const under = (file: string, dir: string) =>
	file === dir || file.startsWith(`${dir}/`);

/** Whether `file` is a program inside a Homebrew prefix's `bin` or `sbin`. */
const homebrewProgram = (file: string, context: ShimContext) => {
	const dir = path.posix.dirname(file);
	const prefixes = [
		"/opt/homebrew",
		LINUXBREW_PREFIX,
		...(context.homebrewPrefix ? [context.homebrewPrefix] : []),
		...context.learntPrefixes,
	];
	if (
		prefixes.some(
			(prefix) => dir === `${prefix}/bin` || dir === `${prefix}/sbin`,
		)
	)
		return true;
	// `/usr/local/bin` is also where Linux users install things by hand, so
	// only a program the engine brokers is taken from it.
	return (
		dir === "/usr/local/bin" &&
		context.hostPrograms.has(path.posix.basename(file))
	);
};

/**
 * `file` with a macOS path replaced by its Linux counterpart: the manifest's
 * maps first, then Homebrew's prefixes to Linuxbrew's. `undefined` when
 * nothing applies.
 */
export const mapPath = (
	file: string,
	context: ShimContext,
): string | undefined => {
	if (!context.raycast || typeof file !== "string") return undefined;
	for (const [from, to] of context.paths) {
		if (under(file, from)) return to + file.slice(from.length);
	}
	const prefix = context.homebrewPrefix;
	if (!prefix) return undefined;
	if (under(file, "/opt/homebrew"))
		return prefix + file.slice("/opt/homebrew".length);
	for (const dir of USR_LOCAL_HOMEBREW) {
		if (under(file, `/usr/local/${dir}`))
			return prefix + file.slice("/usr/local".length);
	}
	return undefined;
};

const OPEN_FLAGS_WITHOUT_VALUE = new Set([
	"-g",
	"-n",
	"-F",
	"-W",
	"-j",
	"-h",
	"-e",
	"-t",
	"-f",
	"--fresh",
	"--new",
	"--wait-apps",
	"--background",
	"--hide",
]);

/**
 * macOS `open` as `xdg-open`: the targets opened with their default
 * application. Naming an application (`-a`, `-b`) chooses nothing on Linux,
 * so the target still opens with its default; naming one with no target is
 * refused.
 */
export const translateOpen = (args: string[], title: string): Decision => {
	const targets: string[] = [];
	let app: string | undefined;
	let reveal = false;
	for (let i = 0; i < args.length; i++) {
		const arg = args[i];
		if (arg === "--args") break;
		if (arg === "-a" || arg === "-b" || arg === "-u") {
			if (arg !== "-u") app = args[i + 1];
			else targets.push(args[i + 1]);
			i++;
			continue;
		}
		if (arg === "-R" || arg === "--reveal") {
			reveal = true;
			continue;
		}
		if (OPEN_FLAGS_WITHOUT_VALUE.has(arg)) continue;
		targets.push(arg);
	}
	const target = targets.find((t) => t !== undefined);
	if (target === undefined) {
		return {
			kind: "refuse",
			message: app
				? `${title} asked macOS to open the application "${app}" by name (open -a), which has no Linux equivalent`
				: `${title} ran macOS's open with nothing to open`,
		};
	}
	return {
		kind: "rewrite",
		file: "xdg-open",
		args: [reveal ? path.posix.dirname(target) : target],
	};
};

/** What to do with `file args`. */
export const decide = (
	file: string,
	args: string[],
	context: ShimContext,
): Decision => {
	if (typeof file !== "string" || file.length === 0) return { kind: "pass" };
	const bare = !file.includes("/");
	const name = path.posix.basename(file);

	if (
		context.hostPrograms.has(name) &&
		(bare || homebrewProgram(file, context))
	)
		return { kind: "host", program: name, args };

	if (!context.raycast) return { kind: "pass" };

	const mappedArgs = args.map((arg) => mapPath(arg, context) ?? arg);
	const changed = mappedArgs.some((arg, i) => arg !== args[i]);

	const command = context.commands[name];
	if (command !== undefined && (bare || homebrewProgram(file, context)))
		return { kind: "rewrite", file: command, args: mappedArgs };

	if (!bare && homebrewProgram(file, context) && !under(file, "/usr/local")) {
		return {
			kind: "refuse",
			message: `${context.title} runs ${name} from Homebrew, which extensions cannot run on Linux: Compass runs only ${[...context.hostPrograms].join(", ")} on the host for them`,
		};
	}

	if (name === "open" && (bare || file === "/usr/bin/open"))
		return translateOpen(mappedArgs, context.title);
	if (name === "pbcopy" && (bare || file === "/usr/bin/pbcopy"))
		return { kind: "clipboard-copy" };
	if (name === "pbpaste" && (bare || file === "/usr/bin/pbpaste"))
		return { kind: "clipboard-paste" };
	if (name === "osascript" && (bare || file === "/usr/bin/osascript")) {
		return {
			kind: "refuse",
			message: `${context.title} needs macOS for this: it runs AppleScript (osascript), which does not exist on Linux`,
		};
	}
	if (MACOS_ONLY.has(name) && !context.onPath(name)) {
		return {
			kind: "refuse",
			message: `${context.title} needs macOS for this: it runs ${name}, a macOS program Linux does not have`,
		};
	}

	const mappedFile = mapPath(file, context);
	if (mappedFile !== undefined || changed)
		return { kind: "rewrite", file: mappedFile ?? file, args: mappedArgs };
	return { kind: "pass" };
};
