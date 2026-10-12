// Local visual check using the existing Playwright CLI and the user's own SVGs.
// Official assets are read at runtime, never committed or shipped with the app.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const cliPath = process.argv[2];
if (!cliPath) throw new Error("Pass the installed @playwright/cli/playwright-cli.js path");
if (!existsSync(cliPath)) throw new Error(`Playwright CLI not found: ${cliPath}`);
const root = fileURLToPath(new URL("../", import.meta.url));
const pageUrl = process.argv[4] || "http://127.0.0.1:1421/index.html";
const skin = process.argv[3] || String.raw`C:\Program Files\DoubaoIME\versions\v0.9.2.7\files\data\skin\default`;
const original = existsSync(join(skin, "dmdm_backup")) ? join(skin, "dmdm_backup") : skin;
const output = join(root, "dist/output/playwright");
mkdirSync(output, { recursive: true });
const variants = [
  { theme: "light", border: "black", colors: { accent: "#4F84FF", background: "#FFFFFF", foreground: "#000000", emphasis: "#FFFFFF" } },
  { theme: "dark", border: "white", colors: { accent: "#545454", background: "#2A2A2A", foreground: "#E8E8E8", emphasis: "#FFFFFF" } },
  { theme: "mix", border: "white", colors: { accent: "#7794E4", background: "#3D57D6", foreground: "#FEE5CA", emphasis: "#FFFFFF" } },
  { theme: "custom-light", border: "black", colors: { accent: "#7794E4", background: "#FEE5CA", foreground: "#000000", emphasis: "#FFFFFF" } },
  { theme: "custom-dark", border: "white", colors: { accent: "#7794E4", background: "#1C2233", foreground: "#FEE5CA", emphasis: "#FFFFFF" } },
];
const read = (name) => readFileSync(join(original, "dark/status_wnd", name), "utf8");
const background = read("status_bar_bg.svg");
assert.equal(background.split('fill="#262626"').length, 2);
assert.equal(background.split('stroke="white" stroke-opacity="0.08"').length, 2);
const sources = {
  bg: background, divider: read("status_divider.svg"), mic: read("mic_normal.svg"),
  lang: read("lang_cn_normal.svg"), punct: read("punct_cn_normal.svg"), half: read("half_shape_normal.svg"),
  logo_size: 22, logo: [...readFileSync(join(original, "dark/status_wnd/logo.png"))],
};
const cases = variants.flatMap(({ theme, colors, border }) => [100, 85, 70].flatMap((opacity) =>
  ["light", "dark"].map((stage) => ({
    theme, colors, border, opacity, stage,
    screenshot: join(output, `borders-${theme}-${opacity}-${stage}-stage.png`),
  }))));
