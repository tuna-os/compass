This package lets you extend [Compass](https://tunaos.org/compass) and [Vicinae](https://docs.vicinae.com/) using React and TypeScript.
The name `@vicinae/api` is kept so that one extension runs on both.

[![Version](https://img.shields.io/npm/v/@vicinae/api.svg)](https://npmjs.org/package/@vicinae/api)
[![Downloads/week](https://img.shields.io/npm/dw/@vicinae/api.svg)](https://npmjs.org/package/@vicinae/api)

# Getting started

The recommended way to start developing a new extension is to read [Vicinae's extension docs](https://docs.vicinae.com/extensions/introduction). Compass runs the same API.

# Installation 

Install the package:

```
npm install @vicinae/api
```

# Versioning

The `@vicinae/api` package follows the same versioning as the launcher, since the API is always embedded in the launcher's extension runtime.

# CLI usage

The package exports the `vici` binary which is used to build and run extensions in development mode.

While convenience scripts are already provided in the boilerplate, you can still call the binary manually:

```bash
npx vici --help

# assuming the launcher is running
npx vici develop

npx vici build -o my/output/path
```
