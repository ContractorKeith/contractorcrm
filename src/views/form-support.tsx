import type { ReactNode } from "react";

import { isCommandError, type ContactRole, type PartyKind } from "../api/types";

// Wire enum options with contractor-facing labels.
export const PARTY_KIND_OPTIONS: { value: PartyKind; label: string }[] = [
  { value: "lead", label: "Lead" },
  { value: "client", label: "Client" },
  { value: "sub", label: "Sub" },
  { value: "vendor", label: "Vendor" },
  { value: "supplier", label: "Supplier" },
  { value: "other", label: "Other" },
];

export const CONTACT_ROLE_OPTIONS: { value: ContactRole; label: string }[] = [
  { value: "owner", label: "Owner" },
  { value: "estimator", label: "Estimator" },
  { value: "site_contact", label: "Site contact" },
  { value: "office", label: "Office" },
  { value: "other", label: "Other" },
];

export function partyKindLabel(kind: PartyKind): string {
  return PARTY_KIND_OPTIONS.find((option) => option.value === kind)?.label ?? kind;
}

export function contactRoleLabel(role: ContactRole | null): string {
  if (!role) return "—";
  return CONTACT_ROLE_OPTIONS.find((option) => option.value === role)?.label ?? role;
}

// Per-form error state derived from a rejected core command.
export interface SaveError {
  // Keyed by wire field path, e.g. "name" or "channels[1].value".
  fields: Record<string, string>;
  general: string | null;
  conflict: boolean;
}

export const NO_SAVE_ERROR: SaveError = { fields: {}, general: null, conflict: false };

// Translate a client rejection into inline form feedback.
export function saveErrorFrom(error: unknown): SaveError {
  if (!isCommandError(error)) {
    return { fields: {}, general: "Something went wrong saving to the local database.", conflict: false };
  }
  switch (error.kind) {
    case "invalid_input":
    case "validation_failed": {
      // The Rust message may lead with the field path; keep the plain part.
      const prefix = `${error.field}: `;
      const message = error.message.startsWith(prefix)
        ? error.message.slice(prefix.length)
        : error.message;
      return { fields: { [error.field]: message }, general: null, conflict: false };
    }
    case "missing_lost_reason":
      // Surface next to the lost-reason select on stage moves.
      return { fields: { lostReasonId: error.message }, general: null, conflict: false };
    case "version_conflict":
      return { fields: {}, general: null, conflict: true };
    default:
      return { fields: {}, general: error.message, conflict: false };
  }
}

// Labeled form field with an optional inline error under the control.
export function Field({
  label,
  error,
  children,
}: {
  label: string;
  error?: string | undefined;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span className="field__label">{label}</span>
      {children}
      {error ? (
        <span className="field__error" role="alert">
          {error}
        </span>
      ) : null}
    </label>
  );
}

// Version-conflict banner: someone (or an agent) changed the record since it
// was loaded; the only safe move is to reload the latest before editing again.
export function ConflictBanner({ onReload }: { onReload: () => void }) {
  return (
    <div className="conflict-banner" role="alert">
      <p>
        This record changed outside this form — likely in another window or by an agent. Reload
        the latest version, then re-apply your edits.
      </p>
      <button type="button" className="button" onClick={onReload}>
        Reload latest
      </button>
    </div>
  );
}

// General (non-field) error line for forms and detail actions.
export function GeneralError({ message }: { message: string | null }) {
  if (!message) return null;
  return (
    <p className="form-error" role="alert">
      {message}
    </p>
  );
}
