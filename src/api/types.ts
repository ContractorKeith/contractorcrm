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

export type SearchEntityType = "contact" | "company" | "opportunity" | "activity";
export type NavigationEntityType = Exclude<SearchEntityType, "activity">;

// Lightweight FTS hit; fetch the canonical record separately when needed.
export interface SearchResult {
  entityType: SearchEntityType;
  entityId: string;
  title: string;
  parentType: "contact" | "company" | "opportunity" | null;
  parentId: string | null;
}

// A named, persisted list state. The definition is deliberately finite: it
// contains no SQL, column names, or operators supplied by callers.
export type SavedViewEntityType = "contact" | "company" | "opportunity";
export type SavedViewSortDirection = "ascending" | "descending";
export type SavedViewSortField = "displayName" | "name" | "stage" | "value" | "expectedClose";
export type TagColorRole = "neutral" | "accent" | "attention";
export type CustomFieldType = "text" | "number" | "date" | "select";

export interface Tag {
  id: string;
  label: string;
  colorRole: TagColorRole | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
  version: number;
}

export interface CustomFieldOption {
  id: string;
  definitionId: string;
  label: string;
  sortKey: number;
}

export interface CustomFieldDefinition {
  id: string;
  entityType: SavedViewEntityType;
  label: string;
  fieldType: CustomFieldType;
  sortKey: number;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
  version: number;
  options: CustomFieldOption[];
}

export interface RecordCustomFieldValue {
  id?: string;
  definitionId: string;
  entityType?: SavedViewEntityType;
  recordId?: string;
  textValue: string | null;
  numberValue: number | null;
  dateValue: string | null;
  optionId: string | null;
  createdAt?: string;
  updatedAt?: string;
}

export interface RecordMetadata {
  tagIds: string[];
  values: RecordCustomFieldValue[];
}

export interface CustomFieldValueInput {
  definitionId: string;
  textValue: string | null;
  numberValue: number | null;
  dateValue: string | null;
  optionId: string | null;
}

export interface CreateTagRequest { actor?: Actor; label: string; colorRole?: TagColorRole | null; }
export interface UpdateTagRequest { actor?: Actor; tagId: string; expectedVersion: number; label: string; colorRole?: TagColorRole | null; }
export interface TagLifecycleRequest { actor?: Actor; tagId: string; expectedVersion: number; }
export interface CustomFieldOptionInput { id?: string; label: string; }
export interface CreateCustomFieldDefinitionRequest {
  actor?: Actor; entityType: SavedViewEntityType; label: string; fieldType: CustomFieldType; options?: CustomFieldOptionInput[];
}
export interface UpdateCustomFieldDefinitionRequest {
  actor?: Actor; definitionId: string; expectedVersion: number; label: string; sortKey: number; options: CustomFieldOptionInput[];
}
export interface CustomFieldDefinitionLifecycleRequest { actor?: Actor; definitionId: string; expectedVersion: number; }
export interface SetRecordMetadataRequest {
  actor?: Actor; entityType: SavedViewEntityType; recordId: string; expectedVersion: number;
  tagIds: string[]; values: CustomFieldValueInput[];
}

export type SavedViewCustomFieldPredicate =
  | { definitionId: string; fieldType: "text"; operator: "contains" | "equals"; value: string }
  | { definitionId: string; fieldType: "number"; operator: "equals" | "greaterThanOrEqual" | "lessThanOrEqual"; value: number }
  | { definitionId: string; fieldType: "date"; operator: "on" | "before" | "after"; value: string }
  | { definitionId: string; fieldType: "select"; operator: "is"; value: string };

export interface SavedViewDefinition {
  schemaVersion: 2;
  filter: {
    includeArchived: boolean;
    tagIdsAll: string[];
    customFields: SavedViewCustomFieldPredicate[];
  };
  sort: { field: SavedViewSortField; direction: SavedViewSortDirection };
}

export interface SavedView {
  id: string;
  name: string;
  entityType: SavedViewEntityType;
  definition: SavedViewDefinition;
  sortKey: number;
  createdAt: string;
  updatedAt: string;
  version: number;
}

export interface CreateSavedViewRequest {
  actor?: Actor;
  name: string;
  entityType: SavedViewEntityType;
  definition: SavedViewDefinition;
}

export interface UpdateSavedViewRequest {
  actor?: Actor;
  savedViewId: string;
  expectedVersion: number;
  name: string;
  definition: SavedViewDefinition;
}

