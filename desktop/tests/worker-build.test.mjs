import test from "node:test";
import assert from "node:assert/strict";
import { workerBuild, workerTarget } from "../scripts/worker-build.mjs";

test("native development preserves the host sidecar name and debug path", () => {
  const build = workerBuild("/repo", [], "aarch64-apple-darwin", {});
  assert.deepEqual(build.args, ["build", "--locked"]);
  assert.equal(build.source, "/repo/target/debug/macrun");
  assert.equal(
    build.destination,
    "/repo/desktop/src-tauri/binaries/macrun-aarch64-apple-darwin",
  );
});

test("cross target controls cargo, output lookup and sidecar name together", () => {
  const build = workerBuild(
    "/repo",
    ["--release", "--target=x86_64-apple-darwin"],
    "aarch64-apple-darwin",
    { CARGO_TARGET_DIR: "/cache" },
  );
  assert.deepEqual(build.args, [
    "build",
    "--locked",
    "--release",
    "--target",
    "x86_64-apple-darwin",
  ]);
  assert.equal(build.source, "/cache/x86_64-apple-darwin/release/macrun");
  assert.equal(
    build.destination,
    "/repo/desktop/src-tauri/binaries/macrun-x86_64-apple-darwin",
  );
});

test("explicit target wins over cargo environment; relative output roots resolve at repo", () => {
  const env = {
    CARGO_BUILD_TARGET: "x86_64-unknown-linux-gnu",
    CARGO_TARGET_DIR: ".local/out",
  };
  assert.equal(workerTarget([], env), env.CARGO_BUILD_TARGET);
  assert.equal(
    workerTarget(["--target", "aarch64-apple-darwin"], env),
    "aarch64-apple-darwin",
  );
  assert.equal(
    workerBuild("/repo", [], "host", env).source,
    "/repo/.local/out/x86_64-unknown-linux-gnu/debug/macrun",
  );
});

test("reject missing targets, universal sidecars and unsupported custom profiles", () => {
  for (const args of [
    ["--target"],
    ["--target="],
    ["--target", "--debug"],
    ["--target=universal-apple-darwin"],
    ["--profile=custom"],
    ["--profile", "custom"],
  ]) {
    assert.throws(() => workerTarget(args, {}));
  }
});
