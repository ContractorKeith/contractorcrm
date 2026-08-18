import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { open, save } from "@tauri-apps/plugin-dialog";

import { App } from "../App";
import { makeContact, stubClient } from "../test/stub-client";

// Native file dialogs only exist inside Tauri, so stand them in for tests.
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

// Open the Backup & Data tab from the shell.
async function openDataView(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Backup & Data" }));
  return screen.findByRole("heading", { name: "Backup & Data" });
}

describe("backup and data view", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.mocked(open).mockReset();
    vi.mocked(save).mockReset();
  });

  it("exports a portable archive to the chosen file and reports what was written", async () => {
    const user = userEvent.setup();
    vi.mocked(save).mockResolvedValue("/tmp/crm.zip");
    const client = stubClient({
      exportArchive: vi.fn().mockResolvedValue({
        path: "/tmp/crm.zip",
        recordCounts: { contacts: 12, companies: 3 },
        fileCount: 19,
      }),
    });

    render(<App client={client} />);
    await openDataView(user);
    await user.click(screen.getByRole("button", { name: "Export archive…" }));

    expect(client.exportArchive).toHaveBeenCalledWith("/tmp/crm.zip", true);
    expect(
      await screen.findByText("Exported 19 files and 15 records to /tmp/crm.zip."),
    ).toBeVisible();
    expect(vi.mocked(save).mock.calls[0]?.[0]?.defaultPath).toMatch(
      /^contractorcrm-archive-\d{4}-\d{2}-\d{2}\.zip$/,
    );
  });

  it("skips the export when the save dialog is cancelled", async () => {
    const user = userEvent.setup();
    vi.mocked(save).mockResolvedValue(null);
    const client = stubClient();

    render(<App client={client} />);
    await openDataView(user);
    await user.click(screen.getByRole("button", { name: "Export archive…" }));

    expect(client.exportArchive).not.toHaveBeenCalled();
  });

  it("surfaces the core's message when an archive export fails", async () => {
    const user = userEvent.setup();
    vi.mocked(save).mockResolvedValue("/tmp/crm.zip");
    const client = stubClient({
      exportArchive: vi.fn().mockRejectedValue({
        kind: "invalid_input",
        message: "/tmp/crm.zip already exists",
        field: "path",
      }),
    });

    render(<App client={client} />);
    await openDataView(user);
    await user.click(screen.getByRole("button", { name: "Export archive…" }));

    expect(await screen.findByText("/tmp/crm.zip already exists")).toBeVisible();
  });

  // Record views re-query on mount, so returning to Contacts reads the
  // replaced database rather than the rows loaded before the import.
  it("previews a picked archive, replaces the data on confirm, and shows the restored records", async () => {
    const user = userEvent.setup();
    vi.mocked(open).mockResolvedValue("/tmp/crm.zip");
    const listContacts = vi
      .fn()
      .mockResolvedValueOnce([makeContact({ id: "old", displayName: "Old Data" })])
      .mockResolvedValue([makeContact({ id: "new", displayName: "Restored Contact" })]);
    const client = stubClient({
      listContacts,
      previewArchiveImport: vi.fn().mockResolvedValue({
        schemaVersion: 1,
        product: { name: "ContractorCRM", version: "0.1.0" },
        exportedAt: "2026-08-18T15:30:00Z",
        databaseMigrationVersion: 9,
        recordCounts: { contacts: 12 },
        issues: [],
      }),
      importArchive: vi.fn().mockResolvedValue({
        recordCounts: { contacts: 12 },
        safetyBackupPath: "/backups/pre-import.sqlite3",
      }),
    });

    render(<App client={client} />);
    expect(await screen.findByText("Old Data")).toBeVisible();
    await openDataView(user);
    await user.click(screen.getByRole("button", { name: "Import archive…" }));

    expect(await screen.findByRole("dialog", { name: "Import portable archive" })).toBeVisible();
    expect(client.previewArchiveImport).toHaveBeenCalledWith("/tmp/crm.zip");
    await user.click(await screen.findByRole("button", { name: "Replace all data and import" }));
    expect(client.importArchive).toHaveBeenCalledWith("/tmp/crm.zip");
    expect(await screen.findByText("Archive imported — 12 records restored.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Done" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await screen.findByText("Archive imported — all records were replaced.")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Contacts" }));
    expect(await screen.findByText("Restored Contact")).toBeVisible();
    expect(screen.queryByText("Old Data")).not.toBeInTheDocument();
  });

  it("blocks the import and lists the problems when the archive is invalid", async () => {
    const user = userEvent.setup();
    vi.mocked(open).mockResolvedValue("/tmp/crm.zip");
    const client = stubClient({
      previewArchiveImport: vi.fn().mockResolvedValue({
        schemaVersion: 2,
        product: { name: "ContractorCRM", version: "0.2.0" },
        exportedAt: "2026-08-18T15:30:00Z",
        databaseMigrationVersion: 9,
        recordCounts: { contacts: 12 },
        issues: [{ code: "unsupported_schema_version", message: "archive schema version 2 is not supported" }],
      }),
    });

    render(<App client={client} />);
    await openDataView(user);
    await user.click(screen.getByRole("button", { name: "Import archive…" }));

    expect(await screen.findByText("This archive can't be imported.")).toBeVisible();
    expect(screen.getByText("archive schema version 2 is not supported")).toBeVisible();
    expect(screen.getByRole("button", { name: "Replace all data and import" })).toBeDisabled();
    expect(client.importArchive).not.toHaveBeenCalled();
  });

  it("restores focus to the import button when the dialog is dismissed with Escape", async () => {
    const user = userEvent.setup();
    vi.mocked(open).mockResolvedValue("/tmp/crm.zip");
    const client = stubClient();

    render(<App client={client} />);
    await openDataView(user);
    const trigger = screen.getByRole("button", { name: "Import archive…" });
    await user.click(trigger);

    expect(await screen.findByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    // The restore is deferred a frame so macOS drops the dialog first.
    await waitFor(() => expect(trigger).toHaveFocus());
    expect(client.importArchive).not.toHaveBeenCalled();
  });

  it("reports a file picker failure instead of failing silently", async () => {
    const user = userEvent.setup();
    vi.mocked(open).mockRejectedValue(new Error("no window handle"));
    const client = stubClient();

    render(<App client={client} />);
    await openDataView(user);
    await user.click(screen.getByRole("button", { name: "Import archive…" }));

    expect(await screen.findByText("The file picker could not be opened.")).toBeVisible();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
