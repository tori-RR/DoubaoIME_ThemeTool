import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/app.js", import.meta.url), "utf8");
const named = source.slice(source.indexOf("function isNamed("), source.indexOf("function faceMatches("));
const current = source.slice(source.indexOf("function withCurrent("), source.indexOf("const DARK_COLORS"));
const bind = source.slice(source.indexOf("function bindFontPickers("), source.indexOf("const MAX_LOGO_BYTES"));

function bindMenu(invoke, lists) {
  let binding;
  const commands = [];
  const context = {
    lists, lastStatus: { fonts: { face: "Saved Font" } },
    $: id => id,
    invoke: async name => {commands.push(name); return invoke(name);},
    setupCombo: (...args) => {binding = args;},
  };
  runInNewContext(`${named}\n${current}\n${bind}\nbindFontPickers(lists);`, context);
  return {binding, commands};
}

test("font control is bound immediately without requesting system fonts", async () => {
  const {binding, commands} = bindMenu(() => ({faces: ["First", "Second"]}));
  assert.equal(binding[0], "font-face");
  assert.equal(binding[3].length, 0);
  assert.deepEqual(commands, []);
  assert.equal(binding[5].previewFace, true);
  const choices = await binding[5].loadFamilies();
  assert.deepEqual(commands, ["list_system_fonts"]);
  assert.deepEqual(Array.from(choices, item => item.name), ["Saved Font", "First", "Second"]);
});

test("localized saved font aliases are retained without adding a duplicate", async () => {
  const {binding} = bindMenu(() => ({faces: [{name: "字体", aliases: ["Saved Font"], keys: ["ziti"]}]}));
  const choices = await binding[5].loadFamilies();
  assert.equal(choices.length, 1);
  assert.equal(choices[0].name, "字体");
  assert.equal(choices[0].keys[0], "ziti");
});

test("browser fixtures avoid native calls and failed enumeration rejects for retry", async () => {
  const mock = bindMenu(() => {throw new Error("unexpected IPC");}, {faces: ["Saved Font"]});
  assert.equal((await mock.binding[5].loadFamilies()).length, 1);
  assert.deepEqual(mock.commands, []);
  const failed = bindMenu(() => {throw new Error("enumeration failed");});
  await assert.rejects(failed.binding[5].loadFamilies(), /enumeration failed/);
});
