import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const violations = [];

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

const forbiddenRuntimePattern =
  /@tauri-apps|tauri-plugin-|cargo-tauri|src-tauri|#\[tauri::|codex_monitor_daemon|CODEX_MONITOR_DAEMON/i;
const scanRoots = [
  "src",
  "browser",
  "server",
  "crates",
  ".github",
  "Cargo.toml",
  "Cargo.lock",
  "flake.nix",
  "package-lock.json",
  "tsconfig.json",
  "vite.config.ts",
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

process.stdout.write("No desktop runtime dependency found.\n");
