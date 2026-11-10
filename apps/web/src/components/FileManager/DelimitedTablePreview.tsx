import { useMemo } from "react";
import { parseDelimitedText } from "./delimitedText";

const MAX_RENDERED_ROWS = 1000;
const NUMERIC_CELL_PATTERN = /^[-+]?(?:\d{1,3}(?:,\d{3})+|\d+)?(?:\.\d+)?(?:[eE][-+]?\d+)?%?$/;

type Props = {
  content: string;
  delimiter: string;
  truncated: boolean;
};

function isNumericColumn(rows: string[][], column: number) {
  let sawValue = false;
  for (const row of rows) {
    const value = (row[column] ?? "").trim();
    if (!value) continue;
    if (!/\d/.test(value) || !NUMERIC_CELL_PATTERN.test(value)) return false;
    sawValue = true;
  }
  return sawValue;
}

export function DelimitedTablePreview({ content, delimiter, truncated }: Props) {
  const parsed = useMemo(
    () => parseDelimitedText(content, delimiter, truncated),
    [content, delimiter, truncated],
  );
  const table = useMemo(() => {
    if (!parsed.ok || parsed.rows.length === 0) return null;
    const [header, ...body] = parsed.rows;
    const rendered = body.slice(0, MAX_RENDERED_ROWS);
    const columnCount = parsed.rows.reduce((max, row) => Math.max(max, row.length), 0);
    const columns = Array.from({ length: columnCount }, (_, column) => ({
      label: header[column] ?? "",
      numeric: isNumericColumn(rendered, column),
    }));
    return { columns, rendered, totalRows: body.length };
  }, [parsed]);

  if (!parsed.ok) {
    return <div className="web-file-error" role="alert">Cannot show this file as a table: {parsed.reason}</div>;
  }
  if (!table) {
    return <div className="web-file-empty">This file has no rows.</div>;
  }

  const { columns, rendered, totalRows } = table;
  const notes: string[] = [];
  if (rendered.length < totalRows) {
    notes.push(`Showing the first ${rendered.length.toLocaleString()} of ${totalRows.toLocaleString()} rows.`);
  }
  if (truncated) {
    notes.push("The file is larger than the preview limit; later rows are not loaded.");
  }

  return (
    <div className="web-file-table-preview">
      <div className="web-file-table-summary">
        {totalRows.toLocaleString()}{truncated ? "+" : ""} {totalRows === 1 && !truncated ? "row" : "rows"} · {columns.length.toLocaleString()} {columns.length === 1 ? "column" : "columns"}
      </div>
      <table className="web-file-table">
        <thead>
          <tr>
            <th className="web-file-table-index" scope="col" aria-label="Row number">#</th>
            {columns.map((column, index) => (
              <th key={index} scope="col" className={column.numeric ? "is-numeric" : undefined} title={column.label}>
                {column.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rendered.map((row, rowIndex) => (
            <tr key={rowIndex}>
              <th className="web-file-table-index" scope="row">{rowIndex + 1}</th>
              {columns.map((column, columnIndex) => {
                const value = row[columnIndex] ?? "";
                return (
                  <td key={columnIndex} className={column.numeric ? "is-numeric" : undefined} title={value.length > 40 ? value : undefined}>
                    {value}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
      {notes.length ? <div className="web-file-table-note">{notes.join(" ")}</div> : null}
    </div>
  );
}
