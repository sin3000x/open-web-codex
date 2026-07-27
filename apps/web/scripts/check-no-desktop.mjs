import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { basename, join, relative } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const violations = [];

const runtimeAliases = {
  "@tauri-apps/api/app": "browser/browser/app.ts",
  "@tauri-apps/api/core": "browser/browser/core.ts",
  "@tauri-apps/api/dpi": "browser/browser/dpi.ts",
  "@tauri-apps/api/event": "browser/browser/event.ts",
  "@tauri-apps/api/menu": "browser/browser/menu.ts",
  "@tauri-apps/api/webview": "browser/browser/webview.ts",
  "@tauri-apps/api/window": "browser/browser/window.ts",
  "@tauri-apps/plugin-dialog": "browser/browser/dialog.ts",
  "@tauri-apps/plugin-notification": "browser/browser/notification.ts",
  "@tauri-apps/plugin-opener": "browser/browser/opener.ts",
  "@tauri-apps/plugin-process": "browser/browser/process.ts",
  "@tauri-apps/plugin-updater": "browser/browser/updater.ts",
  "tauri-plugin-liquid-glass-api": "browser/browser/liquidGlass.ts",
};

for (const directory of ["src-tauri", "desktop", "daemon"]) {
  if (existsSync(join(root, directory))) violations.push(`${directory}/ exists`);
}

const packageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
for (const section of ["scripts", "dependencies", "devDependencies"]) {
  for (const [key, value] of Object.entries(packageJson[section] ?? {})) {
    if (section === "scripts" && key === "check:no-desktop") continue;
    if (/tauri|electron|desktop|sidecar/i.test(`${key} ${String(value)}`)) {
      violations.push(`package.json ${section}.${key}`);
    }
  }
}

const tsconfigSource = readFileSync(join(root, "tsconfig.json"), "utf8");
const viteConfig = readFileSync(join(root, "vite.config.ts"), "utf8");
const escapePattern = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
for (const [specifier, target] of Object.entries(runtimeAliases)) {
  if (!existsSync(join(root, target))) {
    violations.push(`missing browser shim for ${specifier}: ${target}`);
  }
  const tsconfigAlias = new RegExp(
    `"${escapePattern(specifier)}"\\s*:\\s*\\[\\s*"${escapePattern(target)}"\\s*\\]`,
  );
  if (!tsconfigAlias.test(tsconfigSource)) {
    violations.push(`tsconfig alias for ${specifier} must resolve only to ${target}`);
  }
  if (!viteConfig.includes(JSON.stringify(specifier)) || !viteConfig.includes(`./${target}`)) {
    violations.push(`vite alias for ${specifier} must resolve to ${target}`);
  }
}

const usedRuntimeSpecifiers = new Set();
const importPattern = /(?:from\s+|import\s*\()\s*["'](@tauri-apps\/[^"']+|tauri-plugin-[^"']+)["']/g;
function collectRuntimeImports(path) {
  if (statSync(path).isDirectory()) {
    for (const entry of readdirSync(path)) collectRuntimeImports(join(path, entry));
    return;
  }
  if (!/\.(?:ts|tsx)$/.test(path)) return;
  const source = readFileSync(path, "utf8");
  for (const match of source.matchAll(importPattern)) usedRuntimeSpecifiers.add(match[1]);
}
collectRuntimeImports(join(root, "src"));
for (const specifier of usedRuntimeSpecifiers) {
  if (!(specifier in runtimeAliases)) {
    violations.push(`unmapped UI runtime import: ${specifier}`);
  }
}
for (const specifier of Object.keys(runtimeAliases)) {
  if (!usedRuntimeSpecifiers.has(specifier)) {
    violations.push(`unused browser runtime alias: ${specifier}`);
  }
}

const forbiddenRuntimePattern =
  /@tauri-apps|tauri-plugin-|cargo-tauri|src-tauri|#\[tauri::|codex_monitor_daemon|CODEX_MONITOR_DAEMON/i;
const scanRoots = [
  "browser",
  "server",
  "crates",
  ".github",
  "Cargo.toml",
  "Cargo.lock",
  "flake.nix",
  "package-lock.json",
];

function scan(path) {
  if (!existsSync(path)) return;
  if (statSync(path).isDirectory()) {
    for (const entry of readdirSync(path)) scan(join(path, entry));
    return;
  }
  if (!/\.(?:rs|ts|tsx|js|mjs|json|ya?ml|toml|nix|lock)$/.test(path)) return;
  if (forbiddenRuntimePattern.test(readFileSync(path, "utf8"))) {
    violations.push(relative(root, path));
  }
}
for (const entry of scanRoots) scan(join(root, entry));

if (violations.length > 0) {
  process.stderr.write(
    `Desktop runtime coupling is forbidden:\n${violations.map((value) => `- ${value}`).join("\n")}\n`,
  );
  process.exit(1);
}

process.stdout.write(
  `No desktop runtime dependency found; ${usedRuntimeSpecifiers.size} original UI module contracts resolve to browser-only shims.\n`,
);
