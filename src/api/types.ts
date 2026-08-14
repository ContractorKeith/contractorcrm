// TypeScript mirrors of the Rust wire shapes (src-tauri/src/domain.rs and
// application.rs). All fields are camelCase on the wire; enums are the
// snake_case wire strings.

export type PartyKind = "client" | "lead" | "sub" | "vendor" | "supplier" | "other";
export type ContactRole = "owner" | "estimator" | "site_contact" | "office" | "other";
export type ChannelKind = "phone" | "email";
export type Actor = "user" | "agent" | "import";

// Report from the Rust core's `health` command — proves the UI → Rust seam works.
export interface HealthReport {
  app: string;
  version: string;
  status: string;
}

// A company — client, sub, vendor, or supplier grouping contacts.
export interface Company {
  id: string;
  name: string;
  kind: PartyKind;
  phone: string | null;
  email: string | null;
  website: string | null;
  addressLine1: string | null;
  addressLine2: string | null;
  city: string | null;
  state: string | null;
  postalCode: string | null;
  serviceArea: string | null;
  licenseNotes: string | null;
  notes: string | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
  version: number;
}

// One phone or email row belonging to a contact.
export interface ContactChannel {
  id: string;
  contactId: string;
  kind: ChannelKind;
  label: string | null;
  value: string;
  preferred: boolean;
  sortKey: number;
}

// A person; channels are always loaded with the contact.
export interface Contact {
  id: string;
  companyId: string | null;
  firstName: string | null;
  lastName: string | null;
  displayName: string;
  role: ContactRole | null;
  kind: PartyKind;
  preferredContactMethod: string | null;
  addressLine1: string | null;
  addressLine2: string | null;
  city: string | null;
  state: string | null;
  postalCode: string | null;
  propertyType: string | null;
  notes: string | null;
  favorite: boolean;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
  version: number;
  channels: ContactChannel[];
}

// Editable company fields; updates replace the full editable set (v1).
export interface CompanyPatch {
  name: string;
  kind: PartyKind;
  phone: string | null;
  email: string | null;
  website: string | null;
  addressLine1: string | null;
  addressLine2: string | null;
  city: string | null;
  state: string | null;
  postalCode: string | null;
  serviceArea: string | null;
  licenseNotes: string | null;
  notes: string | null;
}

// One phone or email in a create/update request; ids are assigned on write.
export interface ChannelInput {
  kind: ChannelKind;
  label: string | null;
  value: string;
  preferred: boolean;
}

// Editable contact fields; updates replace the full editable set including
// the whole channel list.
export interface ContactPatch {
  companyId: string | null;
  firstName: string | null;
  lastName: string | null;
  displayName: string | null;
  role: ContactRole | null;
  kind: PartyKind;
  preferredContactMethod: string | null;
  addressLine1: string | null;
  addressLine2: string | null;
  city: string | null;
  state: string | null;
  postalCode: string | null;
  propertyType: string | null;
  notes: string | null;
  favorite: boolean;
  channels: ChannelInput[];
}

// Create requests flatten the patch on the wire; actor defaults to "user".
export type CreateCompanyRequest = { actor?: Actor } & CompanyPatch;
export type CreateContactRequest = { actor?: Actor } & ContactPatch;

export interface UpdateCompanyRequest {
  actor?: Actor;
  companyId: string;
  expectedVersion: number;
  patch: CompanyPatch;
}

export interface UpdateContactRequest {
  actor?: Actor;
  contactId: string;
  expectedVersion: number;
  patch: ContactPatch;
}

// Shared shape for archive/unarchive of either record type.
export interface ArchiveRequest {
  actor?: Actor;
  id: string;
  expectedVersion: number;
}

// Error wire shape (CommandError in src-tauri/src/lib.rs): stable kind,
// human message, plus flattened per-kind details.
export type CommandError =
  | { kind: "invalid_input"; message: string; field: string }
  | { kind: "validation_failed"; message: string; code: string; field: string }
  | { kind: "not_found"; message: string; resource: string; recordId: string }
  | {
      kind: "version_conflict";
      message: string;
      resource: string;
      recordId: string;
      expectedVersion: number;
      currentVersion: number;
    }
  | { kind: "invalid_stored_data"; message: string }
  | { kind: "storage_unavailable"; message: string };

// Narrow an unknown rejection into a CommandError.
export function isCommandError(error: unknown): error is CommandError {
  return (
    typeof error === "object" &&
    error !== null &&
    typeof (error as CommandError).kind === "string" &&
    typeof (error as CommandError).message === "string"
  );
}
