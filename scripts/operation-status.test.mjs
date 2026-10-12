import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/app.js", import.meta.url), "utf8");
const calls = source.slice(source.indexOf("async function settleOperationStatus("), source.indexOf("const MOCK_FONTS"));
const unknown = { ime_installed: true, ime_version: null, ime_compatible: false };
const ready = { ime_installed: true, ime_version: "v0.9.2.7", ime_compatible: true };

function harness(invoke) {
  const rendered = [], errors = [], commands = [];
  let delays = 0;
  const context = {
    inflight: null, commandQueue: Promise.resolve(), lastStatus: ready, flashTimer: 0,
    invoke: async (name) => { commands.push(name); return invoke(name); },
    window: { clearTimeout() {}, setTimeout(fn) { delays++; fn(); } },
    document: { querySelectorAll: () => [], querySelector: () => ({ setAttribute() {}, removeAttribute() {} }) },
    showError: (error) => errors.push(error),
    render: (status) => { rendered.push(status); context.lastStatus = status; },
    closeColorCard() {}, setPluginHint() {}, flashUninstalled() {},
    uninstallError: (error) => String(error),
  };
  return {
    call: (name) => runInNewContext(`${calls}\ncall(${JSON.stringify(name)});`, context),
    rendered, errors, commands, delays: () => delays,
  };
}

test("install waits out an empty version snapshot and never repeats the write", async () => {
  let reads = 0;
  const h = harness(name => name === "install" ? unknown : (++reads < 3 ? unknown : ready));
  const result = await h.call("install");
  assert.equal(result.ime_version, "v0.9.2.7");
  assert.equal(h.commands.filter(name => name === "install").length, 1);
  assert.equal(reads, 3);
  assert.ok(h.rendered.every(status => status.ime_version === "v0.9.2.7"));
});

test("failed operation rechecks live status without hiding its original error", async () => {
  let reads = 0;
  const h = harness(name => {
    if (name === "install") throw new Error("transaction failed");
    return ++reads < 3 ? unknown : ready;
  });
  assert.equal(await h.call("install"), null);
  assert.equal(h.commands.filter(name => name === "install").length, 1);
  assert.ok(h.rendered.every(status => status.ime_version === "v0.9.2.7"));
  assert.match(h.errors.at(-1), /transaction failed/);
});

test("persistent uncertainty remains blocked after a bounded read-only retry", async () => {
  const h = harness(() => unknown);
  const result = await h.call("uninstall");
  assert.equal(result.ime_version, null);
  assert.equal(result.ime_compatible, false);
  assert.equal(h.delays(), 8);
  assert.deepEqual(h.commands, ["uninstall", ...Array(8).fill("get_status")]);
});

test("ordinary status reads and a genuinely absent IME are not retried", async () => {
  const read = harness(() => unknown);
  await read.call("get_status");
  assert.equal(read.delays(), 0);
  const absent = harness(() => ({ ime_installed: false, ime_version: null }));
  await absent.call("install");
  assert.equal(absent.delays(), 0);
});
