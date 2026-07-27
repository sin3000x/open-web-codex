import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const repoRoot = resolve(webRoot, "../..");
const sourceRoot = join(webRoot, "src");
const baseline = process.env.UI_PARITY_REF?.trim() || "main";
const uiOverlayRef = process.env.UI_OVERLAY_REF?.trim()
  || "fde0b056104cabfaf4abba40dd45b28d5081f4d6";
const interfaceSeams = new Set([
  "apps/web/src/services/webClient.ts",
  "apps/web/src/services/webClient.test.ts",
]);
// These UI-owned files contain only the reviewed Server-context wiring needed
// to refresh MCP/files/Git when a Thread changes. Pin their complete contents
// so the parity exception cannot silently grow into presentation drift.
const exactIntegrationSeams = new Map([
  ["apps/web/src/WebApp.tsx", "b4d709325aa998d0eab11fd9918a279115d910532636f260cf5ff8c02e0dfe80"],
  ["apps/web/src/components/FileManager/index.tsx", "03a6572d5d777670cc24af634e83a6abb7701f6d093a51d94bf406ed9a28bcda"],
  ["apps/web/src/components/FileManager/index.test.tsx", "769f6fa9c75cf8665e94df325ea067861a45e24c0f3f4ccb1697018445269a52"],
]);
const uiOverlayFiles = new Set([
  "apps/web/src/WebApp.test.tsx",
  "apps/web/src/WebApp.tsx",
  "apps/web/src/components/Conversation/Header.test.tsx",
  "apps/web/src/components/Conversation/Header.tsx",
  "apps/web/src/components/Layout.tsx",
  "apps/web/src/components/Sidebar/Workspaces.test.tsx",
  "apps/web/src/components/Sidebar/Workspaces.tsx",
  "apps/web/src/components/Sidebar/index.test.tsx",
  "apps/web/src/components/Sidebar/index.tsx",
  "apps/web/src/styles/approval-toasts.css",
  "apps/web/src/styles/main.css",
  "apps/web/src/styles/web-refactor.css",
]);

function listWorktreeFiles(root) {
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) files.push(...listWorktreeFiles(path));
    else files.push(path);
  }
  return files;
}

function repositoryPath(path) {
  return `apps/web/${relative(webRoot, path).replaceAll("\\", "/")}`;
}

const baselineRows = execFileSync(
  "git",
  ["ls-tree", "-r", baseline, "--", "apps/web/src"],
  { cwd: repoRoot, encoding: "utf8" },
).trim().split("\n").filter(Boolean);

const baselineFiles = new Map(
  baselineRows.map((row) => {
    const match = row.match(/^(\d+)\s+\w+\s+[0-9a-f]+\t(.+)$/);
    if (!match) throw new Error(`Unable to parse git tree entry: ${row}`);
    return [match[2], match[1]];
  }),
);
for (const path of uiOverlayFiles) {
  const row = execFileSync("git", ["ls-tree", uiOverlayRef, "--", path], {
    cwd: repoRoot,
    encoding: "utf8",
  }).trim();
  const match = row.match(/^(\d+)\s+\w+\s+[0-9a-f]+\t(.+)$/);
  if (!match || match[2] !== path) {
    throw new Error(`Unable to resolve UI overlay ${path} from ${uiOverlayRef}`);
  }
  baselineFiles.set(path, match[1]);
}
const worktreeFiles = new Set(listWorktreeFiles(sourceRoot).map(repositoryPath));
const failures = [];

for (const [path, mode] of baselineFiles) {
  if (interfaceSeams.has(path)) continue;
  if (!worktreeFiles.has(path)) {
    failures.push(`${path}: missing from worktree`);
    continue;
  }
  const absolutePath = join(repoRoot, path);
  const expectedIntegrationHash = exactIntegrationSeams.get(path);
  if (expectedIntegrationHash) {
    const actualHash = createHash("sha256").update(readFileSync(absolutePath)).digest("hex");
    if (actualHash !== expectedIntegrationHash) {
      failures.push(`${path}: differs from its reviewed Server integration seam`);
    }
    continue;
  }
  const stat = lstatSync(absolutePath);
  const worktreeMode = stat.isSymbolicLink() ? "120000" : stat.mode & 0o111 ? "100755" : "100644";
  if (worktreeMode !== mode) {
    const expectedRef = uiOverlayFiles.has(path) ? uiOverlayRef : baseline;
    failures.push(`${path}: mode ${worktreeMode} differs from ${expectedRef} mode ${mode}`);
  }
  const expectedRef = uiOverlayFiles.has(path) ? uiOverlayRef : baseline;
  const expected = execFileSync("git", ["show", `${expectedRef}:${path}`], {
    cwd: repoRoot,
    encoding: "buffer",
  });
  const actual = readFileSync(absolutePath);
  if (!actual.equals(expected)) {
    failures.push(`${path}: content differs byte-for-byte from ${expectedRef}`);
  }
}

for (const path of worktreeFiles) {
  if (interfaceSeams.has(path) || exactIntegrationSeams.has(path)) continue;
  if (!baselineFiles.has(path)) {
    failures.push(`${path}: extra file is not present in ${baseline}`);
  }
}

if (failures.length > 0) {
  console.error(`Web UI source parity check against ${baseline} failed:`);
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log(
  `apps/web/src UI matches ${baseline} plus ${uiOverlayFiles.size} files from ${uiOverlayRef}; `
    + `${interfaceSeams.size} adapter files and ${exactIntegrationSeams.size} exact Server integration seams differ.`,
);
