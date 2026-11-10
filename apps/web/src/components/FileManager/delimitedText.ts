export type DelimitedTextParseResult =
  | { ok: true; rows: string[][]; droppedPartialRow: boolean }
  | { ok: false; reason: string };

const DELIMITED_FILE_DELIMITERS: ReadonlyArray<readonly [RegExp, string]> = [
  [/\.csv$/i, ","],
  [/\.tsv$/i, "\t"],
];

export function delimiterForPath(path: string): string | null {
  for (const [pattern, delimiter] of DELIMITED_FILE_DELIMITERS) {
    if (pattern.test(path)) return delimiter;
  }
  return null;
}

/**
 * Parse RFC 4180 style delimited text (quoted fields, doubled quotes, CRLF or
 * LF row endings). When `truncated` is true the content is a bounded prefix of
 * the file, so the final row is incomplete unless it ended with a newline and
 * is dropped instead of being shown as if it were real data.
 */
export function parseDelimitedText(
  text: string,
  delimiter: string,
  truncated: boolean,
): DelimitedTextParseResult {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;
  let fieldStarted = false;
  let index = text.charCodeAt(0) === 0xfeff ? 1 : 0;

  const endRow = () => {
    row.push(field);
    rows.push(row);
    row = [];
    field = "";
    fieldStarted = false;
  };

  while (index < text.length) {
    const char = text[index];
    if (inQuotes) {
      if (char === '"') {
        if (text[index + 1] === '"') {
          field += '"';
          index += 2;
          continue;
        }
        inQuotes = false;
      } else {
        field += char;
      }
      index += 1;
      continue;
    }
    if (char === '"' && field === "") {
      inQuotes = true;
      fieldStarted = true;
    } else if (char === delimiter) {
      row.push(field);
      field = "";
      fieldStarted = true;
    } else if (char === "\n" || char === "\r") {
      endRow();
      if (char === "\r" && text[index + 1] === "\n") index += 1;
    } else {
      field += char;
      fieldStarted = true;
    }
    index += 1;
  }

  const hasPartialRow = inQuotes || fieldStarted || row.length > 0;
  if (inQuotes && !truncated) {
    return { ok: false, reason: "A quoted field is not closed before the end of the file." };
  }
  if (hasPartialRow) {
    if (truncated) return { ok: true, rows, droppedPartialRow: true };
    endRow();
  }
  return { ok: true, rows, droppedPartialRow: false };
}
