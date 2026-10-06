import path from "node:path";

// Both entry points use this parser so the app and its sidecar share a target.
export function workerTarget(args, env = process.env) {
  const index = args.indexOf("--target");
  const inline = args.find((arg) => arg.startsWith("--target="));
  const target =
    index >= 0
      ? args[index + 1]
      : inline !== undefined
        ? inline.slice(9)
        : env.CARGO_BUILD_TARGET;
  if (
    (index >= 0 || args.includes("--target=")) &&
    (!target || target.startsWith("-"))
  ) {
    throw new Error("--target requires a Rust target triple");
  }
  if (
    target &&
    !/^(aarch64|x86_64)-(apple-darwin|unknown-linux-gnu)$/.test(target)
  ) {
    throw new Error(
      `Unsupported worker target: ${target}; build separate native packages`,
    );
  }
  if (args.some((arg) => arg === "--profile" || arg.startsWith("--profile="))) {
    throw new Error(
      "Custom profiles are not supported; use release or --debug",
    );
  }
  return target;
}

export function workerBuild(root, args, host, env = process.env) {
  const target = workerTarget(args, env);
  const release = args.includes("--release");
  return {
    args: [
      "build",
      "--locked",
      ...(release ? ["--release"] : []),
      ...(target ? ["--target", target] : []),
    ],
    source: path.resolve(
      root,
      env.CARGO_TARGET_DIR || "target",
      ...(target ? [target] : []),
      release ? "release" : "debug",
      "macrun",
    ),
    destination: path.join(
      root,
      "desktop/src-tauri/binaries",
      `macrun-${target || host}`,
    ),
  };
}