const cli = (...args) => {
  const result = execFileSync(process.execPath, [cliPath, "--session", "tt-borders-visual", ...args], {
    cwd: join(root, "dist"), encoding: "utf8", timeout: 60000,
  });
  if (result.includes("### Error")) throw new Error(result);
  return result;
};
try {
  console.log(cli("open", pageUrl, "--browser", "msedge"));
  const code = `async (page) => {
    await page.setViewportSize({width: 464, height: 480});
    await page.evaluate(sources => { window.borderSources = sources; }, ${JSON.stringify(sources)});
    const results = [];
    for (const variant of ${JSON.stringify(cases)}) {
      const result = await page.evaluate(async ({theme, colors, opacity, stage}) => {
        // The Rust integration check separately compares actual preview/install bytes.
        const preview = {...window.borderSources};
        const faceOpacity = opacity / 100;
        preview.bg = preview.bg.replace('fill="#262626"',
          'fill="' + colors.background + '" fill-opacity="' + faceOpacity.toFixed(2) + '"');
        const border = theme === 'light' ? 'black' : theme === 'dark'
          ? 'white' : adaptiveBorderColor(colors.background);
        const borderAlpha = theme === 'light' ? '0.1' : '0.08';
        preview.bg = preview.bg.replace('stroke="white" stroke-opacity="0.08"',
          'stroke="' + border + '" stroke-opacity="' + borderAlpha + '"');
        for (const key of ['mic', 'lang', 'punct', 'half']) preview[key] = preview[key]
          .replaceAll('fill="white" fill-opacity="0.85"', 'fill="' + colors.foreground + '" fill-opacity="0.85"')
          .replaceAll('fill="#5B8CFF"', 'fill="' + colors.accent + '"');
        setThemeInput(theme, {force: true}); setPickers(colors); setGlassInput(opacity);
        paintPreview(colors, {face: 'Microsoft YaHei'}, theme, opacity);
        showToolbar(preview, false); syncToolbarOpacity();
        if (theme.startsWith('custom-')) document.getElementById('theme-select').value =
          theme === 'custom-light' ? '自定义浅底测试' : '自定义深底测试';
        document.querySelector('.preview').classList.toggle('is-light', stage === 'light');
        document.getElementById('kaomoji-word').textContent = '(  ๑ω๑  )';
        await Promise.all([...document.querySelectorAll('#preview-toolbar img')].map(image => image.decode()));
        const bar = getComputedStyle(document.getElementById('preview-bar'));
        const edge = getComputedStyle(document.getElementById('preview-bar'), '::after');
        const toolbar = getComputedStyle(document.getElementById('toolbar-bg'));
        return {theme, opacity, stage, outline: bar.getPropertyValue('--candidate-outline').trim(),
          strokeWidth: edge.borderTopWidth, strokePointerEvents: edge.pointerEvents,
          shadow: bar.boxShadow, background: bar.backgroundColor, toolbarOpacity: toolbar.opacity,
          frameOverflow: getComputedStyle(document.querySelector('.bar-frame')).overflow,
          whitespace: getComputedStyle(document.getElementById('kaomoji-word')).whiteSpace,
          horizontalOverflow: document.documentElement.scrollWidth > innerWidth};
      }, variant);
      const expectedOutline = variant.border === 'white' ? 'rgba(255, 255, 255, 0.08)' : 'rgba(0, 0, 0, 0.08)';
      if (result.outline !== expectedOutline || result.toolbarOpacity !== '1'
          || result.strokeWidth !== '1px' || result.strokePointerEvents !== 'none'
          || result.frameOverflow !== 'visible' || result.whitespace !== 'pre'
          || result.horizontalOverflow) throw new Error(JSON.stringify(result));
      const outlined = await page.screenshot({path: variant.screenshot});
      if (variant.theme === 'light' || variant.theme === 'dark') {
        await page.evaluate(() => document.getElementById('preview-bar')
          .style.setProperty('--candidate-outline', 'transparent'));
        const bare = await page.screenshot();
        result.outlinePixels = await page.evaluate(async ({outlined, bare, outline}) => {
          const decode = async (encoded) => {
            const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
            const bitmap = await createImageBitmap(new Blob([bytes], {type: 'image/png'}));
            const canvas = document.createElement('canvas');
            canvas.width = bitmap.width; canvas.height = bitmap.height;
            const context = canvas.getContext('2d'); context.drawImage(bitmap, 0, 0);
            const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
            bitmap.close(); return pixels;
          };
          const before = await decode(outlined), after = await decode(bare);
          const bar = document.getElementById('preview-bar');
          const rect = bar.getBoundingClientRect(), scale = before.width / innerWidth;
          // Check all four straight edges, including partially covered pixel rows.
          // A mere nonzero pixel difference let the old 0.41px stroke pass despite
          // being imperceptible. Require at least half of the intended 8% blend.
          const sample = (x, y) => {
            const offset = (y * before.width + x) * 4;
            const on = [...before.data.slice(offset, offset + 3)];
            const off = [...after.data.slice(offset, offset + 3)];
            const tone = outline.includes('255') ? 255 : 0;
            const delta = Math.max(...on.map((value, i) => Math.abs(value - off[i])));
            const expected = Math.max(...off.map(value => Math.abs(tone - value) * 0.08));
            return {x, y, on, off, delta, visible: delta >= Math.max(4, expected * 0.5)};
          };
          const edges = [
            [rect.x + rect.width / 2, rect.top, 0, 1],
            [rect.x + rect.width / 2, rect.bottom, 0, -1],
            [rect.left, rect.y + rect.height / 2, 1, 0],
            [rect.right, rect.y + rect.height / 2, -1, 0],
          ].map(([x, y, dx, dy]) => {
            const samples = [0.25, 0.75].map(distance =>
              sample(Math.floor((x + dx * distance) * scale), Math.floor((y + dy * distance) * scale)));
            return samples.reduce((best, next) => next.delta > best.delta ? next : best);
          });
          bar.style.setProperty('--candidate-outline', outline);
          return {edges, visible: edges.every(edge => edge.visible)};
        }, {outlined: outlined.toString('base64'), bare: bare.toString('base64'), outline: result.outline});
        if (!result.outlinePixels.visible) throw new Error('The inset border is not sufficiently painted: ' + JSON.stringify(result));
      }
      results.push(result);
    }
    return results;
  }`;
  // Generated test fixture, not app source. A file avoids Windows argument limits.
  const codePath = join(output, "check-borders.generated.js");
  writeFileSync(codePath, code);
  const result = cli("run-code", "--filename", codePath);
  // CLI echoes the code (including runtime-only assets); report just the result.
  console.log(result.split("### Ran Playwright code")[0]);
} finally {
  console.log(cli("close"));
}
