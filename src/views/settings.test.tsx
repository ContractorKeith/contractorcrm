import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { open, save } from "@tauri-apps/plugin-dialog";

import { App } from "../App";
import type { AiSettings } from "../api/types";
import { makeContact, makeFollowupTemplates, stubClient } from "../test/stub-client";

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

describe("AI assistant settings", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.mocked(open).mockReset();
    vi.mocked(save).mockReset();
  });

  const aiSettings = (overrides: Partial<AiSettings> = {}): AiSettings => ({
    version: 1,
    enabled: false,
    providerLabel: "Local model",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "llama3.1",
    hasApiKey: false,
    ...overrides,
  });

  it("shows the local disclosure line and no test button until the assistant is on", async () => {
    const user = userEvent.setup();
    const client = stubClient({ getAiSettings: vi.fn().mockResolvedValue(aiSettings()) });

    render(<App client={client} />);
    await openDataView(user);

    expect(await screen.findByRole("heading", { name: "AI Assistant" })).toBeVisible();
    expect(screen.getByText("Local · no data leaves this machine")).toBeVisible();
    expect(screen.getByRole("button", { name: "Test connection" })).toBeDisabled();
    expect(screen.getByLabelText("Use an AI assistant")).not.toBeChecked();
  });

  it("names the endpoint host when the model is not on this machine", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getAiSettings: vi
        .fn()
        .mockResolvedValue(aiSettings({ baseUrl: "https://api.openai.com/v1" })),
    });

    render(<App client={client} />);
    await openDataView(user);

    expect(await screen.findByText("Records you send go to api.openai.com")).toBeVisible();
  });

  it("saves the settings the user typed", async () => {
    const user = userEvent.setup();
    const setAiSettings = vi
      .fn()
      .mockResolvedValue(aiSettings({ enabled: true, model: "mistral" }));
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(aiSettings()),
      setAiSettings,
    });

    render(<App client={client} />);
    await openDataView(user);
    await screen.findByRole("heading", { name: "AI Assistant" });

    await user.click(screen.getByLabelText("Use an AI assistant"));
    const modelField = screen.getByLabelText("Model");
    await user.clear(modelField);
    await user.type(modelField, "mistral");
    await user.click(screen.getByRole("button", { name: "Save AI settings" }));

    expect(setAiSettings).toHaveBeenCalledWith({
      enabled: true,
      providerLabel: "Local model",
      baseUrl: "http://127.0.0.1:11434/v1",
      model: "mistral",
    });
    expect(await screen.findByText("AI settings saved.")).toBeVisible();
  });

  it("stores an API key write-only and reports only that one is saved", async () => {
    const user = userEvent.setup();
    const setAiApiKey = vi.fn().mockResolvedValue(aiSettings({ hasApiKey: true }));
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(aiSettings()),
      setAiApiKey,
    });

    render(<App client={client} />);
    await openDataView(user);
    const keyField = await screen.findByLabelText("API key (only needed for online services)");
    expect(keyField).toHaveAttribute("type", "password");
    expect(keyField).toHaveAttribute("placeholder", "No key saved");

    await user.type(keyField, "sk-secret-key");
    await user.click(screen.getByRole("button", { name: "Save key" }));

    expect(setAiApiKey).toHaveBeenCalledWith("sk-secret-key");
    expect(
      await screen.findByText("API key saved to this machine's credential store."),
    ).toBeVisible();
    // The field is cleared and never re-populated from the core.
    expect(keyField).toHaveValue("");
    expect(keyField).toHaveAttribute("placeholder", "A key is saved on this machine");
  });

  it("clears a stored API key", async () => {
    const user = userEvent.setup();
    const clearAiApiKey = vi.fn().mockResolvedValue(aiSettings({ hasApiKey: false }));
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(aiSettings({ hasApiKey: true })),
      clearAiApiKey,
    });

    render(<App client={client} />);
    await openDataView(user);
    const remove = await screen.findByRole("button", { name: "Remove key" });
    await user.click(remove);

    expect(clearAiApiKey).toHaveBeenCalled();
    expect(await screen.findByText("API key removed.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Remove key" })).toBeDisabled();
  });

  it("reports the endpoint and model after a connection test", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(aiSettings({ enabled: true })),
      testAiProvider: vi.fn().mockResolvedValue({
        providerLabel: "Local model",
        endpointHost: "127.0.0.1:11434",
        local: true,
        model: "llama3.1",
        modelAvailable: true,
        availableModels: ["llama3.1"],
      }),
    });

    render(<App client={client} />);
    await openDataView(user);
    await user.click(await screen.findByRole("button", { name: "Test connection" }));

    expect(
      await screen.findByText("Connected to 127.0.0.1:11434 — llama3.1 is ready."),
    ).toBeVisible();
  });

  it("surfaces the core's reason when the provider cannot be reached", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(aiSettings({ enabled: true })),
      testAiProvider: vi.fn().mockRejectedValue({
        kind: "provider_unavailable",
        message: "Couldn't reach 127.0.0.1:11434.",
        reason: "Couldn't reach 127.0.0.1:11434.",
      }),
    });

    render(<App client={client} />);
    await openDataView(user);
    await user.click(await screen.findByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Couldn't reach 127.0.0.1:11434.")).toBeVisible();
  });

  it("shows the agent helper command line for this device and explains both modes", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getDatabaseInfo: vi.fn().mockResolvedValue({
        databasePath: "/Users/sam/Library/Application Support/ContractorCRM/contractorcrm.sqlite3",
        fileSizeBytes: 2048,
        lastBackupAt: null,
      }),
    });

    render(<App client={client} />);
    await openDataView(user);
    await screen.findByRole("heading", { name: "Agent access (MCP)" });

    const readOnly = await screen.findByLabelText("Read-only (recommended)");
    expect(readOnly).toHaveValue(
      'contractorcrm-mcp --database "/Users/sam/Library/Application Support/ContractorCRM/contractorcrm.sqlite3"',
    );
    expect(readOnly).toHaveAttribute("readonly");
    expect(screen.getByLabelText("Read and write")).toHaveValue(
      'contractorcrm-mcp --database "/Users/sam/Library/Application Support/ContractorCRM/contractorcrm.sqlite3" --read-write',
    );
    expect(screen.getByText(/Nothing is written\./)).toBeVisible();
    expect(screen.getByText(/recorded in the audit log/)).toBeVisible();
    expect(screen.getByText(/restart it to go back/)).toBeVisible();
  });

  it("edits and resets the follow-up templates, which work with the assistant off", async () => {
    const user = userEvent.setup();
    const saved = makeFollowupTemplates({
      templates: [{ id: "call_followup", name: "Call check-in", body: "Thanks for the call." }],
    });
    const client = stubClient({
      getFollowupTemplates: vi.fn().mockResolvedValue(makeFollowupTemplates()),
      setFollowupTemplates: vi.fn().mockResolvedValue(saved),
    });

    render(<App client={client} />);
    await openDataView(user);
    await screen.findByRole("heading", { name: "Follow-up templates" });

    const name = screen.getByDisplayValue("Call follow-up");
    await user.clear(name);
    await user.type(name, "Call check-in");
    await user.click(screen.getByRole("button", { name: "Save templates" }));

    const request = vi.mocked(client.setFollowupTemplates).mock.calls[0]?.[0];
    expect(request?.templates[0]).toMatchObject({ id: "call_followup", name: "Call check-in" });
    expect(request?.templates).toHaveLength(3);
    expect(await screen.findByText("Follow-up templates saved.")).toBeVisible();
    expect(screen.getByDisplayValue("Thanks for the call.")).toBeVisible();

    // Resetting sends an empty list; the core restores the built-ins.
    await user.click(screen.getByRole("button", { name: "Reset to defaults" }));
    expect(vi.mocked(client.setFollowupTemplates).mock.calls[1]?.[0]).toEqual({ templates: [] });
  });
});
