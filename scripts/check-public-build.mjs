// Public Tauri builds must never embed the private debugging UI or file loader.
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, extname, join, relative, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const ui = resolve(root, "src-tauri", config.build.frontendDist);
const forbidden = /kaomoji-gallery|btn-refresh-kaomoji|app-menu-debug|set_debug_mode|setDebugMode|renderGallery|fn\s+external_dirs\s*\(/;

function inspect(path) {
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) inspect(child);
    else if ([".html", ".js", ".css", ".rs"].includes(extname(entry.name))) {
      const match = readFileSync(child, "utf8").match(forbidden);
      if (match) throw new Error(`Public build contains private debug content: ${relative(root, child)} (${match[0]})`);
    }
  }
}

inspect(ui);
inspect(join(root, "src-tauri/src"));
console.log("Public build checked: no kaomoji gallery, Debug menu or external debug loader.");
