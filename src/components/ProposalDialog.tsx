import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import type { CoreClient } from "../api/client";
import {
  isCommandError,
  type Proposal,
  type ProposalApplied,
  type ProposalEntityType,
  type ProposalUndone,
} from "../api/types";

// Contractor-facing names for the fields a draft can touch.
const FIELD_LABELS: Record<string, string> = {
  firstName: "First name",
  lastName: "Last name",
  displayName: "Name",
  role: "Role",
  kind: "Kind",
  phone: "Phone",
  email: "Email",
  preferredContactMethod: "Preferred contact",
  addressLine1: "Address",
  addressLine2: "Address line 2",
  city: "City",
  state: "State",
  postalCode: "ZIP",
  propertyType: "Property type",
  notes: "Notes",
  name: "Name",
  website: "Website",
  serviceArea: "Service area",
  licenseNotes: "License notes",
  valueMinor: "Value",
  currencyCode: "Currency",
  probabilityPercent: "Probability",
  expectedCloseDate: "Expected close",
  source: "Source",
  sourceLabel: "Source detail",
  contactId: "Contact",
  companyId: "Company",
  title: "Title",
  dueAt: "Due",
  remindAt: "Remind",
  priority: "Priority",
  parentType: "Linked to",
  parentId: "Linked record",
  body: "Message",
};

export function fieldLabel(field: string): string {
  return FIELD_LABELS[field] ?? field;
}

/** Money is stored in minor units; show it the way a contractor reads it. */
export function displayValue(field: string, value: string | null): string {
  if (value === null || value === "") return "—";
  if (field === "valueMinor") {
    const minor = Number(value);
    if (Number.isFinite(minor)) return (minor / 100).toFixed(2);
  }
  if (field === "probabilityPercent") return `${value}%`;
  return value;
}

// Contractor-facing text for the failures this dialog can hit.
function errorText(rejection: unknown, fallback: string): string {
  if (!isCommandError(rejection)) return fallback;
  switch (rejection.kind) {
    case "version_conflict":
      return "This record changed since the draft was made — ask the assistant again.";
    case "proposal_expired":
      return "This draft expired — ask the assistant again.";
    default:
      return rejection.message;
  }
}

interface ProposalDialogProps {
  client: CoreClient;
  proposal: Proposal;
  /** Called on close; `applied` is true when the draft was written. */
  onClose: (applied: boolean) => void;
}

/**
 * Review one drafted change: the field-level diff, any warnings, and an
 * explicit Apply or Discard. Nothing is written until Apply, and an applied
 * draft can be undone from here.
 */
