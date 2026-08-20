import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RecordTable, VIRTUALIZE_ABOVE, type ColumnDef } from "./RecordTable";

interface Row {
  id: string;
  name: string;
  city: string;
}

const columns: ColumnDef<Row>[] = [
  { key: "name", header: "Name", render: (row) => row.name, sortable: true },
  { key: "city", header: "City", render: (row) => row.city },
];

function makeRows(count: number): Row[] {
  return Array.from({ length: count }, (_, index) => ({
    id: `r${index}`,
    name: `Contact ${index}`,
    city: "Orlando",
  }));
}

// jsdom has no layout, so the windowed table would measure a zero-height
// viewport (the windowing reads offsetWidth/offsetHeight). Give the scroll pane
// a realistic size for the duration of a test.
function giveTheScrollPaneHeight(height = 560) {
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return this.classList.contains("record-table-scroll") ? height : 0;
  });
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (
    this: HTMLElement,
  ) {
    return this.classList.contains("record-table-scroll") ? 900 : 0;
  });
}

// jsdom never really scrolls, so set the offset the windowing reads and fire
// the event it listens for.
function scrollThePaneTo(pane: HTMLElement, top: number) {
  Object.defineProperty(pane, "scrollTop", { value: top, writable: true, configurable: true });
  fireEvent.scroll(pane);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("RecordTable", () => {
  it("renders every row and no scroll pane for an ordinary list", () => {
    const { container } = render(
      <RecordTable label="Contact list" columns={columns} rows={makeRows(120)} onOpen={() => {}} />,
    );

    const table = screen.getByRole("table", { name: "Contact list" });
    // Header row plus every data row.
    expect(within(table).getAllByRole("row")).toHaveLength(121);
    expect(container.querySelector(".record-table-scroll")).toBeNull();
  });

  it("mounts only the visible window of a very long list", () => {
    giveTheScrollPaneHeight();
    const { container } = render(
      <RecordTable
        label="Contact list"
        columns={columns}
        rows={makeRows(10_000)}
        onOpen={() => {}}
      />,
    );

    const table = screen.getByRole("table", { name: "Contact list" });
    const mounted = within(table).getAllByRole("row");
    expect(container.querySelector(".record-table-scroll")).not.toBeNull();
    // A 560px viewport of 28px rows plus overscan — nowhere near 10,000.
    expect(mounted.length).toBeGreaterThan(1);
    expect(mounted.length).toBeLessThan(100);
    expect(table.querySelectorAll("tbody tr[aria-hidden='true']").length).toBeGreaterThan(0);
  });

  it("publishes the true row count and row positions while windowed", () => {
    giveTheScrollPaneHeight();
    render(
      <RecordTable
        label="Contact list"
        columns={columns}
        rows={makeRows(10_000)}
        onOpen={() => {}}
      />,
    );

    const table = screen.getByRole("table", { name: "Contact list" });
    expect(table).toHaveAttribute("aria-rowcount", "10000");
    expect(within(table).getByText(/10000 rows\./)).toBeInTheDocument();
    expect(within(table).getByText(/arrow keys to move between rows and Enter to open/)).toBeInTheDocument();
    // Header is row 1, so the first data row is row 2.
    const firstDataRow = within(table).getByText("Contact 0").closest("tr")!;
    expect(firstDataRow).toHaveAttribute("aria-rowindex", "2");
  });

  it("keeps the roving keyboard model in a windowed list", async () => {
    giveTheScrollPaneHeight();
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <RecordTable
        label="Contact list"
        columns={columns}
        rows={makeRows(10_000)}
        onOpen={onOpen}
      />,
    );

    const table = screen.getByRole("table", { name: "Contact list" });
    const first = within(table).getByText("Contact 0").closest("tr")!;
    // Only the selected row is in the tab order (roving tabindex).
    expect(first).toHaveAttribute("tabindex", "0");
    expect(within(table).getByText("Contact 1").closest("tr")).toHaveAttribute("tabindex", "-1");

    first.focus();
    await user.keyboard("{ArrowDown}");
    expect(within(table).getByText("Contact 1").closest("tr")).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(within(table).getByText("Contact 2").closest("tr")).toHaveFocus();

    await user.keyboard("{ArrowUp}");
    expect(within(table).getByText("Contact 1").closest("tr")).toHaveFocus();

    await user.keyboard("{Home}");
    expect(within(table).getByText("Contact 0").closest("tr")).toHaveFocus();

    await user.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: "r0" }));
  });

  it("stays keyboard-reachable when the selected row scrolls out of the window", async () => {
    giveTheScrollPaneHeight();
    const user = userEvent.setup();
    const { container } = render(
      <RecordTable
        label="Contact list"
        columns={columns}
        rows={makeRows(10_000)}
        onOpen={() => {}}
      />,
    );

    const table = screen.getByRole("table", { name: "Contact list" });
    const pane = container.querySelector<HTMLDivElement>(".record-table-scroll")!;
    // Scroll far past the selected first row so it unmounts.
    scrollThePaneTo(pane, 20_000);
    await waitFor(() => {
      expect(within(table).queryByText("Contact 0")).toBeNull();
    });

    // Nothing inside the table is selected any more, but the pane itself and
    // the first mounted row both remain in the tab order.
    expect(pane).toHaveAttribute("tabindex", "0");
    const firstMounted = within(table)
      .getAllByRole("row")
      .find((row) => row.getAttribute("tabindex") === "0");
    expect(firstMounted).toBeDefined();

    await user.tab();
    expect(pane).toHaveFocus();
    await user.tab();
    expect(firstMounted).toHaveFocus();
    const firstIndex = Number(firstMounted!.getAttribute("aria-rowindex")) - 2;

    // Arrow keys still change the selection from the row...
    await user.keyboard("{ArrowDown}");
    expect(within(table).getByText(`Contact ${firstIndex + 1}`).closest("tr")).toHaveFocus();

    // ...and from the scroll pane itself.
    await user.tab({ shift: true });
    expect(pane).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(within(table).getByText(`Contact ${firstIndex + 2}`).closest("tr")).toHaveFocus();
  });

  it("still reports sort state on a windowed list", async () => {
    giveTheScrollPaneHeight();
    const user = userEvent.setup();
    const onSort = vi.fn();
    render(
      <RecordTable
        label="Contact list"
        columns={columns}
        rows={makeRows(VIRTUALIZE_ABOVE + 1)}
        onOpen={() => {}}
        sort={{ key: "name", direction: "descending" }}
        onSort={onSort}
      />,
    );

    const header = screen.getByRole("columnheader", { name: /Name/ });
    expect(header).toHaveAttribute("aria-sort", "descending");
    await user.click(within(header).getByRole("button"));
    expect(onSort).toHaveBeenCalledWith("name");
  });
});
