import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ArchiveImportPreview, ArchiveImportReport } from "../api/types";
import { stubClient } from "../test/stub-client";
import { ArchiveImportDialog } from "./ArchiveImportDialog";

const preview = (overrides: Partial<ArchiveImportPreview> = {}): ArchiveImportPreview => ({
  schemaVersion: 1,
  product: { name: "ContractorCRM", version: "0.1.0" },
  exportedAt: "2026-08-18T15:30:00Z",
  databaseMigrationVersion: 9,
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
    expect(screen.getByText("Version 1 · database 9")).toBeVisible();
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

  it("caps a flood of problems and counts the rest", async () => {
    const issues = Array.from({ length: 80 }, (_index, number) => ({
      code: "invalid_value",
      message: `contacts row ${number} field "kind" must be text`,
    }));
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue(preview({ issues })),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={vi.fn()} />);

    const problems = await screen.findByRole("list", { name: "Archive problems" });
    expect(within(problems).getAllByRole("listitem")).toHaveLength(50);
    expect(screen.getByText("…and 30 more issues.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Replace all data and import" })).toBeDisabled();
  });

  it("stays open while the core is replacing the database", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    let finishImport = (_report: ArchiveImportReport) => {};
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue(preview()),
      importArchive: vi.fn().mockReturnValue(
        new Promise<ArchiveImportReport>((resolve) => {
          finishImport = resolve;
        }),
      ),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={onClose} />);
    await user.click(await screen.findByRole("button", { name: "Replace all data and import" }));

    // Escape and Cancel are both inert until the import reports back.
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Import portable archive" })).toBeVisible();

    finishImport({
      recordCounts: { contacts: 12 },
      safetyBackupPath: "/backups/pre-import.sqlite3",
    });

    expect(await screen.findByText("Archive imported — 12 records restored.")).toBeVisible();
    expect(screen.getByText(/\/backups\/pre-import\.sqlite3/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Done" }));
    expect(onClose).toHaveBeenCalledWith(true);
  });

  it("announces verifying and importing progress and marks the dialog busy", async () => {
    const user = userEvent.setup();
    let finishPreview = (_preview: ArchiveImportPreview) => {};
    let finishImport = (_report: ArchiveImportReport) => {};
    const client = stubClient({
      previewArchiveImport: vi.fn().mockReturnValue(
        new Promise<ArchiveImportPreview>((resolve) => {
          finishPreview = resolve;
        }),
      ),
      importArchive: vi.fn().mockReturnValue(
        new Promise<ArchiveImportReport>((resolve) => {
          finishImport = resolve;
        }),
      ),
    });

    render(<ArchiveImportDialog client={client} path="/tmp/crm.zip" onClose={vi.fn()} />);

    // Verifying: busy, announced, and the destructive action is unavailable.
    const dialog = screen.getByRole("dialog", { name: "Import portable archive" });
    expect(dialog).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("status")).toHaveTextContent("Verifying archive…");
    expect(screen.getByRole("button", { name: "Replace all data and import" })).toBeDisabled();

    finishPreview(preview());
    expect(
      await screen.findByRole("button", { name: "Replace all data and import" }),
    ).toBeEnabled();
    expect(dialog).toHaveAttribute("aria-busy", "false");
    expect(screen.queryByRole("status")).not.toBeInTheDocument();

    // Importing: busy again, with its own message, and both buttons inert.
    await user.click(screen.getByRole("button", { name: "Replace all data and import" }));
    expect(dialog).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("status")).toHaveTextContent(/Replacing your data/);
    expect(screen.getByRole("button", { name: "Replace all data and import" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();

    finishImport({ recordCounts: { contacts: 12 }, safetyBackupPath: "/backups/pre.sqlite3" });
    expect(await screen.findByText("Archive imported — 12 records restored.")).toBeVisible();
    expect(dialog).toHaveAttribute("aria-busy", "false");
    expect(screen.getByRole("status")).toHaveTextContent("Archive imported — 12 records restored.");
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
