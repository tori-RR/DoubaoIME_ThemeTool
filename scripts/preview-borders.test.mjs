import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/app.js", import.meta.url), "utf8");
const styles = readFileSync(new URL("../ui/styles.css", import.meta.url), "utf8");
const paint = source.slice(source.indexOf("function paintPreview("), source.indexOf("function paintSwatches("));
const rgb = source.slice(source.indexOf("function parseHex("), source.indexOf("function rgbToHex("));
const adaptive = source.slice(source.indexOf("function adaptiveBorderColor("), source.indexOf("function paintPreview("));
const toolbar = source.slice(source.indexOf("function toolbarKey("), source.indexOf("function placeComboMenu("));

function preview(theme, opacity, background = "#2A2A2A") {
  const properties = new Map();
  const bg = { style: {} };
  const bar = { style: { setProperty: (key, value) => properties.set(key, value) } };
  const context = {
    theme, opacity, background, previewBar: bar, lastStatus: null,
    document: { documentElement: { style: { setProperty() {} } } },
    $: () => bg,
    clampGlassOpacity: (value) => Math.max(20, Math.min(100, Math.round(value))),
    hexRgba: (hex, alpha) => `${hex}/${alpha}`,
    readGlassOpacity: () => 100,
  };
  runInNewContext(`${rgb}\n${adaptive}\n${toolbar}\n${paint}\npaintPreview({background, foreground: '#E8E8E8', accent: '#545454'}, null, theme, opacity);`, context);
  return { properties, bg, bar, context };
}

test("Light and Dark candidate borders use official colour/alpha at every opacity", () => {
  for (const opacity of [20, 70, 100]) {
    const light = preview("light", opacity, "#FFFFFF");
    const dark = preview("dark", opacity);
    assert.equal(light.properties.get("--candidate-outline"), "rgba(0, 0, 0, 0.08)");
    assert.equal(dark.properties.get("--candidate-outline"), "rgba(255, 255, 255, 0.08)");
    for (const [view, background] of [[light, "#FFFFFF"], [dark, "#2A2A2A"]]) {
      assert.equal(view.bg.style.opacity, "1", "the generated SVG fades its face, never its border");
      assert.equal(view.bar.style.background, `${background}/${opacity / 100}`);
    }
  }
});

test("candidate outline is a full inset pixel without changing content layout or pointer targets", () => {
  const outline = styles.match(/\.bar::after\s*\{([^}]+)\}/)?.[1];
  assert.ok(outline);
  assert.match(outline, /position:\s*absolute/);
  assert.match(outline, /inset:\s*0/);
  assert.match(outline, /border:\s*1px solid var\(--candidate-outline,/);
  assert.match(outline, /border-radius:\s*inherit/);
  assert.match(outline, /pointer-events:\s*none/);
});

test("Midlight and custom use inverse neutral borders without fading the outline", () => {
  for (const theme of ["mix", "custom-1"]) for (const opacity of [20, 70, 100]) {
    for (const [background, expected] of [
      ["#000000", "rgba(255, 255, 255, 0.08)"],
      ["#FFFFFF", "rgba(0, 0, 0, 0.08)"],
      ["#3D57D6", "rgba(255, 255, 255, 0.08)"],
      ["#FEE5CA", "rgba(0, 0, 0, 0.08)"],
    ]) {
      const view = preview(theme, opacity, background);
      assert.equal(view.properties.get("--candidate-outline"), expected);
      assert.equal(view.bg.style.opacity, "1");
    }
  }
});

test("preview luminance boundary and saturated colours match Rust expectations", () => {
  const { context } = preview("mix", 100);
  for (const [background, expected] of [
    ["#757575", "white"], ["#767676", "black"],
    ["#FF0000", "black"], ["#00FF00", "black"], ["#0000FF", "white"],
  ]) assert.equal(runInNewContext(`adaptiveBorderColor('${background}')`, context), expected);
});

test("toolbar cache differentiates palette-identical themes and opacity changes", () => {
  const { context } = preview("dark", 100);
  context.colors = { background: "#FFFFFF", foreground: "#000000", accent: "#4F84FF" };
  const key = (theme, opacity) => runInNewContext(`toolbarKey({colors, theme_id: '${theme}'}, ${opacity})`, context);
  assert.notEqual(key("light", 100), key("custom-1", 100));
  assert.notEqual(key("dark", 100), key("dark", 70));
});

test("toolbar request carries the preview theme instead of relying on saved state", () => {
  const request = source.slice(source.indexOf('invoke("get_toolbar_preview"'), source.indexOf("// Each render supersedes"));
  assert.match(request, /themeId: status \? status\.theme_id/);
  assert.match(request, /syncToolbarOpacity\(\)/);
});