export interface DeleteSavedViewRequest {
  actor?: Actor;
  savedViewId: string;
  expectedVersion: number;
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

// Stored hand-off reference — where a quote or job lives in an external tool.
export interface HandoffRef {
  tool: string;
  externalId: string;
  label: string | null;
  linkedAt: string;
}

// Caller-supplied reference for link commands; linkedAt is stamped on link.
export interface HandoffRefInput {
  tool: string;
  externalId: string;
  label?: string | null;
}

export interface LinkQuoteRequest {
  actor?: Actor;
  opportunityId: string;
  expectedVersion: number;
  quoteRef: HandoffRefInput;
}

export interface LinkJobRequest {
  actor?: Actor;
  opportunityId: string;
  expectedVersion: number;
  jobRef: HandoffRefInput;
}

// Shared shape for clearing either hand-off reference.
export interface UnlinkHandoffRequest {
  actor?: Actor;
  opportunityId: string;
  expectedVersion: number;
}

// Where an exported hand-off envelope landed (docs/HANDOFF.md).
export interface EnvelopeExportReport {
  destinationPath: string;
  schemaVersion: number;
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
  // Hand-off references to the external quote and job records, when linked.
  quoteRef: HandoffRef | null;
  jobRef: HandoffRef | null;
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

// Which record type an activity or task hangs off — its polymorphic parent.
export type ParentType = "contact" | "company" | "opportunity";

export type ActivityKind = "call" | "email" | "text" | "site_visit" | "meeting" | "note";
export type ActivityDirection = "inbound" | "outbound" | "none";

// One logged touch on a contact, company, or opportunity. occurredAt is
// user-editable UTC ISO-8601; timelines sort by it, not createdAt.
export interface Activity {
  id: string;
  parentType: ParentType;
  parentId: string;
  kind: ActivityKind;
  direction: ActivityDirection;
  occurredAt: string;
  summary: string;
  body: string | null;
  actor: Actor;
  createdAt: string;
  updatedAt: string;
  version: number;
}

// Editable activity fields; updates replace the full editable set (v1).
// direction defaults to "none" and occurredAt to now when absent.
export interface ActivityPatch {
  kind: ActivityKind;
  direction?: ActivityDirection;
  occurredAt?: string;
  summary: string;
  body: string | null;
}

// Log flattens the patch on the wire; the parent never changes after logging.
export type LogActivityRequest = {
  actor?: Actor;
  parentType: ParentType;
  parentId: string;
} & ActivityPatch;

export interface UpdateActivityRequest {
  actor?: Actor;
  activityId: string;
  expectedVersion: number;
  patch: ActivityPatch;
}

// Hard delete — activities are user notes, so there is no archive state.
export interface DeleteActivityRequest {
  actor?: Actor;
  activityId: string;
  expectedVersion: number;
}

export type TaskPriority = "low" | "normal" | "high";
export type TaskStatus = "open" | "done" | "dropped";

// A follow-up or to-do, optionally hanging off a contact, company, or
// opportunity; personal tasks have no parent. Timestamps are UTC ISO-8601.
export interface Task {
  id: string;
  title: string;
  body: string | null;
  parentType: ParentType | null;
  parentId: string | null;
  dueAt: string | null;
  remindAt: string | null;
  priority: TaskPriority;
  status: TaskStatus;
  completedAt: string | null;
  createdAt: string;
  updatedAt: string;
  version: number;
}

// Editable task fields; status moves through complete/reopen/drop, never
// through updates. Parent fields are set together or not at all.
export interface TaskPatch {
  title: string;
  body?: string | null;
  parentType?: ParentType | null;
  parentId?: string | null;
  dueAt?: string | null;
  remindAt?: string | null;
  priority?: TaskPriority;
}

// Create flattens the patch on the wire; actor defaults to "user".
export type CreateTaskRequest = { actor?: Actor } & TaskPatch;

export interface UpdateTaskRequest {
  actor?: Actor;
  taskId: string;
  expectedVersion: number;
  patch: TaskPatch;
}

export interface CompleteTaskRequest {
  actor?: Actor;
  taskId: string;
  expectedVersion: number;
  // Also log a "Completed task" note on the task's parent in the same
  // transaction; invalid for a task with no parent.
  logActivity?: boolean;
}

// Shared shape for reopen, drop, and hard delete of a task.
export interface TaskActionRequest {
  actor?: Actor;
  taskId: string;
  expectedVersion: number;
}

// Filter shape for list_tasks: absent status means every status; overdueOnly
// implies open + past dueAt; parent fields go together or not at all.
export interface ListTasksRequest {
  status?: TaskStatus;
  overdueOnly?: boolean;
  parentType?: ParentType;
  parentId?: string;
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

// ---------------------------------------------------------------------------
// CSV import / export
// ---------------------------------------------------------------------------

// Contact fields a CSV column can be mapped onto.
export type ContactImportTarget =
  | "externalId"
  | "firstName"
  | "lastName"
  | "displayName"
  | "role"
  | "kind"
  | "preferredContactMethod"
  | "addressLine1"
  | "addressLine2"
  | "city"
  | "state"
  | "postalCode"
  | "propertyType"
  | "notes"
  | "company"
  | "email"
  | "phone"
  | "tags";

// Each target names the CSV header it reads from; unset targets are not imported.
export type ContactImportMapping = Partial<Record<ContactImportTarget, string | null>>;

// One row the import refused, identified by its 1-based CSV line number.
export interface ContactImportIssue {
  line: number;
  reason: string;
}

// Read-only look at a CSV file before anything is written.
export interface ContactImportPreview {
  headers: string[];
  rowCount: number;
  mapping: ContactImportMapping;
  sampleRows: string[][];
  issues: ContactImportIssue[];
}

export interface ImportContactsRequest {
  actor?: Actor;
  path: string;
  mapping: ContactImportMapping;
}

// What an import did; skipped rows carry their line and reason.
export interface ContactImportSummary {
  created: number;
  updated: number;
  skipped: ContactImportIssue[];
}

// Where a CSV export landed and how many records it holds.
export interface CsvExportReport {
  path: string;
  rowCount: number;
}

// ---------------------------------------------------------------------------
// Portable archive (docs/DATA_MODEL.md "Archive contract")
// ---------------------------------------------------------------------------

// Which app and version wrote an archive.
export interface ProductInfo {
  name: string;
  version: string;
}

// Row count per canonical table, keyed by table name.
export type ArchiveRecordCounts = Record<string, number>;

// A machine-readable reason an archive cannot be imported.
export interface ArchiveIssue {
  code: string;
  message: string;
}

// Where an archive landed and what it holds.
export interface ArchiveExportReport {
  path: string;
  recordCounts: ArchiveRecordCounts;
  fileCount: number;
}

// Read-only verification of an archive; an empty `issues` list means it imports.
export interface ArchiveImportPreview {
  schemaVersion: number;
  product: ProductInfo;
  exportedAt: string;
  databaseMigrationVersion: number;
  recordCounts: ArchiveRecordCounts;
  issues: ArchiveIssue[];
}

// What an import wrote and where the pre-import safety backup went.
export interface ArchiveImportReport {
  recordCounts: ArchiveRecordCounts;
  safetyBackupPath: string;
}

// ---------------------------------------------------------------------------
// Attachments (managed file copies on contacts and opportunities)
// ---------------------------------------------------------------------------

export type AttachmentParentType = "contact" | "opportunity";

// One managed file. The path is never built by the UI — ask attachment_path.
export interface Attachment {
  id: string;
  parentType: AttachmentParentType;
  parentId: string;
  fileName: string;
  mediaType: string | null;
  sizeBytes: number;
  sha256: string;
  createdAt: string;
  version: number;
}

export interface AddAttachmentRequest {
  actor?: Actor;
  parentType: AttachmentParentType;
  parentId: string;
  sourcePath: string;
}

export interface RemoveAttachmentRequest {
  actor?: Actor;
  attachmentId: string;
  expectedVersion: number;
}

// Whether the managed file went with the row.
export interface AttachmentRemoval {
  fileRemoved: boolean;
}

// Absolute location of a managed file, for opening it with the OS.
export interface AttachmentLocation {
  path: string;
  exists: boolean;
}

// ---------------------------------------------------------------------------
// AI provider seam (src-tauri/src/ai.rs)
// ---------------------------------------------------------------------------

// Non-secret provider settings. The API key itself never crosses this seam:
// hasApiKey is derived from the OS credential store.
export interface AiSettings {
  version: number;
  enabled: boolean;
  providerLabel: string;
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
}

export interface SetAiSettingsRequest {
  actor?: Actor;
  enabled: boolean;
  providerLabel: string;
  baseUrl: string;
  model: string;
}

// One record whose facts were included in a provider call — the disclosure
// list shown with every explanation.
export interface RecordRef {
  entityType: string;
  entityId: string;
  label: string;
}

// One model completion plus the provenance shown next to it.
export interface ProviderCompletion {
  purpose: string;
  model: string;
  text: string;
  includedRecordRefs: RecordRef[];
}

// Plain-language explanation of one deterministic attention flag. Explanation
// only — the flag itself still comes from the rules, never from the model.
export interface AttentionExplanation {
  flagId: string;
  endpointHost: string;
  local: boolean;
  explanation: ProviderCompletion;
}

// Result of an explicit connection test — never run on its own.
export interface ProviderCheck {
  providerLabel: string;
  endpointHost: string;
  local: boolean;
  model: string;
  modelAvailable: boolean;
  availableModels: string[];
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
  | { kind: "provider_unavailable"; message: string; reason: string }
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
