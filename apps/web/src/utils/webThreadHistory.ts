import type { AgentMessagePhase, LogEntry } from "../WebApp";
import { stripLeadingProviderSentinel } from "./providerText";
import { parseInlineVisualizationArtifact } from "./replyCards";
import { parseApprovalStatus } from "./approvalStatus";

export function unwrapWebRpcResult(value: unknown): unknown {
  let current = value;
  const visited = new Set<object>();

  for (let depth = 0; depth < 8; depth += 1) {
    if (!current || typeof current !== "object") break;
    if (visited.has(current)) break;
    visited.add(current);

    const record = current as Record<string, unknown>;
    if (!("result" in record)) break;
    current = record.result;
  }

  return current;
}

function asText(value: unknown) {
  return typeof value === "string" ? value : "";
}

export function agentMessagePhase(value: unknown): AgentMessagePhase | undefined {
  return value === "commentary" || value === "final_answer" ? value : undefined;
}

export function commandText(value: unknown) {
  return Array.isArray(value)
    ? value.map((part) => String(part)).join(" ")
    : asText(value);
}

export function appendTerminalInteractionOutput(output: string, stdin: string) {
  if (!stdin) return output;
  return `${output}\n[stdin supplied]\n`.slice(-200_000);
}

function messageText(item: Record<string, unknown>) {
  if (asText(item.text)) return asText(item.text);
  if (!Array.isArray(item.content)) return "";
  return item.content
    .map((part) => asText((part as Record<string, unknown>)?.text))
    .filter(Boolean)
    .join("\n");
}

function jsonText(value: unknown) {
  if (value == null || value === "") return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

const SENSITIVE_FIELD = /(?:authorization|cookie|credential|password|secret|token|api[_-]?key|stdin|chars)/i;

function redactSensitive(value: unknown, key = ""): unknown {
  if (SENSITIVE_FIELD.test(key)) return "[redacted]";
  if (Array.isArray(value)) return value.map((entry) => redactSensitive(entry));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value as Record<string, unknown>)
      .map(([entryKey, entryValue]) => [entryKey, redactSensitive(entryValue, entryKey)]));
  }
  return value;
}

function dynamicOutput(item: Record<string, unknown>) {
  if (!Array.isArray(item.contentItems)) return "";
  return item.contentItems
    .map((entry) => {
      const record = entry as Record<string, unknown>;
      if (record.type === "inputText") return asText(record.text);
      if (record.type === "inputImage") return asText(record.imageUrl ?? record.image_url);
      return "";
    })
    .filter(Boolean)
    .join("\n");
}

function parseCommandResult(output: string) {
  const exitMatch = output.match(/(?:Process exited with code|Exit code:)\s*(-?\d+)/i);
  const wallMatch = output.match(/Wall time:\s*([\d.]+)\s*seconds?/i);
  return {
    exitCode: exitMatch ? Number(exitMatch[1]) : undefined,
    durationMs: wallMatch ? Math.round(Number(wallMatch[1]) * 1000) : undefined,
  };
}

function dynamicCommand(item: Record<string, unknown>) {
  const args = item.arguments;
  if (args && typeof args === "object" && typeof (args as Record<string, unknown>).cmd === "string") {
    return (args as Record<string, unknown>).cmd as string;
  }
  if (typeof args !== "string") return "";
  try {
    const parsed = JSON.parse(args) as Record<string, unknown>;
    return typeof parsed.cmd === "string" ? parsed.cmd : args;
  } catch {
    return args;
  }
}

function diffLines(value: string) {
  return value
    .split("\n")
    .filter((line) => line && !line.startsWith("*** "))
    .map((line) => line.startsWith("+")
      ? { type: "add" as const, text: line.slice(1) }
      : line.startsWith("-")
        ? { type: "del" as const, text: line.slice(1) }
        : { type: "ctx" as const, text: line })
    .slice(0, 500);
}

export function isUserThreadItem(item: Record<string, unknown>): boolean {
  return item.type === "userMessage" || item.role === "user";
}

