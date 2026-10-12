// Read-only browser benchmark. Font names come from this Windows installation;
// no font files or application settings are copied or modified.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const cliPath = process.argv[2];
if (!cliPath) throw new Error("Pass the installed Playwright CLI path");
const baseline = process.argv.includes("--baseline");
const screenshot = !process.argv.includes("--no-screenshot");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = join(root, "dist/output/playwright");
mkdirSync(output, { recursive: true });
const names = JSON.parse(execFileSync("powershell.exe", ["-NoProfile", "-Command",
  "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Add-Type -AssemblyName System.Drawing; $collection = New-Object System.Drawing.Text.InstalledFontCollection; try { @($collection.Families | ForEach-Object { $_.Name }) | ConvertTo-Json -Compress } finally { $collection.Dispose() }",
], { encoding: "utf8" }).replace(/^\uFEFF/, ""));
const cli = (...args) => {
  const result = execFileSync(process.execPath, [cliPath, "--session", "tt-font-menu", ...args], {
    cwd: join(root, "dist"), encoding: "utf8", timeout: 60000,
  });
  if (result.includes("### Error")) throw new Error(result);
  return result;
};
try {
  console.log(cli("open", "http://127.0.0.1:1421/index.html", "--browser", "msedge"));
  const code = `async page => {
    await page.setViewportSize({width: 464, height: 480});
    const timing = await page.evaluate(async names => {
      // Replace the mock picker's nodes so a single real-list binding is timed.
      for (const id of ['font-face', 'font-face-menu', 'font-face-toggle']) {
        const old = document.getElementById(id); old.replaceWith(old.cloneNode(true));
      }
      bindFontPickers({faces: names.map(name => ({name, keys: []}))});
      const menu = document.getElementById('font-face-menu');
      const start = performance.now();
      document.getElementById('font-face-toggle').click();
      const handlerMs = performance.now() - start;
      const immediate = {open: !menu.hidden, loading: menu.textContent.includes('加载'),
        options: menu.querySelectorAll('[role=option]').length};
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return {fontCount: names.length, handlerMs, firstFrameMs: performance.now() - start, immediate};
    }, ${JSON.stringify(names)});
    await page.waitForFunction(() => {
      const menu = document.getElementById('font-face-menu');
      return !menu.hidden && menu.getAttribute('aria-busy') !== 'true';
    });
    const final = await page.evaluate(() => ({
      options: document.querySelectorAll('#font-face-menu [role=option]').length,
      styledFonts: [...document.querySelectorAll('#font-face-menu .combo-name')].filter(el => el.style.fontFamily).length,
      active: document.getElementById('font-face').getAttribute('aria-activedescendant'),
    }));
    if (${screenshot}) await page.screenshot({path: ${JSON.stringify(join(output, `font-menu-${baseline ? "before" : "after"}.png`))}});
    if (${baseline}) return {timing, final};
    if (!timing.immediate.open || !timing.immediate.loading || timing.immediate.options !== 0)
      throw new Error('The menu must open before any font rows are created');
    if (final.styledFonts >= 30) throw new Error('Offscreen typefaces were eagerly rendered');
    const behaviors = await page.evaluate(async () => {
      const passed = [];
      const check = (ok, label) => { if (!ok) throw new Error(label); passed.push(label); };
      const until = async predicate => {
        const deadline = performance.now() + 5000;
        while (!predicate()) {
          if (performance.now() > deadline) throw new Error('Timed out waiting for font menu');
          await new Promise(resolve => requestAnimationFrame(resolve));
        }
      };
      const fixtures = [];
      const fixture = loadFamilies => {
        const wrap = document.createElement('div');
        wrap.className = 'combo';
        wrap.style.cssText = 'position:fixed;left:20px;top:20px;width:200px';
        const input = document.createElement('input');
        input.id = 'fixture-font-' + fixtures.length;
        input.value = 'Microsoft YaHei';
        const toggle = document.createElement('button');
        toggle.className = 'combo-toggle';
        const menu = document.createElement('div');
        menu.className = 'combo-menu'; menu.hidden = true;
        const list = document.createElement('ul');
        list.className = 'combo-list'; list.setAttribute('role', 'listbox');
        menu.append(list); wrap.append(input, toggle, menu); document.body.append(wrap);
        const commits = [];
        setupCombo(input, menu, toggle, [], item => commits.push(item.name), {
          previewFace: true, fallback: () => 'Microsoft YaHei', loadFamilies,
        });
        const api = {input, toggle, menu, list, commits,
          open: () => toggle.click(),
          key: key => input.dispatchEvent(new KeyboardEvent('keydown', {key, bubbles: true})),
          query: value => {input.value = value; input.dispatchEvent(new Event('input', {bubbles: true}));},
          ready: () => !menu.hidden && menu.getAttribute('aria-busy') === 'false',
        };
        fixtures.push(wrap); return api;
      };
      const items = [{name: '微软雅黑', aliases: ['Microsoft YaHei'], keys: ['weiruanyahei', 'wryh', 'microsoftyahei']},
        ...Array.from({length: 150}, (_, i) => ({name: 'Family ' + String(i).padStart(3, '0'), keys: []}))];
      let calls = 0, finish;
      const first = fixture(() => {calls++; return new Promise(resolve => {finish = resolve;});});
      first.open();
      check(!first.menu.hidden && calls === 0, 'menu opens before enumeration starts');
      await until(() => calls === 1);
      first.query('missing'); first.query('wryh');
      finish(items); await until(first.ready);
      check(first.list.querySelectorAll('[role=option]').length === 1 && first.list.textContent === '微软雅黑',
        'rapid search keeps only the latest pinyin result');
      first.key('Escape');
      check(first.menu.hidden && first.input.value === 'Microsoft YaHei' && !first.commits.length,
        'Escape restores the previous value without committing');
      first.open(); await until(first.ready);
      check(calls === 1 && first.list.querySelectorAll('[role=option]').length === 151,
        'reopen reuses the font enumeration and retains the full list');
      first.query('Microsoft'); await until(first.ready);
      check(first.list.textContent === '微软雅黑', 'localized font aliases remain searchable');
      first.query('Family'); await until(first.ready);
      first.key('ArrowDown'); first.key('Enter');
      check(first.menu.hidden && first.commits[0] === 'Family 001', 'arrow keys and Enter commit the chosen row');
      first.open(); await until(first.ready); first.query('no-such-typeface'); await until(first.ready);
      first.key('Enter');
      check(first.list.textContent === '无匹配' && first.commits.length === 1,
        'empty results cannot be committed');
      first.key('Escape');

      let completeLate, lateCalls = 0;
      const late = fixture(() => {lateCalls++; return new Promise(resolve => {completeLate = resolve;});});
      late.open(); await until(() => lateCalls === 1); late.key('Escape'); completeLate(items);
      await new Promise(resolve => setTimeout(resolve, 50));
      check(late.menu.hidden && late.list.querySelectorAll('[role=option]').length === 0,
        'late results do not reopen a closed menu');
      late.open(); await until(late.ready);
      check(lateCalls === 1, 'reopen can reuse a request that finished while closed');
      late.key('Escape');

      let attempts = 0;
      const failed = fixture(() => {if (++attempts === 1) throw new Error('simulated'); return items;});
      failed.open(); await until(failed.ready);
      check(failed.list.textContent.includes('加载失败') && !failed.commits.length,
        'enumeration failure stays visible and does not change the selection');
      failed.key('Escape'); failed.open(); await until(failed.ready);
      check(attempts === 2 && failed.list.querySelectorAll('[role=option]').length === 151,
        'a failed request is retried on the next open');
      failed.key('Escape');

      let finishOther, otherCalls = 0;
      const other = fixture(() => {otherCalls++; return new Promise(resolve => {finishOther = resolve;});});
      other.open(); await until(() => otherCalls === 1);
      document.getElementById('theme-toggle').click();
      finishOther(items); await new Promise(resolve => setTimeout(resolve, 50));
      check(other.menu.hidden && other.input.getAttribute('aria-expanded') === 'false'
        && !document.getElementById('theme-menu').hidden,
        'opening the theme menu cancels visible font loading without affecting themes');
      for (const wrap of fixtures) wrap.remove();
      return passed;
    });
    return {timing, final, behaviors};
  }`;
  const codePath = join(output, "font-menu.generated.js");
  writeFileSync(codePath, code);
  const result = cli("run-code", "--filename", codePath).split("### Ran Playwright code")[0];
  writeFileSync(join(output, `font-menu-${baseline ? "before" : "after"}.txt`), result);
  console.log(result);
} finally {
  console.log(cli("close"));
}
