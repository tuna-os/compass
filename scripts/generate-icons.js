// Codegen for icons
// Generates the typescript enum consumed by extensions from the builtin icon set.
// compass-core's build.rs reads the same enum, so this is the one list of names.

const path = require('path');
const fs = require('fs');
const ICON_DIR = path.join(__dirname, "..", "extra", "builtin-icons");

const toEnumType = (str) => {
	return str
		.toLowerCase()
		.replace(/[^a-zA-Z0-9]+(.)/g, (match, char) => char.toUpperCase())
		.replace(/^./, char => char.toUpperCase());
}

const generateSources = (files) => {
	const serializedFileNames = files.map(file => `"${file.split('.')[0]}"`);
	const enumNames = files.map(file => toEnumType(`${file.split('.')[0]}`));

	const tsEnum = `
export enum Icon {
${enumNames.map((name, i) => `${name} = ${serializedFileNames[i]}`)
			.join(',\n\t')}
}
`;

	return { ts: { iconEnum: tsEnum } };
};


const writeFile = (path, data) => {
	fs.writeFileSync(path, data);
	console.log(`Wrote file at ${path}`);
}

const icons = fs.readdirSync(ICON_DIR).filter((file) => file.endsWith('.svg'));
const { ts } = generateSources(icons);
const apiIconSource = path.join(__dirname, "..", "src", "typescript", "api", "src", "api", "icon.ts");

writeFile(apiIconSource, ts.iconEnum);