export function mergeWebThreadHistory(history: LogEntry[], live: LogEntry[]): LogEntry[] {
  const merged = [...history];
  for (const entry of live) {
    const stableIndex = merged.findIndex((candidate) =>
      candidate.id === entry.id
      || (
        candidate.kind === "approval"
        && entry.kind === "approval"
        && candidate.approvalRequestId !== undefined
        && candidate.approvalRequestId === entry.approvalRequestId
      ));
    if (stableIndex >= 0) {
      const historical = merged[stableIndex];
      const liveToolIsActive = entry.toolStatus === "inProgress" || entry.toolStatus === "running";
      const historicalToolIsTerminal = historical.toolStatus != null
        && historical.toolStatus !== "inProgress"
        && historical.toolStatus !== "running";
      if (liveToolIsActive && historicalToolIsTerminal) {
        continue;
      }
      const historicalApprovalIsTerminal = historical.kind === "approval"
        && historical.approvalStatus !== undefined
        && historical.approvalStatus !== "pending";
      merged[stableIndex] = {
        ...historical,
        ...entry,
        id: historical.id,
        messagePhase: entry.messagePhase ?? historical.messagePhase,
        inlineArtifacts: entry.inlineArtifacts ?? historical.inlineArtifacts,
        approvalStatus: historicalApprovalIsTerminal && entry.approvalStatus === "pending"
          ? historical.approvalStatus
          : entry.approvalStatus ?? historical.approvalStatus,
      };
      continue;
    }
    const lastMerged = merged[merged.length - 1];
    const echoedUserIndex = entry.level === "user" && lastMerged?.level === "user"
      && lastMerged.text === entry.text
      ? merged.length - 1
      : -1;
    if (echoedUserIndex >= 0) continue;
    merged.push(entry);
  }
  return merged.slice(-200);
}

