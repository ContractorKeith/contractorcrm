import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import type { CoreClient } from "../api/client";
import {
  isCommandError,
  type ContactImportMapping,
  type ContactImportPreview,
  type ContactImportSummary,
  type ContactImportTarget,
} from "../api/types";

// Every contact field a CSV column can feed, in the order shown in the picker.
const IMPORT_TARGETS: { value: ContactImportTarget; label: string }[] = [
  { value: "externalId", label: "External ID" },
  { value: "firstName", label: "First name" },
  { value: "lastName", label: "Last name" },
  { value: "displayName", label: "Display name" },
  { value: "role", label: "Role" },
  { value: "kind", label: "Kind (lead, client, sub, vendor)" },
  { value: "preferredContactMethod", label: "Preferred contact method" },
  { value: "addressLine1", label: "Address line 1" },
  { value: "addressLine2", label: "Address line 2" },
  { value: "city", label: "City" },
  { value: "state", label: "State" },
  { value: "postalCode", label: "Postal code" },
  { value: "propertyType", label: "Property type" },
  { value: "notes", label: "Notes" },
  { value: "company", label: "Company" },
  { value: "email", label: "Email" },
  { value: "phone", label: "Phone" },
  { value: "tags", label: "Tags" },
];

interface CsvImportDialogProps {
  client: CoreClient;
  path: string;
  /** Called on close; `imported` is true when records were written. */
  onClose: (imported: boolean) => void;
}

// Which target a CSV header is currently mapped to, if any.
function targetForHeader(mapping: ContactImportMapping, header: string): ContactImportTarget | "" {
  const hit = IMPORT_TARGETS.find((target) => mapping[target.value] === header);
  return hit ? hit.value : "";
}

// Move a header onto a target, clearing any other target it fed.
function remap(
  mapping: ContactImportMapping,
  header: string,
  target: ContactImportTarget | "",
): ContactImportMapping {
  const next: ContactImportMapping = { ...mapping };
  for (const known of IMPORT_TARGETS) {
    if (next[known.value] === header) next[known.value] = null;
  }
  if (target !== "") next[target] = header;
  return next;
}

/** Mapping wizard for a picked CSV file: preview, adjust, import, summary. */
export function CsvImportDialog({ client, path, onClose }: CsvImportDialogProps) {
  const [preview, setPreview] = useState<ContactImportPreview | null>(null);
  const [mapping, setMapping] = useState<ContactImportMapping | null>(null);
  const [summary, setSummary] = useState<ContactImportSummary | null>(null);
  const [busy, setBusy] = useState(false);
  // True only while the core is writing contacts — the dialog is sealed shut
  // for that window so a half-written import cannot lose its summary.
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  // Ask the core for a preview — with the caller's mapping when one is set.
  const loadPreview = useCallback(
    async (explicit: ContactImportMapping | null) => {
      setBusy(true);
      try {
        const next = await client.previewContactImport(path, explicit);
        setPreview(next);
        setMapping(next.mapping);
        setError(null);
      } catch (rejection) {
        // A rejected preview (duplicate or empty headers, unreadable file) leaves
        // nothing safe to import, so the mapping table is cleared with it.
        setPreview(null);
        setError(
          isCommandError(rejection) ? rejection.message : "That CSV file could not be read.",
        );
      } finally {
        setBusy(false);
      }
    },
    [client, path],
  );

  useEffect(() => {
    void loadPreview(null);
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

  const changeMapping = (header: string, target: ContactImportTarget | "") => {
    const next = remap(mapping ?? {}, header, target);
    setMapping(next);
    void loadPreview(next);
  };

  const runImport = async () => {
    if (!mapping) return;
    setBusy(true);
    setImporting(true);
    try {
      const result = await client.importContacts({ path, mapping });
      setSummary(result);
      setError(null);
    } catch (rejection) {
      setError(
        isCommandError(rejection) ? rejection.message : "The contacts could not be imported.",
      );
    } finally {
      setImporting(false);
      setBusy(false);
    }
  };

  // Reading the CSV is a read-only round trip the user may abandon; writing
  // contacts is not, so only the write says "don't close this window".
  const progress = importing
    ? "Importing contacts — don't close this window…"
    : busy
      ? "Reading CSV…"
      : null;

  const trapFocus = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      // Never leave mid-import: contacts are being written and the caller
      // still needs the created/updated/skipped summary.
      if (!importing) onClose(summary !== null);
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

  return createPortal(
    <div className="global-search-backdrop">
      <section
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="csv-import-title"
        aria-busy={busy}
        className="saved-views__dialog csv-import"
        onKeyDown={trapFocus}
      >
        <h2 id="csv-import-title">Import contacts from CSV</h2>
        <p className="csv-import__path">{path}</p>

        {progress ? (
          <p role="status" aria-live="polite" className="csv-import__progress">
            {progress}
          </p>
        ) : null}

        {error ? (
          <p role="alert" className="form-error">
            {error}
          </p>
        ) : null}

        {summary ? (
          <div className="csv-import__summary">
            <p role="status" aria-live="polite">
              {summary.created} created, {summary.updated} updated, {summary.skipped.length} skipped.
            </p>
            {summary.skipped.length > 0 ? (
              <ul aria-label="Skipped rows">
                {summary.skipped.map((issue, index) => (
                  <li key={index}>
                    Line {issue.line}: {issue.reason}
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
        ) : null}

        {!summary && preview ? (
          <>
            <p className="csv-import__count">{preview.rowCount} rows found.</p>

            <table className="csv-import__mapping">
              <caption>Column mapping</caption>
              <thead>
                <tr>
                  <th scope="col">CSV column</th>
                  <th scope="col">Contact field</th>
                </tr>
              </thead>
              <tbody>
                {preview.headers.map((header, columnIndex) => (
                  <tr key={columnIndex}>
                    <th scope="row">{header}</th>
                    <td>
                      <select
                        aria-label={`Map column ${header}`}
                        value={targetForHeader(mapping ?? {}, header)}
                        onChange={(event) =>
                          changeMapping(header, event.target.value as ContactImportTarget | "")
                        }
                      >
                        <option value="">Ignored</option>
                        {IMPORT_TARGETS.map((target) => (
                          <option key={target.value} value={target.value}>
                            {target.label}
                          </option>
                        ))}
                      </select>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            {preview.sampleRows.length > 0 ? (
              <table className="csv-import__sample">
                <caption>Sample rows</caption>
                <thead>
                  <tr>
                    {preview.headers.map((header, columnIndex) => (
                      <th key={columnIndex} scope="col">
                        {header}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {preview.sampleRows.map((row, rowIndex) => (
                    <tr key={rowIndex}>
                      {preview.headers.map((_header, cellIndex) => (
                        <td key={cellIndex}>{row[cellIndex] ?? ""}</td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}

            {preview.issues.length > 0 ? (
              <ul aria-label="Import issues">
                {preview.issues.map((issue, index) => (
                  <li key={index}>
                    Line {issue.line}: {issue.reason}
                  </li>
                ))}
              </ul>
            ) : null}
          </>
        ) : null}

        <div className="form-actions">
          <button
            ref={closeRef}
            type="button"
            className="button"
            disabled={importing}
            onClick={() => onClose(summary !== null)}
          >
            {summary ? "Done" : "Cancel"}
          </button>
          {!summary ? (
            <button
              type="button"
              className="button button--primary"
              disabled={busy || preview === null}
              onClick={() => void runImport()}
            >
              Import
            </button>
          ) : null}
        </div>
      </section>
    </div>,
    document.body,
  );
}
