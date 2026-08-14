// TypeScript mirrors of the Rust wire shapes (src-tauri/src/domain.rs and
// application.rs). All fields are camelCase on the wire; enums are the
// snake_case wire strings.

export type PartyKind = "client" | "lead" | "sub" | "vendor" | "supplier" | "other";
export type ContactRole = "owner" | "estimator" | "site_contact" | "office" | "other";
export type ChannelKind = "phone" | "email";
export type Actor = "user" | "agent" | "import";
export type StageKind = "open" | "won" | "lost";
export type OpportunitySource = "referral" | "repeat_client" | "website" | "sign" | "other";

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

// One user-editable pipeline step; renaming/reordering never rewrites history.
export interface Stage {
  id: string;
  pipelineId: string;
  name: string;
  sortKey: number;
  kind: StageKind;
  createdAt: string;
  updatedAt: string;
  version: number;
}

// A user-editable reason an opportunity was lost.
export interface LostReason {
  id: string;
  label: string;
  sortKey: number;
  active: boolean;
}

// Money as integer minor units plus ISO currency code — no floats anywhere.
export interface Money {
  valueMinor: number;
  currencyCode: string;
}

// Potential work moving through the pipeline toward won or lost.
export interface Opportunity {
  id: string;
  name: string;
  contactId: string | null;
  companyId: string | null;
  stageId: string;
  value: Money;
  probabilityPercent: number | null;
  expectedCloseDate: string | null;
  source: OpportunitySource | null;
  sourceLabel: string | null;
  lostReasonId: string | null;
  notes: string | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
  version: number;
}

// One append-only stage change; stores stage ids only, never names.
export interface StageHistoryEntry {
  id: string;
  opportunityId: string;
  fromStageId: string | null;
  toStageId: string;
  actor: Actor;
  lostReasonId: string | null;
  createdAt: string;
}

// Table row for the opportunity list — record flattened with display names
// plus read-time projections (computed from activities/tasks, never stored).
export type OpportunityListItem = Opportunity & {
  stageName: string;
  contactDisplayName: string | null;
  companyName: string | null;
  lastContactedAt?: string | null;
  nextOpenTaskDueAt?: string | null;
};

// List row for the contact table — record flattened with read-time
// projections (latest activity incl. related opportunities, next open task).
export type ContactListItem = Contact & {
  lastContactedAt?: string | null;
  nextOpenTaskDueAt?: string | null;
};

// Detail view — the record flattened with its full stage history.
export type OpportunityDetail = Opportunity & {
  stageHistory: StageHistoryEntry[];
};

// Editable opportunity fields; updates replace the full editable set (v1).
// Stage changes go through move_opportunity_stage, never through updates.
export interface OpportunityPatch {
  name: string;
  contactId: string | null;
  companyId: string | null;
  valueMinor: number;
  currencyCode: string;
  probabilityPercent: number | null;
  expectedCloseDate: string | null;
  source: OpportunitySource | null;
  sourceLabel: string | null;
  notes: string | null;
}

// Create flattens the patch on the wire; stage defaults to first open stage.
export type CreateOpportunityRequest = {
  actor?: Actor;
  stageId?: string | null;
} & OpportunityPatch;

export interface UpdateOpportunityRequest {
  actor?: Actor;
  opportunityId: string;
  expectedVersion: number;
  patch: OpportunityPatch;
}

export interface MoveOpportunityStageRequest {
  actor?: Actor;
  opportunityId: string;
  toStageId: string;
  // Required when the target stage kind is "lost".
  lostReasonId: string | null;
  expectedVersion: number;
}

// Rename or reorder one stage; kind and pipeline are fixed in v1.
export interface UpdateStageRequest {
  actor?: Actor;
  stageId: string;
  expectedVersion: number;
  name: string;
  sortKey: number;
}

// Needs-attention thresholds (src-tauri/src/attention.rs); persisted in
// app_settings, defaults 21 / 7 / "Proposal Sent".
export interface AttentionThresholds {
  staleLeadDays: number;
  proposalNoResponseDays: number;
  proposalStageName: string;
}

export interface SetAttentionThresholdsRequest {
  actor?: Actor;
  staleLeadDays: number;
  proposalNoResponseDays: number;
  proposalStageName?: string | null;
}

export type AttentionRule = "stale_lead" | "overdue_task" | "proposal_no_response";
export type AttentionRecordType = "contact" | "opportunity" | "task";

// One deterministic attention flag — computed on demand, never stored.
// Ordered by severity: overdue tasks, then proposals, then stale leads.
export interface AttentionFlag {
  id: string;
  rule: AttentionRule;
  recordType: AttentionRecordType;
  recordId: string;
  recordDisplayName: string;
  explanation: string;
}

// Error wire shape (CommandError in src-tauri/src/lib.rs): stable kind,
// human message, plus flattened per-kind details.
export type CommandError =
  | { kind: "invalid_input"; message: string; field: string }
  | { kind: "validation_failed"; message: string; code: string; field: string }
  | { kind: "not_found"; message: string; resource: string; recordId: string }
  | { kind: "missing_lost_reason"; message: string; resource: string; recordId: string }
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
