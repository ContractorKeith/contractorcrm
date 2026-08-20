import { useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";

import type { CoreClient } from "../api/client";
import { isCommandError, type AiSettings, type FollowupTemplate } from "../api/types";
import { ArchiveImportDialog, totalRecords } from "../components/ArchiveImportDialog";
import { Field } from "./form-support";

interface SettingsViewProps {
  client: CoreClient;
}

// contractorcrm-archive-YYYY-MM-DD.zip in the user's local date.
function suggestedArchiveName(today = new Date()): string {
  const parts = [
    today.getFullYear(),
    String(today.getMonth() + 1).padStart(2, "0"),
    String(today.getDate()).padStart(2, "0"),
  ];
  return `contractorcrm-archive-${parts.join("-")}.zip`;
}

// Host (with port) of an endpoint URL, for the disclosure line.
export function endpointHost(baseUrl: string): string {
  const withoutScheme = baseUrl.includes("://") ? baseUrl.split("://")[1] ?? "" : baseUrl;
  const authority = withoutScheme.split(/[/?#]/)[0] ?? "";
  return authority.split("@").pop() ?? "";
}

// True when the model runs on this machine, so no CRM data leaves the device.
export function isLocalEndpoint(baseUrl: string): boolean {
  const host = (endpointHost(baseUrl).split(":")[0] ?? "").replace(/[[\]]/g, "");
  return ["localhost", "127.0.0.1", "::1", "0.0.0.0"].includes(host) || host.endsWith(".localhost");
}

// One plain line telling the user where their contact details would go.
export function disclosureLine(baseUrl: string): string {
  const host = endpointHost(baseUrl);
  if (!host) return "Add a model address to see where your records would go.";
  if (isLocalEndpoint(baseUrl)) return "Local · no data leaves this machine";
  return `Records you send go to ${host}`;
}

/** AI Assistant: which model to use and where it runs. Off by default. */
function AiAssistantSection({ client }: SettingsViewProps) {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [providerLabel, setProviderLabel] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [status, setStatus] = useState("");
  const [error, setError] = useState<string | null>(null);

  // Apply a fresh settings payload to both the record and the form fields.
  const apply = (next: AiSettings) => {
    setSettings(next);
    setEnabled(next.enabled);
    setProviderLabel(next.providerLabel);
    setBaseUrl(next.baseUrl);
    setModel(next.model);
  };

  useEffect(() => {
    let active = true;
    void client
      .getAiSettings()
      .then((loaded) => {
        if (active) apply(loaded);
      })
      .catch((rejection: unknown) => {
        if (active)
          setError(
            isCommandError(rejection) ? rejection.message : "AI settings could not be loaded.",
          );
      });
    return () => {
      active = false;
    };
  }, [client]);

  const run = async (action: () => Promise<void>, fallback: string) => {
    setError(null);
    setStatus("");
    try {
      await action();
    } catch (rejection) {
      setError(isCommandError(rejection) ? rejection.message : fallback);
    }
  };

  const saveSettings = () =>
    run(async () => {
      apply(await client.setAiSettings({ enabled, providerLabel, baseUrl, model }));
      setStatus("AI settings saved.");
    }, "The AI settings could not be saved.");

  const saveApiKey = () =>
    run(async () => {
      apply(await client.setAiApiKey(apiKey));
      setApiKey("");
      setStatus("API key saved to this machine's credential store.");
    }, "The API key could not be saved.");

  const removeApiKey = () =>
    run(async () => {
      apply(await client.clearAiApiKey());
      setApiKey("");
      setStatus("API key removed.");
    }, "The API key could not be removed.");

  const testConnection = () =>
    run(async () => {
      const check = await client.testAiProvider();
      const models = check.modelAvailable
        ? `${check.model} is ready`
        : `${check.model} was not on the list (${check.availableModels.join(", ") || "no models"})`;
      setStatus(`Connected to ${check.endpointHost} — ${models}.`);
    }, "The AI provider could not be reached.");

  return (
    <div className="data-section">
      <h3>AI Assistant</h3>
      <p>
        The assistant is off until you turn it on, and it only runs when you ask it to. Point it at
        a model on this machine, or at a service you have an API key for.
      </p>

      <label className="field field--inline">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(event) => setEnabled(event.target.checked)}
        />
        <span className="field__label">Use an AI assistant</span>
      </label>

      <Field label="Provider name">
        <input
          type="text"
          value={providerLabel}
          placeholder="Local model"
          onChange={(event) => setProviderLabel(event.target.value)}
        />
      </Field>
      <Field label="Model address">
        <input
          type="text"
          value={baseUrl}
          placeholder="http://127.0.0.1:11434/v1"
          onChange={(event) => setBaseUrl(event.target.value)}
        />
      </Field>
      <Field label="Model">
        <input
          type="text"
          value={model}
          placeholder="llama3.1"
          onChange={(event) => setModel(event.target.value)}
        />
      </Field>

      <p className="data-section__disclosure">{disclosureLine(baseUrl)}</p>

      <Field label="API key (only needed for online services)">
        <input
          type="password"
          value={apiKey}
          autoComplete="off"
          placeholder={settings?.hasApiKey ? "A key is saved on this machine" : "No key saved"}
          onChange={(event) => setApiKey(event.target.value)}
        />
      </Field>

      <div className="data-section__actions">
        <button type="button" className="button" onClick={() => void saveSettings()}>
          Save AI settings
        </button>
        <button
          type="button"
          className="button"
          disabled={apiKey.trim() === ""}
          onClick={() => void saveApiKey()}
        >
          Save key
        </button>
        <button
          type="button"
          className="button"
          disabled={!settings?.hasApiKey}
          onClick={() => void removeApiKey()}
        >
          Remove key
        </button>
        <button
          type="button"
          className="button"
          disabled={!settings?.enabled}
          onClick={() => void testConnection()}
        >
          Test connection
        </button>
      </div>

      <p className="data-section__result" role="status" aria-live="polite">
        {status}
      </p>
      {error ? (
        <p role="alert" className="saved-views__error">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** Agent access: how to wire the MCP helper into an agent client, and what
 *  each mode allows. Read-only display — the mode is chosen at launch. */
function AgentAccessSection({ client }: SettingsViewProps) {
  const [databasePath, setDatabasePath] = useState<string | null>(null);
  const [helperPath, setHelperPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // The helper ships next to the app executable, so the command line the user
  // copies has to name that file — it is not on anyone's PATH.
  useEffect(() => {
    let active = true;
    void client
      .getAgentHelperPath()
      .then((resolved) => {
        if (active) setHelperPath(resolved);
      })
      .catch(() => {
        // Fall back to the bare name; the surrounding copy explains the rest.
        if (active) setHelperPath(null);
      });
    return () => {
      active = false;
    };
  }, [client]);

  useEffect(() => {
    let active = true;
    void client
      .getDatabaseInfo()
      .then((info) => {
        if (active) setDatabasePath(info.databasePath);
      })
      .catch((rejection: unknown) => {
        if (active)
          setError(
            isCommandError(rejection)
              ? rejection.message
              : "The database location could not be read.",
          );
      });
    return () => {
      active = false;
    };
  }, [client]);

  const path = databasePath ?? "<your database path>";
  const helper = helperPath ? `"${helperPath}"` : "contractorcrm-mcp";

  return (
    <div className="data-section">
      <h3>Agent access (MCP)</h3>
      <p>
        ContractorCRM ships a helper that lets an AI agent read — and, if you allow it, change —
        your CRM through the same rules the app itself follows. The agent client launches the
        helper on this machine; nothing is opened to the network.
      </p>

      <Field label="Read-only (recommended)">
        <input type="text" readOnly value={`${helper} --database "${path}"`} />
      </Field>
      <p>
        Look up contacts, companies, opportunities, history, tasks, and flags, and draft changes
        for you to review. Nothing is written.
      </p>

      <Field label="Read and write">
        <input
          type="text"
          readOnly
          value={`${helper} --database "${path}" --read-write`}
        />
      </Field>
      <p>
        Adds the write commands. Every change is version-checked, recorded in the audit log
        against the agent and its client name, and can be undone right after it is applied.
      </p>

      <p className="data-section__disclosure">
        The mode is whatever you launch the helper with, so it is reversible: drop{" "}
        <code>--read-write</code> from your agent client's configuration and restart it to go back
        to read-only. Backups, archive import/export, and CSV import/export stay in this app and
        are not offered to agents.
      </p>

      {error ? (
        <p role="alert" className="saved-views__error">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** Follow-up templates: the wordings drafting starts from. Usable with the
 *  assistant off, so this section is always available. */
function FollowupTemplatesSection({ client }: SettingsViewProps) {
  const [templates, setTemplates] = useState<FollowupTemplate[]>([]);
  const [status, setStatus] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    void client
      .getFollowupTemplates()
      .then((stored) => setTemplates(stored.templates))
      .catch((rejection: unknown) =>
        setError(
          isCommandError(rejection) ? rejection.message : "Templates could not be loaded.",
        ),
      );
  };

  useEffect(load, [client]);

  const edit = (index: number, field: "name" | "body", value: string) =>
    setTemplates((current) =>
      current.map((template, position) =>
        position === index ? { ...template, [field]: value } : template,
      ),
    );

  const save = async (next: FollowupTemplate[]) => {
    setError(null);
    setStatus("");
    try {
      const stored = await client.setFollowupTemplates({ templates: next });
      setTemplates(stored.templates);
      setStatus("Follow-up templates saved.");
    } catch (rejection) {
      setError(
        isCommandError(rejection) ? rejection.message : "The templates could not be saved.",
      );
    }
  };

  return (
    <div className="data-section">
      <h3>Follow-up templates</h3>
      <p>
        These are the wordings a follow-up starts from. They work as written with the assistant
        off; with it on, the assistant adjusts one to fit the record's history.
      </p>

      {/* Every template repeats the same two field labels, so the group name is
          what tells one "Template name" box from the next. */}
      {templates.map((template, index) => (
        <div
          key={template.id}
          className="followup"
          role="group"
          aria-label={`Template ${index + 1} of ${templates.length}`}
        >
          <Field label="Template name">
            <input
              type="text"
              value={template.name}
              onChange={(event) => edit(index, "name", event.target.value)}
            />
          </Field>
          <Field label="Wording">
            <textarea
              rows={4}
              value={template.body}
              onChange={(event) => edit(index, "body", event.target.value)}
            />
          </Field>
        </div>
      ))}

      <div className="data-section__actions">
        <button type="button" className="button" onClick={() => void save(templates)}>
          Save templates
        </button>
        <button type="button" className="button" onClick={() => void save([])}>
          Reset to defaults
        </button>
      </div>

      <p className="data-section__result" role="status" aria-live="polite">
        {status}
      </p>
      {error ? (
        <p role="alert" className="saved-views__error">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** Backup & Data: portable archive export and import for this device. */
export function SettingsView({ client }: SettingsViewProps) {
  const [importPath, setImportPath] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [error, setError] = useState<string | null>(null);

  // Pick a destination, then write the whole CRM to it. The native save dialog
  // already confirms replacement, so the export overwrites without asking again.
  const exportArchive = async () => {
    setError(null);
    setStatus("");
    try {
      const destination = await save({
        defaultPath: suggestedArchiveName(),
        filters: [{ name: "ContractorCRM archive", extensions: ["zip"] }],
      });
      if (typeof destination !== "string") return;
      const report = await client.exportArchive(destination, true);
      setStatus(
        `Exported ${report.fileCount} files and ${totalRecords(report.recordCounts)} records to ${report.path}.`,
      );
    } catch (rejection) {
      setError(
        isCommandError(rejection) ? rejection.message : "The archive could not be exported.",
      );
    }
  };

  // Pick an archive file, then hand it to the confirmation dialog.
  const pickArchive = async () => {
    setError(null);
    setStatus("");
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "ContractorCRM archive", extensions: ["zip"] }],
      });
      if (typeof picked === "string") setImportPath(picked);
    } catch (rejection) {
      setError(
        isCommandError(rejection) ? rejection.message : "The file picker could not be opened.",
      );
    }
  };

  return (
    <section className="crm-section" aria-label="Backup and data">
      <div className="section-rule">
        <h2>Backup &amp; Data</h2>
      </div>

      <div className="data-section">
        <h3>Portable archive</h3>
        <p>
          A portable archive is a single file holding your CRM records — contacts, companies,
          opportunities, activities, tasks, tags, and saved views. Keep it as a backup or move
          your records to another machine. App preferences such as attention thresholds stay on
          this device and do not travel with the archive.
        </p>
        <div className="data-section__actions">
          <button type="button" className="button" onClick={() => void exportArchive()}>
            Export archive…
          </button>
          <button type="button" className="button" onClick={() => void pickArchive()}>
            Import archive…
          </button>
        </div>
        <p className="data-section__result" role="status" aria-live="polite">
          {status}
        </p>
        {error ? (
          <p role="alert" className="saved-views__error">
            {error}
          </p>
        ) : null}
      </div>

      <AiAssistantSection client={client} />

      <AgentAccessSection client={client} />

      <FollowupTemplatesSection client={client} />

      {importPath ? (
        <ArchiveImportDialog
          client={client}
          path={importPath}
          onClose={(imported) => {
            setImportPath(null);
            // Record views re-query when they mount, so navigating back to
            // them after an import always reads the replaced database.
            if (imported) setStatus("Archive imported — all records were replaced.");
          }}
        />
      ) : null}
    </section>
  );
}
