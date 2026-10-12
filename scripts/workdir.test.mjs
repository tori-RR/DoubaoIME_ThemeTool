import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const workdir = read("src-tauri/src/workdir.rs");
const backend = read("src-tauri/src/main.rs");
const frontend = read("ui/app.js");

test("runtime and fallback use the new directory while retaining backup filenames", () => {
  assert.ok(workdir.includes('pub const APP_DIR: &str = r"C:\\tmp\\DoubaoIME_TT";'));
  const context = {};
  runInNewContext(`${frontend.match(/^const WORKDIR = .*;$/m)[0]}\nresult = WORKDIR;`, context);
  assert.equal(context.result, String.raw`C:\tmp\DoubaoIME_TT`);
  assert.ok(workdir.includes('const STATE_FILE: &str = "DoubaoIME Darkmode.json";'));
});

test("review mode skips migration before any real directory is accessed", () => {
  const migration = workdir.slice(workdir.indexOf("pub fn migrate_legacy_dir"), workdir.indexOf("pub fn check_migration"));
  assert.ok(migration.indexOf("if review_mode()") < migration.indexOf("ensure_app_dir()?"));
  assert.ok(backend.includes("workdir::migrate_legacy_dir("));
  for (const functionName of ["state_lock", "prepare_job"]) {
    assert.match(backend.slice(backend.indexOf(`fn ${functionName}(`)), /\{\s*workdir::check_migration\(\)\?;/);
  }
});

test("migration failure is visible and disables direct installation and restoration", () => {
  assert.match(backend, /Err\(err\) => \{\s*warning = err;\s*false\s*\}/);
  assert.ok(backend.includes("ime_compatible: compatible && migration_ok"));
  assert.ok(backend.includes("can_restore: migration_ok &&"));
});

test("artifact names use TT but local builds retain source and test labels", () => {
  const script = read("scripts/pack_release.ps1");
  assert.ok(script.includes('$name = "DoubaoIME_TT-$($conf.version)-$flavor-$commit-$stamp.exe"'));
  assert.ok(script.includes("if ($dirty -and -not $Development)"));
  assert.ok(script.includes("'test' } else { 'release'"));
  assert.ok(script.includes("[System.IO.File]::Copy($out, $desktop, $false)"));
});
