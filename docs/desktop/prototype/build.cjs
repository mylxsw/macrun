// Builds the offline prototype (index.html) from the design sources in screens/.
// Usage: node docs/desktop/prototype/build.cjs [--check]
// --check exits non-zero when index.html is out of date.
const fs = require('fs');
const path = require('path');

const dir = __dirname;
const screens = [
  { file: 'Main.dc.html', label: '现场', kind: 'page' },
  { file: 'Tasks.dc.html', label: '任务', kind: 'page' },
  { file: 'Desktop.dc.html', label: '桌面控制', kind: 'page' },
  { file: 'Settings.dc.html', label: '设置与安全', kind: 'page' },
  { file: 'MenuBar.dc.html', label: '菜单栏', kind: 'fixed' },
  { file: 'Overlay.dc.html', label: '屏幕提示', kind: 'fixed' },
  { file: 'Pairing.dc.html', label: '首次配对', kind: 'fixed' }
];

function decodeAttr(s) {
  return s.replace(/&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&');
}

function pick(src, re, what, file) {
  const m = src.match(re);
  if (!m) throw new Error(`${file}: missing ${what}`);
  return m[1];
}

function parse(screen) {
  const file = screen.file;
  const src = fs.readFileSync(path.join(dir, 'screens', file), 'utf8');
  const helmet = pick(src, /<helmet>([\s\S]*?)<\/helmet>/, '<helmet>', file);
  const style = pick(helmet, /<style>([\s\S]*?)<\/style>/, '<style>', file);
  const markup = pick(src, /<\/helmet>([\s\S]*?)<\/x-dc>/, 'markup', file).trim();
  const propsJson = decodeAttr(pick(src, /data-props='([^']*)'/, 'data-props', file));
  const code = pick(src, /<script type="text\/x-dc"[^>]*>([\s\S]*?)<\/script>/, 'logic', file);
  const defaults = {};
  for (const [k, v] of Object.entries(JSON.parse(propsJson))) {
    if (!k.startsWith('$') && v && 'default' in v) defaults[k] = v.default;
  }
  const id = file.replace('.dc.html', '');
  return Object.assign({ id, style, markup, defaults, code }, screen);
}

const safe = (s) => s.replace(/<\/(script)/gi, '<\\/$1');

function build() {
  const parsed = screens.map(parse);
  const templates = parsed.map((s) => `<template id="t-${s.id}">${s.markup}</template>`).join('\n');
  const meta = parsed.map((s) => ({ id: s.id, label: s.label, kind: s.kind, file: s.file, style: s.style, defaults: s.defaults }));
  const classes = parsed
    .map((s) => `defs[${JSON.stringify(s.id)}] = (function () {\n${s.code.trim()}\nreturn Component;\n})();`)
    .join('\n');
  const tpl = fs.readFileSync(path.join(dir, 'shell.html'), 'utf8');
  return tpl
    .replace('<!--TEMPLATES-->', () => templates)
    .replace('/*SCREENS*/', () => safe(JSON.stringify(meta)))
    .replace('/*CLASSES*/', () => safe(classes));
}

const out = path.join(dir, 'index.html');
const html = build();
if (process.argv.includes('--check')) {
  const current = fs.existsSync(out) ? fs.readFileSync(out, 'utf8') : '';
  if (current !== html) {
    console.error('index.html is out of date; run: node docs/desktop/prototype/build.cjs');
    process.exit(1);
  }
  console.log('index.html is up to date');
} else {
  fs.writeFileSync(out, html);
  console.log(`wrote ${path.relative(process.cwd(), out)} (${html.length} bytes)`);
}
