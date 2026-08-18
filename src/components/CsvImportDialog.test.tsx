import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ContactImportPreview } from "../api/types";
import { stubClient } from "../test/stub-client";
import { CsvImportDialog } from "./CsvImportDialog";

const preview = (overrides: Partial<ContactImportPreview> = {}): ContactImportPreview => ({
  headers: ["Name", "Phone", "Crew"],
  rowCount: 2,
  mapping: { displayName: "Name", phone: "Phone" },
  sampleRows: [
    ["Dana Ruiz", "555-0134", "North"],
    ["Marco Bell", "555-0199", "South"],
  ],
  issues: [],
  ...overrides,
});

describe("CsvImportDialog", () => {
  it("previews the picked file with the auto-guessed mapping", async () => {
    const client = stubClient({
      previewContactImport: vi.fn().mockResolvedValue(preview()),
    });

    render(<CsvImportDialog client={client} path="/tmp/leads.csv" onClose={vi.fn()} />);

    expect(await screen.findByText("2 rows found.")).toBeVisible();
    expect(client.previewContactImport).toHaveBeenCalledWith("/tmp/leads.csv", null);
    expect(screen.getByRole("combobox", { name: "Map column Name" })).toHaveValue("displayName");
    expect(screen.getByRole("combobox", { name: "Map column Phone" })).toHaveValue("phone");
    expect(screen.getByRole("combobox", { name: "Map column Crew" })).toHaveValue("");
    const sample = screen.getByRole("table", { name: "Sample rows" });
    expect(within(sample).getByText("Dana Ruiz")).toBeVisible();
    expect(within(sample).getByText("555-0199")).toBeVisible();
  });

  it("re-previews with an explicit mapping when a column is remapped", async () => {
    const user = userEvent.setup();
    const previewContactImport = vi
      .fn()
      .mockResolvedValueOnce(preview())
      .mockResolvedValueOnce(
        preview({ mapping: { displayName: "Name", phone: "Phone", notes: "Crew" } }),
      );
    const client = stubClient({ previewContactImport });

    render(<CsvImportDialog client={client} path="/tmp/leads.csv" onClose={vi.fn()} />);
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Map column Crew" }),
      "notes",
    );

    expect(previewContactImport).toHaveBeenNthCalledWith(2, "/tmp/leads.csv", {
      displayName: "Name",
      phone: "Phone",
      notes: "Crew",
    });
    expect(await screen.findByRole("combobox", { name: "Map column Crew" })).toHaveValue("notes");
  });

  it("shows the import summary, lists skipped rows, and reports the result on close", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const client = stubClient({
      previewContactImport: vi.fn().mockResolvedValue(preview()),
      importContacts: vi.fn().mockResolvedValue({
        created: 4,
        updated: 1,
        skipped: [{ line: 7, reason: "display name is required" }],
      }),
    });

    render(<CsvImportDialog client={client} path="/tmp/leads.csv" onClose={onClose} />);
    await user.click(await screen.findByRole("button", { name: "Import" }));

    expect(client.importContacts).toHaveBeenCalledWith({
      path: "/tmp/leads.csv",
      mapping: { displayName: "Name", phone: "Phone" },
    });
    expect(await screen.findByRole("status")).toHaveTextContent("4 created, 1 updated, 1 skipped.");
    const skipped = screen.getByRole("list", { name: "Skipped rows" });
    expect(within(skipped).getByText("Line 7: display name is required")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(onClose).toHaveBeenCalledWith(true);
  });

  it("surfaces preview validation issues before anything is written", async () => {
    const client = stubClient({
      previewContactImport: vi
        .fn()
        .mockResolvedValue(preview({ issues: [{ line: 3, reason: "kind is not recognized" }] })),
    });

    render(<CsvImportDialog client={client} path="/tmp/leads.csv" onClose={vi.fn()} />);
    const issues = await screen.findByRole("list", { name: "Import issues" });
    expect(within(issues).getByText("Line 3: kind is not recognized")).toBeVisible();
    expect(client.importContacts).not.toHaveBeenCalled();
  });

  it("surfaces a rejected preview and keeps the import disabled", async () => {
    const client = stubClient({
      previewContactImport: vi.fn().mockRejectedValue({
        kind: "invalid_input",
        message: "CSV header \"Name\" appears more than once",
        field: "headers",
      }),
    });

    render(<CsvImportDialog client={client} path="/tmp/leads.csv" onClose={vi.fn()} />);

    expect(
      await screen.findByText('CSV header "Name" appears more than once'),
    ).toBeVisible();
    expect(screen.queryByRole("table", { name: "Column mapping" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Import" })).toBeDisabled();
  });

  it("keys mapping rows by column so repeated header text still renders", async () => {
    const client = stubClient({
      previewContactImport: vi.fn().mockResolvedValue(
        preview({
          headers: ["Name", "Name", ""],
          mapping: { displayName: "Name" },
          sampleRows: [["Dana Ruiz", "Ruiz", "x"]],
        }),
      ),
    });

    render(<CsvImportDialog client={client} path="/tmp/leads.csv" onClose={vi.fn()} />);

    const mappingTable = await screen.findByRole("table", { name: "Column mapping" });
    // Header row plus one row per column, duplicates and blanks included.
    expect(within(mappingTable).getAllByRole("row")).toHaveLength(4);
    expect(within(mappingTable).getAllByRole("combobox")).toHaveLength(3);
  });

  it("focuses the dialog on open and closes on Escape without importing", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const client = stubClient({ previewContactImport: vi.fn().mockResolvedValue(preview()) });

    render(<CsvImportDialog client={client} path="/tmp/leads.csv" onClose={onClose} />);
    expect(await screen.findByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledWith(false);
    expect(client.importContacts).not.toHaveBeenCalled();
  });
});
