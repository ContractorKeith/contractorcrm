import { useEffect, useRef, useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

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

// Above this many rows the body is windowed into its own scroll pane; below it
// the whole table is in the DOM and the page scrolls, exactly as before. A
// normal contractor's book never crosses the line, so day-to-day behaviour and
// every existing test are untouched (issue #42 measured 10k rows as the only
// case that needed the machinery).
export const VIRTUALIZE_ABOVE = 150;

// Rows are a fixed 28px by the design system (--row-h), so the windowing math
// needs no per-row measurement.
const ROW_HEIGHT = 28;

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
  const scrollRef = useRef<HTMLDivElement>(null);
  // A row the keyboard asked for that may not be mounted yet; the effect below
  // focuses it once windowing has scrolled it in.
  const [pendingFocus, setPendingFocus] = useState<number | null>(null);

  const virtualized = rows.length > VIRTUALIZE_ABOVE;
  const virtualizer = useVirtualizer({
    // Count 0 keeps the hook inert (hooks cannot be conditional) for the
    // small-list path, where every row is rendered directly.
    count: virtualized ? rows.length : 0,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  // Keep the selection inside the list when rows are filtered or reloaded.
  useEffect(() => {
    setSelected((index) => Math.min(index, Math.max(rows.length - 1, 0)));
  }, [rows]);

  // Focus a row the keyboard moved to. When it is outside the window, scroll it
  // in and wait for the next render — the ref only exists once it is mounted.
  useEffect(() => {
    if (pendingFocus === null) return;
    const row = rows[pendingFocus];
    if (!row) {
      setPendingFocus(null);
      return;
    }
    const element = rowRefs.current.get(row.id);
    if (element) {
      element.focus();
      setPendingFocus(null);
    } else if (virtualized) {
      virtualizer.scrollToIndex(pendingFocus, { align: "auto" });
    } else {
      setPendingFocus(null);
    }
  });

  const moveTo = (index: number) => {
    const target = Math.max(0, Math.min(index, rows.length - 1));
    if (!rows[target]) return;
    setSelected(target);
    setPendingFocus(target);
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

  // One body row — identical markup on both paths, so the roving tabindex,
  // selection highlight, and click/keyboard handlers never diverge.
  const renderRow = (row: T, index: number) => (
    <tr
      key={row.id}
      ref={(element) => {
        if (element) rowRefs.current.set(row.id, element);
        else rowRefs.current.delete(row.id);
      }}
      // Header row is 1, so body rows start at 2.
      aria-rowindex={index + 2}
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
  );

  const windowed = virtualizer.getVirtualItems();
  const paddingTop = windowed.length > 0 ? windowed[0]!.start : 0;
  const paddingBottom =
    windowed.length > 0 ? virtualizer.getTotalSize() - windowed[windowed.length - 1]!.end : 0;

  const table = (
    <table
      className="record-table"
      aria-label={label}
      // Windowed bodies hold only the visible slice, so the true row count and
      // each row's position are published for assistive technology.
      aria-rowcount={rows.length}
    >
      {/* Screen-reader-only caption: the roving selection is not discoverable
          from the markup alone, so the operating instructions live here. */}
      <caption className="sr-only">
        {label} — {rows.length} {rows.length === 1 ? "row" : "rows"}. Use the arrow keys to move
        between rows and Enter to open the highlighted row.
      </caption>
      <thead>
        <tr aria-rowindex={1}>
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
                  <button
                    type="button"
                    className="sort-header"
                    // The column header text is the button's name; aria-sort on the
                    // <th> carries the direction, so no extra label is added here
                    // (it would also rename the column header for assistive tech).
                    onClick={() => onSort!(column.key)}
                  >
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
        {virtualized ? (
          <>
            {/* Spacers stand in for the rows above and below the window; they
                are hidden from assistive tech so only real rows are announced. */}
            {paddingTop > 0 ? (
              <tr aria-hidden="true" style={{ height: paddingTop }}>
                <td colSpan={columns.length} />
              </tr>
            ) : null}
            {windowed.map((item) => {
              const row = rows[item.index];
              return row ? renderRow(row, item.index) : null;
            })}
            {paddingBottom > 0 ? (
              <tr aria-hidden="true" style={{ height: paddingBottom }}>
                <td colSpan={columns.length} />
              </tr>
            ) : null}
          </>
        ) : (
          rows.map((row, index) => renderRow(row, index))
        )}
      </tbody>
    </table>
  );

  // Long lists scroll inside their own pane so the windowing has a viewport to
  // measure; short lists stay in the page flow, unchanged.
  return virtualized ? (
    <div className="record-table-scroll" ref={scrollRef}>
      {table}
    </div>
  ) : (
    table
  );
}
