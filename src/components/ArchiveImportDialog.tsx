import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import type { CoreClient } from "../api/client";
import {
  isCommandError,
  type ArchiveImportPreview,
  type ArchiveImportReport,
  type ArchiveRecordCounts,
} from "../api/types";
import { formatLocalDateTime } from "../views/date-format";

// Contractor-facing names for the canonical tables an archive carries.
const TABLE_LABELS: Record<string, string> = {
  companies: "Companies",
  contacts: "Contacts",
  contact_channels: "Contact phones & emails",
  pipelines: "Pipelines",
  stages: "Stages",
  lost_reasons: "Lost reasons",
  opportunities: "Opportunities",
  stage_history: "Stage history",
  activities: "Activities",
  tasks: "Tasks",
  saved_views: "Saved views",
  tags: "Tags",
  record_tags: "Tag assignments",
  custom_field_defs: "Custom fields",
  custom_field_options: "Custom field options",
  custom_field_values: "Custom field values",
};

function tableLabel(table: string): string {
  return TABLE_LABELS[table] ?? table.replace(/_/g, " ");
}

export function totalRecords(counts: ArchiveRecordCounts): number {
  return Object.values(counts).reduce((sum, count) => sum + count, 0);
}

interface ArchiveImportDialogProps {
  client: CoreClient;
  path: string;
  /** Called on close; `imported` is true when the archive replaced live data. */
  onClose: (imported: boolean) => void;
}

/** Counts for every table the archive carries, plus the total. */
function RecordCounts({ counts, caption }: { counts: ArchiveRecordCounts; caption: string }) {
  const tables = Object.keys(counts).sort();
  return (
    <table className="archive-import__counts">
      <caption>{caption}</caption>
      <thead>
        <tr>
          <th scope="col">Records</th>
          <th scope="col">Count</th>
        </tr>
      </thead>
      <tbody>
        {tables.map((table) => (
          <tr key={table}>
            <th scope="row">{tableLabel(table)}</th>
            <td>{counts[table]}</td>
          </tr>
        ))}
        <tr>
          <th scope="row">Total</th>
          <td>{totalRecords(counts)}</td>
        </tr>
      </tbody>
    </table>
  );
}

/**
 * Confirmation for a picked archive file: verify it, show what it holds, and
 * require a deliberate confirm because importing replaces every CRM record.
 */
export function ArchiveImportDialog({ client, path, onClose }: ArchiveImportDialogProps) {
  const [preview, setPreview] = useState<ArchiveImportPreview | null>(null);
  const [report, setReport] = useState<ArchiveImportReport | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  // Verify the archive without touching the database.
  const loadPreview = useCallback(async () => {
    setBusy(true);
    try {
      setPreview(await client.previewArchiveImport(path));
      setError(null);
    } catch (rejection) {
      setPreview(null);
      setError(
        isCommandError(rejection) ? rejection.message : "That archive file could not be read.",
      );
    } finally {
      setBusy(false);
    }
  }, [client, path]);

  useEffect(() => {
    void loadPreview();
  }, [loadPreview]);

  // Focus the dialog on open and hand focus back to the trigger on close.
  // The restore runs on the next frame — macOS needs the dialog gone first.
  useEffect(() => {
    restoreFocusRef.current = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    return () => {
      const invoker = restoreFocusRef.current;
      requestAnimationFrame(() => invoker?.focus());
    };
  }, []);

  const runImport = async () => {
    setBusy(true);
    try {
      setReport(await client.importArchive(path));
      setError(null);
    } catch (rejection) {
      setError(
        isCommandError(rejection) ? rejection.message : "The archive could not be imported.",
      );
    } finally {
      setBusy(false);
    }
  };

  const trapFocus = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose(report !== null);
      return;
    }
    if (event.key !== "Tab") return;
    const nodes = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        "button:not([disabled]),input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex='-1'])",
      ) ?? [],
    );
    const first = nodes[0];
    const last = nodes.at(-1);
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const blocked = preview !== null && preview.issues.length > 0;
  const canImport = preview !== null && !blocked && report === null && !busy;

  return createPortal(
    <div className="global-search-backdrop">
      <section
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="archive-import-title"
        className="saved-views__dialog archive-import"
        onKeyDown={trapFocus}
      >
        <h2 id="archive-import-title">Import portable archive</h2>
        <p className="csv-import__path">{path}</p>

        {error ? (
          <p role="alert" className="form-error">
            {error}
          </p>
        ) : null}

        {report ? (
          <div className="archive-import__done">
            <p role="status" aria-live="polite">
              Archive imported — {totalRecords(report.recordCounts)} records restored.
            </p>
            <RecordCounts counts={report.recordCounts} caption="Records imported" />
            <p className="archive-import__backup">
              Safety backup of your previous data: {report.safetyBackupPath}
            </p>
          </div>
        ) : null}

        {!report && preview ? (
          <>
            <dl className="archive-import__manifest">
              <dt>Written by</dt>
              <dd>
                {preview.product.name} {preview.product.version}
              </dd>
              <dt>Exported</dt>
              <dd>{formatLocalDateTime(preview.exportedAt)}</dd>
              <dt>Archive format</dt>
              <dd>
                Version {preview.schemaVersion} · database {preview.databaseMigrationVersion}
              </dd>
            </dl>

            <RecordCounts counts={preview.recordCounts} caption="Records in this archive" />

            {blocked ? (
              <div className="archive-import__blocked">
                <p role="alert">This archive can't be imported.</p>
                <ul aria-label="Archive problems">
                  {preview.issues.map((archiveIssue, index) => (
                    <li key={index}>{archiveIssue.message}</li>
                  ))}
                </ul>
              </div>
            ) : (
              <div className="archive-import__warning">
                <p>
                  <strong>Importing replaces all current CRM data</strong> — contacts, companies,
                  pipeline, activities, tasks, tags, and saved views are deleted and rebuilt from
                  this archive.
                </p>
                <p>
                  A safety backup of your current data is created automatically before anything is
                  replaced.
                </p>
              </div>
            )}
          </>
        ) : null}

        <div className="form-actions">
          <button
            ref={closeRef}
            type="button"
            className="button"
            onClick={() => onClose(report !== null)}
          >
            {report ? "Done" : "Cancel"}
          </button>
          {!report ? (
            <button
              type="button"
              className="button button--danger"
              disabled={!canImport}
              onClick={() => void runImport()}
            >
              Replace all data and import
            </button>
          ) : null}
        </div>
      </section>
    </div>,
    document.body,
  );
}