export function ProposalDialog({ client, proposal, onClose }: ProposalDialogProps) {
  const [applied, setApplied] = useState<ProposalApplied | null>(null);
  const [undone, setUndone] = useState<ProposalUndone | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

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

  const apply = async () => {
    setBusy(true);
    try {
      setApplied(
        await client.applyProposal({
          proposalId: proposal.id,
          expectedVersions: proposal.affectedVersions,
        }),
      );
      setError(null);
    } catch (rejection) {
      setError(errorText(rejection, "That draft could not be applied."));
    } finally {
      setBusy(false);
    }
  };

  const undo = async () => {
    if (!applied) return;
    setBusy(true);
    try {
      setUndone(
        await client.undoProposal({
          undoToken: applied.undoToken,
          expectedVersions: [
            {
              entityType: applied.entityType,
              entityId: applied.entityId,
              version: applied.version,
            },
          ],
        }),
      );
      setError(null);
    } catch (rejection) {
      setError(errorText(rejection, "That change could not be undone."));
    } finally {
      setBusy(false);
    }
  };

  const trapFocus = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      if (!busy) onClose(applied !== null);
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

  const isCreate = proposal.entityId === null;
  const canApply = applied === null && !busy && proposal.changes.length > 0;

  return createPortal(
    <div className="global-search-backdrop">
      <section
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="proposal-title"
        className="saved-views__dialog proposal"
        onKeyDown={trapFocus}
      >
        <h2 id="proposal-title">Review the assistant's draft</h2>
        <p className="proposal__summary">{proposal.summary}</p>

        {error ? (
          <p role="alert" className="form-error">
            {error}
          </p>
        ) : null}

        {applied ? (
          <p role="status" aria-live="polite" className="proposal__applied">
            {undone
              ? undone.action === "archived"
                ? "Undone — the new record was archived."
                : "Undone — the record is back the way it was."
              : isCreate
                ? "Draft applied — the record was created."
                : "Draft applied — the record was updated."}
          </p>
        ) : null}

        {proposal.changes.length > 0 ? (
          <table className="proposal__changes">
            <caption>{isCreate ? "New record" : "Proposed changes"}</caption>
            <thead>
              <tr>
                <th scope="col">Field</th>
                <th scope="col">Now</th>
                <th scope="col">After</th>
              </tr>
            </thead>
            <tbody>
              {proposal.changes.map((change) => (
                <tr key={change.field}>
                  <th scope="row">{fieldLabel(change.field)}</th>
                  <td>{displayValue(change.field, change.before)}</td>
                  <td>{displayValue(change.field, change.after)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="proposal__empty">
            The assistant didn't find anything to change. Try describing it another way.
          </p>
        )}

        {proposal.warnings.length > 0 ? (
          <div className="proposal__warnings">
            <p>Check these before applying:</p>
            <ul aria-label="Draft warnings">
              {proposal.warnings.map((warning, index) => (
                <li key={index}>{warning}</li>
              ))}
            </ul>
          </div>
        ) : null}

        <div className="form-actions">
          <button
            ref={closeRef}
            type="button"
            className="button"
            disabled={busy}
            onClick={() => onClose(applied !== null)}
          >
            {applied ? "Done" : "Discard"}
          </button>
          {applied && !undone ? (
            <button type="button" className="button" disabled={busy} onClick={() => void undo()}>
              Undo
            </button>
          ) : null}
          {!applied ? (
            <button
              type="button"
              className="button button--primary"
              disabled={!canApply}
              onClick={() => void apply()}
            >
              Apply
            </button>
          ) : null}
        </div>
      </section>
    </div>,
    document.body,
  );
}

interface AssistantPromptProps {
  client: CoreClient;
  entityType: ProposalEntityType;
  /** Present for an update: which record, and the version it is at. */
  target?: { entityId: string; expectedVersion: number };
  label: string;
  placeholder: string;
  /** Called after a draft was applied, so the caller can reload. */
  onApplied?: () => void;
}

/**
 * "Ask the assistant" entry point: one line of plain language in, a reviewable
 * draft out. Renders nothing at all while the assistant is switched off.
 */
export function AssistantPrompt({
  client,
  entityType,
  target,
  label,
  placeholder,
  onApplied,
}: AssistantPromptProps) {
  const [enabled, setEnabled] = useState(false);
  const [text, setText] = useState("");
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    client
      .getAiSettings()
      .then((settings) => {
        if (active) setEnabled(settings.enabled);
      })
      .catch(() => {
        if (active) setEnabled(false);
      });
    return () => {
      active = false;
    };
  }, [client]);

  if (!enabled) return null;

  const ask = async () => {
    if (text.trim() === "") return;
    setBusy(true);
    try {
      setProposal(
        target
          ? await client.proposeUpdate(entityType, target.entityId, text, target.expectedVersion)
          : await client.proposeRecord(entityType, text),
      );
      setError(null);
    } catch (rejection) {
      setError(errorText(rejection, "The assistant couldn't draft that."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="assistant-prompt">
      <label className="assistant-prompt__field">
        <span className="assistant-prompt__label">{label}</span>
        <input
          type="text"
          value={text}
          placeholder={placeholder}
          disabled={busy}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void ask();
            }
          }}
        />
      </label>
      <button
        type="button"
        className="button"
        disabled={busy || text.trim() === ""}
        onClick={() => void ask()}
      >
        Ask
      </button>
      {error ? (
        <span role="alert" className="saved-views__error">
          {error}
        </span>
      ) : null}
      {proposal ? (
        <ProposalDialog
          client={client}
          proposal={proposal}
          onClose={(applied) => {
            setProposal(null);
            setText("");
            if (applied) onApplied?.();
          }}
        />
      ) : null}
    </div>
  );
}
