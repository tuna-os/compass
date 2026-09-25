import { spawnSync } from "child_process";

const notifyVicinae = () => {
	spawnSync("compass", ["compass://internal/restart-extension-runtime"]);
};

import "./build.mjs";

notifyVicinae();
