import { execFileSync } from "node:child_process";
import { mkdirSync, copyFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL("../../", import.meta.url));
const release = process.argv.includes("--release");
execFileSync(
  path.join(root, "scripts/cargo-local.sh"),
  ["build", "--locked", ...(release ? ["--release"] : [])],
  { cwd: root, stdio: "inherit" },
);
const target = execFileSync(
  path.join(root, "scripts/rustc-local.sh"),
  ["-vV"],
  { cwd: root, encoding: "utf8" },
).match(/^host: (.+)$/m)[1];
mkdirSync(path.join(root, "desktop/src-tauri/binaries"), { recursive: true });
copyFileSync(
  path.join(root, `target/${release ? "release" : "debug"}/macrun`),
  path.join(root, `desktop/src-tauri/binaries/macrun-${target}`),
);
