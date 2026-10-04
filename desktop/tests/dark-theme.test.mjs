import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { build, darken } from "../scripts/dark-theme.mjs";

const lightness = (hex) => {
  const n = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  return (Math.max(...n) + Math.min(...n)) / 2;
};

test("surfaces stay ordered while text and borders invert", () => {
  const page = lightness(darken("#fbfbfa"));
  const card = lightness(darken("#ffffff"));
  const border = lightness(darken("#e6e5e1"));
  const text = lightness(darken("#1a1a19"));
  assert.ok(page < 0.2 && card < 0.2, "surfaces become dark");
  assert.ok(card > page, "cards stay lighter than the page");
  assert.ok(border > card, "borders stay visible on cards");
  assert.ok(text > 0.85, "body text becomes light");
});

test("vivid fills and translucent colours keep their value", () => {
  assert.equal(darken("#c2361f", true), "#c2361f");
  assert.notEqual(darken("#c2361f"), "#c2361f");
  assert.equal(darken("#0003"), "#0003");
  // Pale tints become dark tints of the same hue.
  assert.ok(lightness(darken("#fff9ed", true)) < 0.3);
});

test("generated stylesheet is current and leaves the terminal dark", () => {
  const css = build();
  assert.equal(readFileSync(new URL("../src/dark.css", import.meta.url), "utf8"), css);
  assert.match(css, /^\/\* Generated/);
  assert.match(css, /@media \(prefers-color-scheme: dark\)/);
  assert.doesNotMatch(css, /\.term\b/);
  assert.doesNotMatch(css, /overlay/);
});
