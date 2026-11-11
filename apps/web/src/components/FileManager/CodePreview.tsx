import { useMemo } from "react";
import { languageFromPath } from "../../utils/languageFromPath";
import { highlightLine } from "../../utils/syntax";
import { MAX_HIGHLIGHT_CHARS, formatJsonPreview } from "./jsonFormat";

type Props = {
  path: string;
  content: string;
  truncated: boolean;
  formatted: boolean;
};

export default function CodePreview({ path, content, truncated, formatted }: Props) {
  const language = languageFromPath(path);
  const json = language === "json" && formatted ? formatJsonPreview(content, truncated) : null;
  const text = json?.ok ? json.text : content;
  const highlightable = Boolean(language) && language !== "text" && text.length <= MAX_HIGHLIGHT_CHARS;
  const html = useMemo(
    () => (highlightable ? highlightLine(text, language) : null),
    [highlightable, language, text],
  );
  const lineCount = useMemo(() => {
    let count = 1;
    for (let index = 0; index < text.length; index += 1) {
      if (text.charCodeAt(index) === 10) count += 1;
    }
    return text.endsWith("\n") ? count - 1 : count;
  }, [text]);
  const gutter = useMemo(
    () => Array.from({ length: lineCount }, (_, index) => index + 1).join("\n"),
    [lineCount],
  );

  return (
    <div className="web-file-code-preview">
      {json && !json.ok ? <div className="web-file-code-note" role="status">{json.reason}</div> : null}
      {!highlightable && language && language !== "text" ? (
        <div className="web-file-code-note" role="status">Syntax highlighting is off for files over {MAX_HIGHLIGHT_CHARS.toLocaleString()} characters.</div>
      ) : null}
      <div className="web-file-code">
        <pre className="web-file-code-gutter" aria-hidden="true">{gutter}</pre>
        {html !== null ? (
          <pre className={`web-file-code-body language-${language}`}>
            <code dangerouslySetInnerHTML={{ __html: html }} />
          </pre>
        ) : (
          <pre className="web-file-code-body"><code>{text}</code></pre>
        )}
      </div>
    </div>
  );
}
