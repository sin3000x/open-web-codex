import { readFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

const packageJson = JSON.parse(
  readFileSync(new URL("./package.json", import.meta.url), "utf-8"),
) as { version: string };

function commitHash() {
  const supplied = process.env.GIT_COMMIT ?? process.env.GITHUB_SHA ?? process.env.CI_COMMIT_SHA;
  if (supplied?.trim()) return supplied.trim().slice(0, 12);
  try {
    return execSync("git rev-parse --short=12 HEAD", { encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
}

function buildDate() {
  const sourceDateEpoch = process.env.SOURCE_DATE_EPOCH;
  if (sourceDateEpoch && /^\d+$/.test(sourceDateEpoch)) {
    const milliseconds = Number(sourceDateEpoch) * 1000;
    if (Number.isFinite(milliseconds) && milliseconds > 0) {
      return new Date(milliseconds).toISOString();
    }
  }
  return new Date().toISOString();
}

function gitBranch() {
  const supplied = process.env.GIT_BRANCH ?? process.env.GITHUB_REF_NAME ?? process.env.CI_COMMIT_REF_NAME;
  if (supplied?.trim()) return supplied.trim();
  try {
    const branch = execSync("git rev-parse --abbrev-ref HEAD", { encoding: "utf8" }).trim();
    return branch && branch !== "HEAD" ? branch : "unknown";
  } catch {
    return "unknown";
  }
}

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@tauri-apps/api/app": fileURLToPath(new URL("./browser/browser/app.ts", import.meta.url)),
      "@tauri-apps/api/core": fileURLToPath(new URL("./browser/browser/core.ts", import.meta.url)),
      "@tauri-apps/api/dpi": fileURLToPath(new URL("./browser/browser/dpi.ts", import.meta.url)),
      "@tauri-apps/api/event": fileURLToPath(new URL("./browser/browser/event.ts", import.meta.url)),
      "@tauri-apps/api/menu": fileURLToPath(new URL("./browser/browser/menu.ts", import.meta.url)),
      "@tauri-apps/api/webview": fileURLToPath(new URL("./browser/browser/webview.ts", import.meta.url)),
      "@tauri-apps/api/window": fileURLToPath(new URL("./browser/browser/window.ts", import.meta.url)),
      "@tauri-apps/plugin-dialog": fileURLToPath(new URL("./browser/browser/dialog.ts", import.meta.url)),
      "@tauri-apps/plugin-notification": fileURLToPath(new URL("./browser/browser/notification.ts", import.meta.url)),
      "@tauri-apps/plugin-opener": fileURLToPath(new URL("./browser/browser/opener.ts", import.meta.url)),
      "@tauri-apps/plugin-process": fileURLToPath(new URL("./browser/browser/process.ts", import.meta.url)),
      "@tauri-apps/plugin-updater": fileURLToPath(new URL("./browser/browser/updater.ts", import.meta.url)),
      "tauri-plugin-liquid-glass-api": fileURLToPath(new URL("./browser/browser/liquidGlass.ts", import.meta.url)),
      "@": fileURLToPath(new URL("./src", import.meta.url)),
      "@app": fileURLToPath(new URL("./src/features/app", import.meta.url)),
      "@settings": fileURLToPath(new URL("./src/features/settings", import.meta.url)),
      "@threads": fileURLToPath(new URL("./src/features/threads", import.meta.url)),
      "@services": fileURLToPath(new URL("./src/services", import.meta.url)),
      "@utils": fileURLToPath(new URL("./src/utils", import.meta.url)),
    },
  },
  worker: {
    format: "es",
  },
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
    __APP_COMMIT_HASH__: JSON.stringify(commitHash()),
    __APP_BUILD_DATE__: JSON.stringify(buildDate()),
    __APP_GIT_BRANCH__: JSON.stringify(gitBranch()),
  },
  server: {
    host: process.env.OPEN_WEB_CODEX_FRONTEND_HOST ?? "127.0.0.1",
    port: Number(process.env.OPEN_WEB_CODEX_FRONTEND_PORT ?? 1420),
    strictPort: true,
    proxy: {
      "/api": {
        target: process.env.OPEN_WEB_CODEX_DEV_SERVER ?? "http://127.0.0.1:4800",
        ws: true,
      },
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx", "browser/**/*.test.ts", "browser/**/*.test.tsx"],
    setupFiles: ["src/test/vitest.setup.ts"],
  },
});
