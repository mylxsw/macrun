import { execFileSync } from "node:child_process";
import { mkdirSync, copyFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { workerBuild } from "./worker-build.mjs";
const root = fileURLToPath(new URL("../../", import.meta.url));
const host = execFileSync(path.join(root, "scripts/rustc-local.sh"), ["-vV"], {
  cwd: root,
  encoding: "utf8",
}).match(/^host: (.+)$/m)[1];
const build = workerBuild(root, process.argv.slice(2), host);
execFileSync(path.join(root, "scripts/cargo-local.sh"), build.args, {
  cwd: root,
  stdio: "inherit",
});
mkdirSync(path.join(root, "desktop/src-tauri/binaries"), { recursive: true });
copyFileSync(build.source, build.destination);
