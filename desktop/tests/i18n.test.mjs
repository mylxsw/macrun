import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import ts from "typescript";
import english from "../src/locales/en.json" with { type: "json" };
import {
  getLanguage,
  getLocale,
  resolveLocale,
  setLanguage,
  subscribeLanguage,
  tr,
  refreshSystemLanguage,
} from "../src/i18n.mjs";
import {
  statuses,
  tierLabels,
  tierPolicyLabels,
  presets,
  title,
  explainTask,
} from "../src/model.mjs";

test("language resolution covers both supported languages and system fallback", () => {
  for (const name of ["zh", "zh-CN", "zh-Hans", "zh-TW", "ZH_hant"])
    assert.equal(resolveLocale(name), "zh-CN");
  for (const name of ["en", "en-GB", "de-DE", "ja", "", undefined])
    assert.equal(resolveLocale(name), "en");
  setLanguage("system", "zh-TW");
  assert.equal(getLanguage(), "system");
  assert.equal(getLocale(), "zh-CN");
  setLanguage("system", "fr-FR");
  assert.equal(getLocale(), "en");
  setLanguage("zh-CN", "en");
  assert.equal(getLocale(), "zh-CN");
  setLanguage("invalid");
  assert.equal(getLanguage(), "system");
  assert.equal(getLocale(), resolveLocale(globalThis.navigator?.language));
});

test("switching updates all label models and leaves user content untouched", () => {
  setLanguage("en");
  assert.equal(tr("pairing.connect::连接"), "Connect");
  assert.equal(statuses.awaiting_approval, "Awaiting approval");
  assert.deepEqual(Object.values(statuses), [
    "Awaiting approval",
    "Denied",
    "Accepted",
    "Running",
    "Succeeded",
    "Failed",
    "Cancelled",
    "Timed out",
    "Unknown",
  ]);
  assert.deepEqual(Object.values(tierLabels), [
    "Observe screen",
    "Click and type",
    "Irreversible operations",
  ]);
  assert.deepEqual(Object.values(tierPolicyLabels), [
    "Allow",
    "Ask first",
    "Deny",
  ]);
  for (const preset of Object.values(presets)) {
    assert.ok(!/[\u3400-\u9fff]/.test(preset.label + preset.detail));
  }
  assert.equal(tierLabels.high, "Irreversible operations");
  assert.equal(presets.balanced.label, "Balanced");
  assert.equal(
    tr("已切换到“{0}”", "中文 {1} <script>"),
    "Switched to “中文 {1} <script>”",
  );
  assert.equal(tr("{0}/{1} 台服务器在线", 1, 2), "1/2 servers online");
  assert.equal(tr("{0}/{1} 台服务器在线", null), "/{1} servers online");
  assert.equal(tr("tool returns {}"), "tool returns {}");
  assert.equal(tr("toString"), "toString");
  assert.equal(tr("__proto__"), "__proto__");
  const command = "echo '中文'";
  assert.equal(title({ kind: "exec.start", arguments: { command } }), command);
  assert.equal(
    explainTask({ status: "failed", error: { message: "服务端原文" } }).what,
    "服务端原文",
  );
  setLanguage("zh-CN");
  assert.equal(tr("pairing.connect::连接"), "连接");
  assert.equal(statuses.awaiting_approval, "待确认");
  assert.equal(tr("未提供的文案"), "未提供的文案");
});

test("subscriptions unsubscribe and browser language changes update the document", async () => {
  let updates = 0;
  const unsubscribe = subscribeLanguage(() => updates++);
  setLanguage("en");
  assert.equal(updates, 1);
  unsubscribe();
  setLanguage("zh-CN");
  assert.equal(updates, 1);
  globalThis.document = { documentElement: { lang: "" } };
  try {
    setLanguage("system", "zh-CN");
    assert.equal(document.documentElement.lang, "zh-CN");
    refreshSystemLanguage();
    assert.equal(getLocale(), resolveLocale(globalThis.navigator?.language));
    setLanguage("zh-CN");
    refreshSystemLanguage();
    assert.equal(getLocale(), "zh-CN");
  } finally {
    delete globalThis.document;
  }
});

test("catalog has translations and matching placeholders for every UI and native key", () => {
  const slots = (text) =>
    [...text.matchAll(/\{(\d+)\}/g)].map((m) => m[1]).sort();
  for (const [key, value] of Object.entries(english)) {
    assert.ok(value.trim(), key);
    assert.ok(!/[\u3400-\u9fff]/.test(value), key);
    assert.deepEqual(slots(value), slots(key), key);
  }
  const dir = new URL("../src/", import.meta.url);
  for (const name of fs
    .readdirSync(dir)
    .filter((n) => /\.(tsx|ts|mjs)$/.test(n))) {
    const ast = ts.createSourceFile(
      name,
      fs.readFileSync(new URL(name, dir), "utf8"),
      ts.ScriptTarget.Latest,
      true,
    );
    function visit(node) {
      if (
        (ts.isStringLiteral(node) || ts.isJsxText(node)) &&
        /[\u3400-\u9fff]/.test(node.text) &&
        node.text !== "中文"
      ) {
        assert.ok(
          ts.isCallExpression(node.parent) &&
            node.parent.expression.getText(ast) === "tr" &&
            node.parent.arguments[0] === node,
          `${name}: untranslated UI text: ${node.text}`,
        );
      }
      if (ts.isCallExpression(node) && node.expression.getText(ast) === "tr") {
        assert.ok(ts.isStringLiteral(node.arguments[0]), name);
        assert.ok(
          english[node.arguments[0].text],
          `${name}: ${node.arguments[0].text}`,
        );
      }
      ts.forEachChild(node, visit);
    }
    visit(ast);
  }
  const native = new URL("../src-tauri/src/", import.meta.url);
  for (const name of fs.readdirSync(native).filter((n) => n.endsWith(".rs"))) {
    const source = fs.readFileSync(new URL(name, native), "utf8");
    for (const m of source.matchAll(
      /\b(?:tr|tr_format)\(\s*("(?:\\.|[^"\\])*?")/g,
    )) {
      const key = JSON.parse(m[1]);
      assert.ok(english[key], `${name}: ${key}`);
    }
  }
  const panel = fs.readFileSync(new URL("native_panel.m", native), "utf8");
  for (const match of panel.matchAll(/localized:@"([^"\n]*)"/g)) {
    assert.ok(english[match[1]], `native panel: ${match[1]}`);
  }
});
