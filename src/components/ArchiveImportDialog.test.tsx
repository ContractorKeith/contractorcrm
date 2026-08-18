import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ArchiveImportPreview } from "../api/types";
import { stubClient } from "../test/stub-client";
import { ArchiveImportDialog } from "./ArchiveImportDialog";

const preview = (overrides: Partial<ArchiveImportPreview> = {}): ArchiveImportPreview => ({
  schemaVersion: 1,
  product: { name: "ContractorCRM", version: "0.1.0" },
  exportedAt: "2026-08-18T15:30:00Z",
  databaseMigrationVersion: 8,
  recordCounts: { contacts: 12, companies: 3, tasks: 5 },
  issues: [],
  ...overrides,
});

describe("ArchiveImportDialog", () => {
  it("summarizes the manifest and the records the archive carries", async () => {
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue(preview()),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={vi.fn()} />);

    expect(await screen.findByText("ContractorCRM 0.1.0")).toBeVisible();
    expect(client.previewArchiveImport).toHaveBeenCalledWith("/tmp/crm.zip");
    expect(screen.getByText("Version 1 · database 8")).toBeVisible();
    const counts = screen.getByRole("table", { name: "Records in this archive" });
    expect(within(counts).getByRole("rowheader", { name: "Contacts" })).toBeVisible();
    expect(within(counts).getByRole("rowheader", { name: "Total" })).toBeVisible();
    expect(within(counts).getByText("20")).toBeVisible();
  });

  it("warns that importing replaces everything and requires a deliberate confirm", async () => {
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue(preview()),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={vi.fn()} />);

    const dialog = await screen.findByRole("dialog", { name: "Import portable archive" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(screen.getByText(/Importing replaces all current CRM data/)).toBeVisible();
    expect(screen.getByText(/safety backup .* is created automatically/i)).toBeVisible();
    expect(
      await screen.findByRole("button", { name: "Replace all data and import" }),
    ).toBeEnabled();
  });

  it("reports the imported counts and the safety backup, then closes as imported", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue(preview()),
      importArchive: vi.fn().mockResolvedValue({
        recordCounts: { contacts: 12, companies: 3, tasks: 5 },
        safetyBackupPath: "/backups/pre-import-2026-08-18.sqlite3",
      }),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={onClose} />);
    await user.click(await screen.findByRole("button", { name: "Replace all data and import" }));

    expect(client.importArchive).toHaveBeenCalledWith("/tmp/crm.zip");
    expect(await screen.findByText("Archive imported — 20 records restored.")).toBeVisible();
    expect(
      screen.getByText(/Safety backup of your previous data: \/backups\/pre-import/),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(onClose).toHaveBeenCalledWith(true);
  });

  it("lists the problems and blocks the import when the archive is invalid", async () => {
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue(
        preview({
          issues: [
            { code: "checksum_mismatch", message: '"data/contacts.json" does not match its checksum' },
            { code: "missing_table_file", message: "archive has no data/tasks.json" },
          ],
        }),
      ),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={vi.fn()} />);

    expect(await screen.findByText("This archive can't be imported.")).toBeVisible();
    const problems = screen.getByRole("list", { name: "Archive problems" });
    expect(within(problems).getAllByRole("listitem")).toHaveLength(2);
    expect(within(problems).getByText("archive has no data/tasks.json")).toBeVisible();
    expect(screen.getByRole("button", { name: "Replace all data and import" })).toBeDisabled();
    expect(screen.queryByText(/Importing replaces all current CRM data/)).not.toBeInTheDocument();
  });

  it("surfaces the core's message when the archive cannot be read at all", async () => {
    const client = stubClient({
      previewArchiveImport: vi.fn().mockRejectedValue({
        kind: "invalid_input",
        message: 'cannot read archive "/tmp/crm.zip": no manifest.json',
        field: "path",
      }),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={vi.fn()} />);

    expect(await screen.findByRole("alert")).toHaveTextContent("no manifest.json");
    expect(screen.getByRole("button", { name: "Replace all data and import" })).toBeDisabled();
  });

  it("closes without importing on Escape", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue(preview()),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={onClose} />);
    expect(await screen.findByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.keyboard("{Escape}");

    expect(onClose).toHaveBeenCalledWith(false);
    expect(client.importArchive).not.toHaveBeenCalled();
  });
});
