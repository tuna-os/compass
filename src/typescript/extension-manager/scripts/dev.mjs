import { spawnSync } from "child_process";

const notifyCompass = () => {
	spawnSync("compass", ["compass://internal/restart-extension-runtime"]);
};

import "./build.mjs";

notifyCompass();
