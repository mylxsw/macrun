// Lightweight checks for the offline prototype; does not replace a browser review.
// Usage: node docs/desktop/prototype/check.cjs
const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');
const { execFileSync } = require('child_process');

execFileSync(process.execPath, [path.join(__dirname, 'build.cjs'), '--check'], { stdio: 'inherit' });

const html = fs.readFileSync(path.join(__dirname, 'index.html'), 'utf8');
const script = html.match(/<script>([\s\S]*)<\/script>/)[1];
const logic = script.slice(0, script.indexOf('function lookup('));
const ctx = {};
vm.createContext(ctx);
vm.runInContext(logic + '\nthis.SCREENS = SCREENS; this.defs = defs;', ctx);

for (const s of ctx.SCREENS) {
  assert(html.includes(`<template id="t-${s.id}">`), `${s.id}: template missing`);
  assert(ctx.defs[s.id], `${s.id}: logic missing`);
}

function make(id, props) {
  let renders = 0;
  const inst = new ctx.defs[id](Object.assign({}, props), () => { renders += 1; });
  return { inst, vals: () => inst.renderVals(), renders: () => renders };
}

for (const s of ctx.SCREENS) make(s.id, s.defaults).vals();

const main = make('Main', { paused: false });
assert.equal(main.vals().paused, false);
main.vals().togglePause();
assert.equal(main.vals().paused, true);
assert.equal(main.renders(), 1);

const tasks = make('Tasks', {});
assert.equal(tasks.vals().list.length, 8);
assert.equal(tasks.vals().sel.stLabel, '未知');
tasks.vals().filters.find((f) => f.label === '失败').pick();
assert.deepEqual(tasks.vals().list.map((t) => t.title), ['make test']);
tasks.vals().list[0].pick();
assert.equal(tasks.vals().sel.exit, '2');
tasks.vals().filters.find((f) => f.label === '取消 / 超时').pick();
assert.equal(tasks.vals().list.length, 2);

const desktop = make('Desktop', {});
assert.equal(desktop.vals().enabled, true);
desktop.vals().toggle();
assert.equal(desktop.vals().enabled, false);

const settings = make('Settings', {});
assert.equal(settings.vals().showRules, true);
settings.vals().modes[0].pick();
assert.equal(settings.vals().showRules, false);

const menu = make('MenuBar', { initial: 'approval' });
assert.equal(menu.vals().approval, true);
menu.vals().allow();
assert.equal(menu.vals().working, true);
menu.vals().togglePause();
assert.equal(menu.vals().headline, '已暂停接收新任务');

const pair = make('Pairing', {});
assert.equal(pair.vals().s1, true);
pair.vals().next();
pair.vals().next();
assert.equal(pair.vals().s3, true);
pair.vals().reset();
assert.equal(pair.vals().s1, true);

console.log(`prototype checks passed (${ctx.SCREENS.length} screens)`);
