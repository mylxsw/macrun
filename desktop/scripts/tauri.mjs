import { spawnSync, execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("../../", import.meta.url));
execFileSync(
  process.execPath,
  [
    "scripts/prepare-worker.mjs",
    ...(process.argv[2] === "build" && !process.argv.includes("--debug")
      ? ["--release"]
      : []),
  ],
  { cwd: path.join(root, "desktop"), stdio: "inherit" },
);
const result = spawnSync(
  "/bin/sh",
  [
    "-c",
    'project_root="$1"; . "$project_root/scripts/rust-env.sh"; shift; exec "$@"',
    "sh",
    root,
    path.join(root, "desktop/node_modules/.bin/tauri"),
    ...process.argv.slice(2),
  ],
  { cwd: path.join(root, "desktop"), stdio: "inherit" },
);
process.exit(result.status ?? 1);
