// Existing fixtures describe the Chinese interface, independent of the host OS.
if (globalThis.navigator)
  Object.defineProperty(navigator, "language", {
    configurable: true,
    value: "zh-CN",
  });
