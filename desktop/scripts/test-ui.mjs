import { spawnSync } from "node:child_process";
const result = spawnSync(
  process.execPath,
  ["node_modules/vitest/vitest.mjs", "run", "tests/components.test.tsx"],
  { stdio: "inherit", env: { ...process.env, NODE_ENV: "test" } },
);
process.exit(result.status ?? 1);
