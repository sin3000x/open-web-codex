import type { InlineVisualizationArtifact } from "./replyCards";

const DIRECTIVE_PREFIX = "::codex-inline-vis{";
const ARTIFACT_PREFIX = `${DIRECTIVE_PREFIX}artifact="`;
const FILE_PREFIX = `${DIRECTIVE_PREFIX}file="`;
const SAFE_REFERENCE = /^[A-Za-z0-9_.-]{1,128}$/;
const SAFE_HTML_FILE = /^[A-Za-z0-9_.-]{1,123}\.html$/;

export type InlineVisualizationSegment =
  | { kind: "markdown"; text: string }
  | { kind: "artifact"; ref: string }
  | { kind: "file"; file: string }
  | { kind: "unavailable"; label: string };

function directiveValue(
  directive: string,
  prefix: string,
  pattern: RegExp,
): string | null {
  const value = directive.startsWith(prefix) && directive.endsWith('"}')
    ? directive.slice(prefix.length, -2)
    : "";
  return value && pattern.test(value) ? value : null;
}

function fenceMarker(line: string): { character: string; length: number } | null {
  const leadingSpaces = line.match(/^ */)?.[0].length ?? 0;
  if (leadingSpaces > 3) return null;
  const trimmed = line.slice(leadingSpaces);
  const character = trimmed[0];
  if (character !== "`" && character !== "~") return null;
  const length = [...trimmed].findIndex((value) => value !== character);
  const markerLength = length < 0 ? trimmed.length : length;
  return markerLength >= 3 ? { character, length: markerLength } : null;
}

export function segmentInlineVisualizations(
  markdown: string,
  streaming = false,
): InlineVisualizationSegment[] {
  if (!markdown.includes(DIRECTIVE_PREFIX)) {
    return markdown ? [{ kind: "markdown", text: markdown }] : [];
  }

  const segments: InlineVisualizationSegment[] = [];
  let markdownStart = 0;
  let lineStart = 0;
  let fence: { character: string; length: number } | null = null;

  while (lineStart < markdown.length) {
    const newlineIndex = markdown.indexOf("\n", lineStart);
    const lineEnd = newlineIndex < 0 ? markdown.length : newlineIndex + 1;
    const line = markdown
      .slice(lineStart, newlineIndex < 0 ? markdown.length : newlineIndex)
      .replace(/\r$/, "");
    const marker = fenceMarker(line);
    if (marker) {
      if (!fence) {
        fence = marker;
      } else if (
        fence.character === marker.character
        && marker.length >= fence.length
      ) {
        fence = null;
      }
      lineStart = lineEnd;
      continue;
    }

    const leadingSpaces = line.match(/^ */)?.[0].length ?? 0;
    const inIndentedCode = leadingSpaces >= 4 || line.startsWith("\t");
    const directive = line.trim();
    if (!fence && !inIndentedCode && directive.startsWith(DIRECTIVE_PREFIX)) {
      const artifact = directiveValue(directive, ARTIFACT_PREFIX, SAFE_REFERENCE);
      const file = directiveValue(directive, FILE_PREFIX, SAFE_HTML_FILE);
      const incomplete = streaming
        && newlineIndex < 0
        && !directive.endsWith("}");
      if (artifact || file || directive.endsWith("}") || incomplete) {
        if (lineStart > markdownStart) {
          segments.push({
            kind: "markdown",
            text: markdown.slice(markdownStart, lineStart),
          });
        }
        if (artifact) {
          segments.push({ kind: "artifact", ref: artifact });
        } else if (file) {
          segments.push({ kind: "file", file });
        } else if (!incomplete) {
          segments.push({ kind: "unavailable", label: "Visualization unavailable" });
        }
        markdownStart = lineEnd;
      }
    }
    lineStart = lineEnd;
  }

  if (markdownStart < markdown.length) {
    segments.push({ kind: "markdown", text: markdown.slice(markdownStart) });
  }
  return segments.filter((segment) => segment.kind !== "markdown" || segment.text);
}

export function indexInlineVisualizationArtifacts(
  artifacts: InlineVisualizationArtifact[] | undefined,
): ReadonlyMap<string, InlineVisualizationArtifact> {
  return new Map((artifacts ?? []).map((artifact) => [artifact.ref, artifact]));
}
