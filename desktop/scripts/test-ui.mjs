import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
const tests = readdirSync("tests")
  .filter((name) => name.endsWith(".test.tsx"))
  .sort()
  .map((name) => `tests/${name}`);
const result = spawnSync(
  process.execPath,
  ["node_modules/vitest/vitest.mjs", "run", ...tests],
  { stdio: "inherit", env: { ...process.env, NODE_ENV: "test" } },
);
process.exit(result.status ?? 1);
