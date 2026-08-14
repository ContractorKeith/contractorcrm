import { useEffect, useRef, useState, type ReactNode } from "react";

// One column of a record table; render returns the cell content for a row.
export interface ColumnDef<T> {
  key: string;
  header: string;
  render: (row: T) => ReactNode;
  numeric?: boolean;
  sortable?: boolean;
}

// Current sort: which column key and which direction.
export interface SortState {
  key: string;
  direction: "ascending" | "descending";
}

interface RecordTableProps<T extends { id: string }> {
  label: string;
  columns: ColumnDef<T>[];
  rows: T[];
  onOpen: (row: T) => void;
  sort?: SortState;
  onSort?: (key: string) => void;
}

// Industry-rhythm table (28px rows, hairline dividers, condensed uppercase
// headers) with a keyboard-first roving selection: arrows move, Enter opens.
// Columns marked sortable render header buttons and report clicks via onSort.
export function RecordTable<T extends { id: string }>({
  label,
  columns,
  rows,
  onOpen,
  sort,
  onSort,
}: RecordTableProps<T>) {
  const [selected, setSelected] = useState(0);
  const rowRefs = useRef(new Map<string, HTMLTableRowElement>());

  // Keep the selection inside the list when rows are filtered or reloaded.
  useEffect(() => {
    setSelected((index) => Math.min(index, Math.max(rows.length - 1, 0)));
  }, [rows]);

  const moveTo = (index: number) => {
    const next = rows[Math.max(0, Math.min(index, rows.length - 1))];
    if (!next) return;
    setSelected(rows.indexOf(next));
    rowRefs.current.get(next.id)?.focus();
  };

  const handleKeyDown = (event: React.KeyboardEvent, index: number) => {
    if (event.key === "ArrowDown") moveTo(index + 1);
    else if (event.key === "ArrowUp") moveTo(index - 1);
    else if (event.key === "Home") moveTo(0);
    else if (event.key === "End") moveTo(rows.length - 1);
    else if (event.key === "Enter") {
      const row = rows[index];
      if (row) onOpen(row);
    } else return;
    event.preventDefault();
  };

  return (
    <table className="record-table" aria-label={label}>
      <thead>
        <tr>
          {columns.map((column) => {
            const sortableHere = Boolean(column.sortable && onSort);
            const sorted = sort?.key === column.key ? sort.direction : undefined;
            return (
              <th
                key={column.key}
                scope="col"
                className={column.numeric ? "is-numeric" : undefined}
                aria-sort={sortableHere ? (sorted ?? "none") : undefined}
              >
                {sortableHere ? (
                  <button type="button" className="sort-header" onClick={() => onSort!(column.key)}>
                    {column.header}
                    <span aria-hidden="true">
                      {sorted === "ascending" ? " ▲" : sorted === "descending" ? " ▼" : ""}
                    </span>
                  </button>
                ) : (
                  column.header
                )}
              </th>
            );
          })}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, index) => (
          <tr
            key={row.id}
            ref={(element) => {
              if (element) rowRefs.current.set(row.id, element);
              else rowRefs.current.delete(row.id);
            }}
            tabIndex={index === selected ? 0 : -1}
            data-selected={index === selected || undefined}
            onFocus={() => setSelected(index)}
            onClick={() => onOpen(row)}
            onKeyDown={(event) => handleKeyDown(event, index)}
          >
            {columns.map((column) => (
              <td key={column.key} className={column.numeric ? "is-numeric" : undefined}>
                {column.render(row)}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
