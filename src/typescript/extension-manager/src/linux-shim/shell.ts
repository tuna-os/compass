/**
 * Just enough of a POSIX shell's word splitting to tell a simple command
 * (`"/opt/homebrew/bin/brew" info --json=v2 --installed`) from one that uses
 * the shell (a pipe, a redirection, an expansion), and to find where each
 * command in a line starts.
 *
 * A simple command is taken apart into its words and can run without a
 * shell, or on the host. Anything else is left to the shell, with only its
 * command words ever touched, in place.
 */

export type Word = {
	/** The word as the shell would pass it. */
	value: string;
	/** Where its raw text starts and ends in the line. */
	start: number;
	end: number;
	/** Whether any of it was quoted or escaped. */
	quoted: boolean;
};

export type Parsed =
	| { simple: true; words: Word[] }
	| { simple: false; heads: Word[] };

const OPERATOR = new Set(["|", "&", ";", "<", ">", "(", ")", "\n"]);
const EXPANSION = new Set(["$", "`", "*", "?", "[", "{", "}"]);

export const parse = (line: string): Parsed => {
	const words: Word[] = [];
	const heads: Word[] = [];
	let simple = true;
	let atHead = true;
	let i = 0;

	while (i < line.length) {
		const c = line[i];
		if (c === " " || c === "\t") {
			i++;
			continue;
		}
		if (OPERATOR.has(c)) {
			simple = false;
			atHead = c === "|" || c === "&" || c === ";" || c === "(" || c === "\n";
			i++;
			continue;
		}
		if (c === "#") {
			simple = false;
			break;
		}

		const start = i;
		let value = "";
		let quoted = false;
		while (i < line.length) {
			const ch = line[i];
			if (ch === " " || ch === "\t" || OPERATOR.has(ch)) break;
			if (ch === "'") {
				const close = line.indexOf("'", i + 1);
				if (close < 0) return { simple: false, heads };
				value += line.slice(i + 1, close);
				quoted = true;
				i = close + 1;
				continue;
			}
			if (ch === '"') {
				quoted = true;
				i++;
				while (i < line.length && line[i] !== '"') {
					if (line[i] === "\\" && i + 1 < line.length) {
						const next = line[i + 1];
						if ('"\\$`'.includes(next)) {
							value += next;
							i += 2;
							continue;
						}
						if (next === "\n") {
							i += 2;
							continue;
						}
					}
					if (line[i] === "$" || line[i] === "`") simple = false;
					value += line[i];
					i++;
				}
				if (i >= line.length) return { simple: false, heads };
				i++;
				continue;
			}
			if (ch === "\\") {
				if (i + 1 < line.length) value += line[i + 1];
				quoted = true;
				i += 2;
				continue;
			}
			if (EXPANSION.has(ch)) simple = false;
			if (ch === "~" && i === start) simple = false;
			if (ch === "=" && atHead && !quoted) simple = false;
			value += ch;
			i++;
		}
		const word = { value, start, end: i, quoted };
		words.push(word);
		if (atHead) heads.push(word);
		atHead = false;
	}

	return simple && words.length > 0
		? { simple: true, words }
		: { simple: false, heads };
};

/** One word, quoted so a shell reads it back as exactly `value`. */
export const quote = (value: string): string =>
	/^[A-Za-z0-9_@%+=:,./-]+$/.test(value)
		? value
		: `'${value.split("'").join("'\\''")}'`;

/** A command line a shell reads back as exactly `words`. */
export const join = (words: string[]): string => words.map(quote).join(" ");

/** `line` with each of `replacements`' spans replaced, right to left. */
export const splice = (
	line: string,
	replacements: { start: number; end: number; text: string }[],
): string =>
	[...replacements]
		.sort((a, b) => b.start - a.start)
		.reduce(
			(out, { start, end, text }) =>
				out.slice(0, start) + text + out.slice(end),
			line,
		);
