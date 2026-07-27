import type { AppSettings } from "@/types";

const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
const primary = isMac ? "cmd" : "ctrl";

export const defaultAppSettings: AppSettings = {
  codexBin: null,
  codexArgs: null,
  backendMode: "remote",
  remoteBackendProvider: "tcp",
  remoteBackendHost: "",
  remoteBackendToken: null,
  remoteBackends: [],
  activeRemoteBackendId: null,
  keepDaemonRunningAfterAppClose: false,
  defaultAccessMode: "current",
  reviewDeliveryMode: "inline",
  composerModelShortcut: `${primary}+shift+m`,
  composerAccessShortcut: `${primary}+shift+a`,
  composerReasoningShortcut: `${primary}+shift+r`,
  composerCollaborationShortcut: "shift+tab",
  interruptShortcut: isMac ? "ctrl+c" : "ctrl+shift+c",
  newAgentShortcut: `${primary}+n`,
  newWorktreeAgentShortcut: `${primary}+shift+n`,
  newCloneAgentShortcut: `${primary}+alt+n`,
  archiveThreadShortcut: isMac ? "cmd+ctrl+a" : "ctrl+alt+a",
  toggleProjectsSidebarShortcut: `${primary}+shift+p`,
  toggleGitSidebarShortcut: `${primary}+shift+g`,
  branchSwitcherShortcut: `${primary}+shift+b`,
  toggleDebugPanelShortcut: `${primary}+shift+d`,
  toggleTerminalShortcut: `${primary}+shift+t`,
  cycleAgentNextShortcut: isMac ? "cmd+ctrl+down" : "ctrl+alt+down",
  cycleAgentPrevShortcut: isMac ? "cmd+ctrl+up" : "ctrl+alt+up",
  cycleWorkspaceNextShortcut: isMac ? "cmd+shift+down" : "ctrl+alt+shift+down",
  cycleWorkspacePrevShortcut: isMac ? "cmd+shift+up" : "ctrl+alt+shift+up",
  lastComposerModelId: null,
  lastComposerReasoningEffort: null,
  uiScale: 1,
  theme: "system",
  usageShowRemaining: false,
  showMessageFilePath: true,
  chatHistoryScrollbackItems: 200,
  threadTitleAutogenerationEnabled: false,
  automaticAppUpdateChecksEnabled: false,
  uiFontFamily: "system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif",
  codeFontFamily: "ui-monospace, \"Cascadia Mono\", Menlo, Monaco, Consolas, monospace",
  codeFontSize: 11,
  notificationSoundsEnabled: true,
  systemNotificationsEnabled: true,
  subagentSystemNotificationsEnabled: true,
  splitChatDiffView: false,
  preloadGitDiffs: true,
  gitDiffIgnoreWhitespaceChanges: false,
  commitMessagePrompt: "Generate a concise conventional commit message for these changes:\n\n{diff}",
  commitMessageModelId: null,
  collaborationModesEnabled: true,
  steerEnabled: true,
  followUpMessageBehavior: "queue",
  composerFollowUpHintEnabled: true,
  pauseQueuedMessagesWhenResponseRequired: true,
  unifiedExecEnabled: true,
  experimentalAppsEnabled: false,
  personality: "friendly",
  dictationEnabled: false,
  dictationModelId: "base",
  dictationPreferredLanguage: null,
  dictationHoldKey: "alt",
  composerEditorPreset: "default",
  composerFenceExpandOnSpace: false,
  composerFenceExpandOnEnter: false,
  composerFenceLanguageTags: false,
  composerFenceWrapSelection: false,
  composerFenceAutoWrapPasteMultiline: false,
  composerFenceAutoWrapPasteCodeLike: false,
  composerListContinuation: false,
  composerCodeBlockCopyUseModifier: false,
  workspaceGroups: [],
  globalWorktreesFolder: null,
  openAppTargets: [
    { id: "vscode", label: "VS Code", kind: "command", command: "code", args: [] },
    { id: "cursor", label: "Cursor", kind: "command", command: "cursor", args: [] },
    { id: "zed", label: "Zed", kind: "command", command: "zed", args: [] },
    { id: "finder", label: "File Manager", kind: "finder", args: [] },
  ],
  selectedOpenAppId: "vscode",
};

const SETTINGS_KEY = "open-web-codex:app-settings:v1";

export function loadAppSettings(): AppSettings {
  if (typeof localStorage === "undefined") return { ...defaultAppSettings };
  try {
    const stored = JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "null") as Partial<AppSettings> | null;
    return { ...defaultAppSettings, ...(stored ?? {}) };
  } catch {
    return { ...defaultAppSettings };
  }
}

export function saveAppSettings(settings: AppSettings): AppSettings {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  return settings;
}