export function webLogEntryFromThreadItem(
  item: Record<string, unknown>,
  createId: () => string,
): LogEntry | null {
  const type = asText(item.type);
  const id = asText(item.id) || createId();
  if (type === "commandExecution") {
    const command = commandText(item.command);
    if (!command) return null;
    return {
      id,
      level: "info",
      text: command,
      kind: "command_exec",
      toolStatus: asText(item.status) || undefined,
      cmdOutput: asText(item.aggregatedOutput) || undefined,
      cmdExitCode: typeof item.exitCode === "number" ? item.exitCode : undefined,
      cmdDurationMs: typeof item.durationMs === "number" ? item.durationMs : undefined,
      cmdCwd: asText(item.cwd) || undefined,
      cmdActions: Array.isArray(item.commandActions)
        ? item.commandActions.map((action) => {
            const record = action as Record<string, unknown>;
            return { type: asText(record.type), path: asText(record.path) };
          })
        : [],
    };
  }
  if (type === "reasoning") {
    const summary = Array.isArray(item.summary)
      ? item.summary.map(String).join("\n\n")
      : asText(item.summary);
    const content = Array.isArray(item.content) ? item.content.map(String).join("\n\n") : "";
    const genericSummary = /^(reasoning completed|reasoning in progress|reasoning)$/i.test(summary.trim());
    const text = content || summary;
    return text ? {
      id,
      level: "system",
      text,
      kind: "reasoning",
      reasoningSummary: summary && !genericSummary ? summary : undefined,
    } : null;
  }
  const text = messageText(item);
  if (type === "userMessage") return text ? { id, level: "user", text } : null;
  if (type === "agentMessage") {
    const cleanText = stripLeadingProviderSentinel(text);
    const inlineArtifacts = Array.isArray(item.inlineArtifacts)
      ? item.inlineArtifacts.flatMap((value) => {
          const artifact = parseInlineVisualizationArtifact(value);
          return artifact ? [artifact] : [];
        })
      : [];
    return cleanText.trim() ? {
      id,
      level: "assistant",
      text: cleanText,
      messagePhase: agentMessagePhase(item.phase),
      inlineArtifacts: inlineArtifacts.length ? inlineArtifacts : undefined,
    } : null;
  }
  if (type === "platformApproval") {
    const approvalStatus = parseApprovalStatus(item.approvalStatus);
    return {
      id,
      level: "info",
      text: asText(item.text) || "Approval requested",
      kind: "approval",
      approvalRequestId: asText(item.approvalRequestId) || undefined,
      approvalStatus: approvalStatus ?? "resolved",
      approvalMode: asText(item.approvalMode) || undefined,
      approvalUrl: asText(item.approvalUrl) || undefined,
      approvalServerName: asText(item.approvalServerName) || undefined,
      approvalTool: asText(item.approvalTool) || undefined,
    };
  }
  if (type === "hookPrompt") {
    const fragments = Array.isArray(item.fragments)
      ? item.fragments.map((entry) => asText((entry as Record<string, unknown>).text)).filter(Boolean).join("\n")
      : "";
    return fragments ? { id, level: "system", text: fragments, kind: "tool", toolType: "Hook", toolTitle: "Hook prompt", toolStatus: "completed", toolOutput: fragments } : null;
  }
  if (type === "fileChange") {
    const changes = Array.isArray(item.changes) ? item.changes as Record<string, unknown>[] : [];
    const title = changes.length === 1 ? `Updated ${asText(changes[0].path)}` : `Updated ${changes.length} files`;
    const diff = changes.map((change) => asText(change.diff)).filter(Boolean).join("\n");
    return { id, level: "info", text: title, kind: "diff", diffTitle: title, diffLines: diffLines(diff), streaming: asText(item.status) === "inProgress" };
  }
  if (type === "dynamicToolCall") {
    const tool = asText(item.tool) || "Tool";
    const output = dynamicOutput(item);
    const status = asText(item.status) || "completed";
    if (tool === "exec_command") {
      const command = dynamicCommand(item);
      const result = parseCommandResult(output);
      return { id, level: "info", text: command || tool, kind: "command_exec", toolStatus: status, cmdOutput: output || undefined, cmdExitCode: result.exitCode, cmdDurationMs: result.durationMs };
    }
    if (tool === "apply_patch") {
      const patch = typeof item.arguments === "string" ? item.arguments : jsonText(item.arguments);
      return { id, level: "info", text: "File changes", kind: "diff", diffTitle: "File changes", diffLines: diffLines(patch), streaming: status === "inProgress" };
    }
    return { id, level: "info", text: tool, kind: "tool", toolType: "Tool", toolTitle: tool, toolStatus: status, toolDetail: jsonText(redactSensitive(item.arguments)), toolOutput: output };
  }

  const toolStatus = asText(item.status) || "completed";
  if (type === "mcpToolCall") {
    const server = asText(item.server);
    const tool = asText(item.tool);
    return {
      id,
      level: "info",
      text: tool,
      kind: "tool",
      toolType: "MCP",
      toolTitle: [server, tool].filter(Boolean).join(" / "),
      toolStatus,
      toolDetail: jsonText(redactSensitive(item.arguments)),
      toolOutput: jsonText(item.result ?? item.error),
    };
  }
  if (type === "webSearch") return { id, level: "info", text: "Web search", kind: "tool", toolType: "Search", toolTitle: asText(item.query) || "Web search", toolStatus };
  if (type === "plan") return { id, level: "info", text: "Plan", kind: "tool", toolType: "Plan", toolTitle: "Plan updated", toolStatus, toolOutput: asText(item.text) };
  if (type === "imageView") return { id, level: "info", text: "Image view", kind: "tool", toolType: "Image", toolTitle: "Viewed image", toolStatus, filePath: asText(item.path) };
  if (type === "imageGeneration") return { id, level: "info", text: "Image generation", kind: "tool", toolType: "Image", toolTitle: "Generated image", toolStatus, filePath: asText(item.savedPath), toolDetail: asText(item.revisedPrompt) };
  if (type === "sleep") return { id, level: "info", text: "Wait", kind: "tool", toolType: "Wait", toolTitle: `Waited ${Number(item.durationMs ?? 0)}ms`, toolStatus };
  if (type === "contextCompaction") return { id, level: "info", text: "Context compacted", kind: "tool", toolType: "Context", toolTitle: "Context compacted", toolStatus };
  if (type === "collabAgentToolCall") return { id, level: "info", text: "Agent collaboration", kind: "tool", toolType: "Agent", toolTitle: asText(item.tool) || "Agent collaboration", toolStatus, toolDetail: asText(item.prompt), toolOutput: jsonText(item.agentsStates) };
  if (type === "subAgentActivity") return { id, level: "info", text: "Sub-agent activity", kind: "tool", toolType: "Agent", toolTitle: asText(item.kind) || "Sub-agent activity", toolStatus, toolDetail: asText(item.agentPath) };
  if (type === "enteredReviewMode" || type === "exitedReviewMode") return { id, level: "system", text: asText(item.review) || (type === "enteredReviewMode" ? "Entered review mode" : "Exited review mode"), kind: "tool", toolType: "Review", toolTitle: type === "enteredReviewMode" ? "Review started" : "Review completed", toolStatus };
  return { id, level: "info", text: type || "Unknown item", kind: "tool", toolType: type || "Unknown", toolTitle: type || "Unknown item", toolStatus };
}

export function buildWebThreadHistory(
  thread: Record<string, unknown>,
  createId: () => string,
): LogEntry[] {
  const turns = Array.isArray(thread.turns) ? thread.turns : [];
  const entries = turns.flatMap((turn) => {
    const items = Array.isArray((turn as Record<string, unknown>)?.items)
      ? ((turn as Record<string, unknown>).items as Record<string, unknown>[])
      : [];
    const mapped = items.flatMap((item): LogEntry[] => {
      const entry = webLogEntryFromThreadItem(item, createId);
      return entry ? [entry] : [];
    });
    const turnError = (turn as Record<string, unknown>).error;
    if (turnError && typeof turnError === "object") {
      const error = turnError as Record<string, unknown>;
      mapped.push({ id: createId(), level: "error", text: asText(error.message) || "Turn failed." });
    }
    return mapped;
  });

  const status = thread.status;
  const statusType = status && typeof status === "object"
    ? asText((status as Record<string, unknown>).type)
    : "";
  if (statusType === "systemError" && !entries.some((entry) => entry.level === "error")) {
    entries.push({
      id: createId(),
      level: "error",
      text: "System error. The runtime did not provide any additional error details.",
    });
  }

  return entries;
}
