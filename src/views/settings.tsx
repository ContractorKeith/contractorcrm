import { useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";

import type { CoreClient } from "../api/client";
import { isCommandError } from "../api/types";
import { ArchiveImportDialog, totalRecords } from "../components/ArchiveImportDialog";

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
