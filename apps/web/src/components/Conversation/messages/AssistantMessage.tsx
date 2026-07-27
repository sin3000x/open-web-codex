import { useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  indexInlineVisualizationArtifacts,
  segmentInlineVisualizations,
} from "../../../utils/inlineVisualizations";
import type { InlineVisualizationArtifact } from "../../../utils/replyCards";
import ReplyCard from "./ReplyCard";

type Props = {
  text: string;
  streaming?: boolean;
  onOpenFile?: (path: string) => void;
  variant?: "reply" | "commentary";
  inlineArtifacts?: InlineVisualizationArtifact[];
};

function MarkdownText({ text, onOpenFile }: { text: string; onOpenFile?: Props["onOpenFile"] }) {
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
      a: ({ href, children, ...props }) => {
        const external = Boolean(href && /^(?:https?:|mailto:)/i.test(href));
        const navigational = external || Boolean(href?.startsWith("#"));
        return <a
          href={href}
          {...props}
          className={external ? "web-external-link" : undefined}
          target={external ? "_blank" : undefined}
          rel={external ? "noopener noreferrer" : undefined}
          onClick={(event) => {
            if (!navigational && href && onOpenFile) {
              event.preventDefault();
              const decoded = decodeURIComponent(href);
              const path = decoded.startsWith("file://") ? new URL(decoded).pathname : decoded;
              onOpenFile(path.replace(/^\.\//, ""));
            }
          }}
        >{children}</a>;
      },
    }}>{text}</ReactMarkdown>
  );
}

export default function AssistantMessage({
  text,
  streaming = false,
  onOpenFile,
  variant = "reply",
  inlineArtifacts,
}: Props) {
  const commentary = variant === "commentary";
  const segments = useMemo(
    () => segmentInlineVisualizations(text, streaming),
    [streaming, text],
  );
  const artifactIndex = useMemo(
    () => indexInlineVisualizationArtifacts(inlineArtifacts),
    [inlineArtifacts],
  );
  const hasVisualization = segments.some((segment) => segment.kind !== "markdown");
  return (
    <div
      className={[
        "web-msg-assistant",
        commentary ? "web-msg-commentary" : "",
        hasVisualization ? "has-inline-visualization" : "",
      ].filter(Boolean).join(" ")}
    >
      <div className={`web-msg-assistant-body${commentary ? " web-msg-commentary-body" : ""}`}>
        {segments.map((segment, index) => {
          if (segment.kind === "markdown") {
            return (
              <MarkdownText
                key={`markdown-${index}`}
                text={segment.text}
                onOpenFile={onOpenFile}
              />
            );
          }
          if (segment.kind === "artifact") {
            const artifact = artifactIndex.get(segment.ref);
            if (artifact) {
              return (
                <ReplyCard
                  key={`artifact-${segment.ref}-${index}`}
                  card={artifact.card}
                />
              );
            }
            if (streaming) return null;
            return (
              <div
                className="web-inline-visualization-unavailable"
                role="status"
                key={`artifact-unavailable-${segment.ref}-${index}`}
              >
                Visualization unavailable
              </div>
            );
          }
          return (
            <div
              className="web-inline-visualization-unavailable"
              role="status"
              key={`visualization-unavailable-${index}`}
            >
              {segment.kind === "file"
                ? `HTML visualization “${segment.file}” is unavailable in Web.`
                : segment.label}
            </div>
          );
        })}
      </div>
    </div>
  );
}
