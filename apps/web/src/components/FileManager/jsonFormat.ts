export const MAX_HIGHLIGHT_CHARS = 300_000;

export type JsonFormatResult =
  | { ok: true; text: string }
  | { ok: false; reason: string };

/**
 * Pretty-print JSON for the preview. A truncated read is only a prefix of the
 * file, so it cannot be parsed and is reported instead of being guessed at.
 */
export function formatJsonPreview(content: string, truncated: boolean): JsonFormatResult {
  if (truncated) {
    return { ok: false, reason: "The file is larger than the preview limit, so it is shown unformatted." };
  }
  try {
    return { ok: true, text: JSON.stringify(JSON.parse(content), null, 2) };
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    return { ok: false, reason: `The file is not valid JSON (${detail}), so it is shown unformatted.` };
  }
}
