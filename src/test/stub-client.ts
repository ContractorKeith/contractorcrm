import { vi } from "vitest";

import type { CoreClient } from "../api/client";
import type {
  Activity,
  Attachment,
  AttentionExplanation,
  ContextPreview,
  FollowupDraft,
  FollowupTemplates,
  HistorySummary,
  Proposal,
  AttentionFlag,
  Company,
  Contact,
  HandoffRef,
  LostReason,
  OpportunityDetail,
  OpportunityListItem,
  Stage,
  StageHistoryEntry,
  Task,
} from "../api/types";

// Fully-stubbed CoreClient; tests override the methods they care about.
export const stubClient = (overrides: Partial<CoreClient> = {}): CoreClient => ({
  health: vi.fn().mockResolvedValue({ app: "ContractorCRM", version: "0.1.0", status: "ok" }),
  searchRecords: vi.fn().mockResolvedValue([]),
  listRecentRecords: vi.fn().mockResolvedValue([]),
  recordRecent: vi.fn(),
  listFavoriteContacts: vi.fn().mockResolvedValue([]),
  listSavedViews: vi.fn().mockResolvedValue([]),
  createSavedView: vi.fn(),
  updateSavedView: vi.fn(),
  deleteSavedView: vi.fn(),
  listTags: vi.fn().mockResolvedValue([]),
  createTag: vi.fn(),
  updateTag: vi.fn(),
  archiveTag: vi.fn(),
  unarchiveTag: vi.fn(),
  listCustomFieldDefs: vi.fn().mockResolvedValue([]),
  createCustomFieldDef: vi.fn(),
  updateCustomFieldDef: vi.fn(),
  archiveCustomFieldDef: vi.fn(),
  unarchiveCustomFieldDef: vi.fn(),
  getRecordMetadata: vi.fn().mockResolvedValue({ tagIds: [], values: [] }),
  setRecordMetadata: vi.fn(),
  matchSavedView: vi.fn().mockResolvedValue([]),
  createCompany: vi.fn(),
  updateCompany: vi.fn(),
  archiveCompany: vi.fn(),
  unarchiveCompany: vi.fn(),
  listCompanies: vi.fn().mockResolvedValue([]),
  getCompany: vi.fn(),
  createContact: vi.fn(),
  updateContact: vi.fn(),
  archiveContact: vi.fn(),
  unarchiveContact: vi.fn(),
  listContacts: vi.fn().mockResolvedValue([]),
  getContact: vi.fn(),
  listStages: vi.fn().mockResolvedValue(makeStages()),
  updateStage: vi.fn(),
  listLostReasons: vi.fn().mockResolvedValue([]),
  createOpportunity: vi.fn(),
  updateOpportunity: vi.fn(),
  archiveOpportunity: vi.fn(),
  unarchiveOpportunity: vi.fn(),
  listOpportunities: vi.fn().mockResolvedValue([]),
  getOpportunity: vi.fn(),
  moveOpportunityStage: vi.fn(),
  linkQuote: vi.fn(),
  unlinkQuote: vi.fn(),
  linkJob: vi.fn(),
  unlinkJob: vi.fn(),
  exportHandoffEnvelope: vi.fn(),
  logActivity: vi.fn(),
  updateActivity: vi.fn(),
  deleteActivity: vi.fn(),
  getTimeline: vi.fn().mockResolvedValue([]),
  createTask: vi.fn(),
  updateTask: vi.fn(),
  completeTask: vi.fn(),
  reopenTask: vi.fn(),
  dropTask: vi.fn(),
  deleteTask: vi.fn(),
  listTasks: vi.fn().mockResolvedValue([]),
  getAttentionFlags: vi.fn().mockResolvedValue([]),
  getAttentionThresholds: vi.fn().mockResolvedValue({
    staleLeadDays: 21,
    proposalNoResponseDays: 7,
    proposalStageName: "Proposal Sent",
  }),
  setAttentionThresholds: vi.fn(),
  previewContactImport: vi.fn().mockResolvedValue({
    headers: [],
    rowCount: 0,
    mapping: {},
    sampleRows: [],
    issues: [],
  }),
  importContacts: vi.fn().mockResolvedValue({ created: 0, updated: 0, skipped: [] }),
  exportContactsCsv: vi.fn().mockResolvedValue({ path: "", rowCount: 0 }),
  exportOpportunitiesCsv: vi.fn().mockResolvedValue({ path: "", rowCount: 0 }),
  exportArchive: vi.fn().mockResolvedValue({ path: "", recordCounts: {}, fileCount: 0 }),
  previewArchiveImport: vi.fn().mockResolvedValue({
    schemaVersion: 1,
    product: { name: "ContractorCRM", version: "0.1.0" },
    exportedAt: "2026-08-18T12:00:00Z",
    databaseMigrationVersion: 9,
    recordCounts: {},
    issues: [],
  }),
  importArchive: vi.fn().mockResolvedValue({ recordCounts: {}, safetyBackupPath: "" }),
  getDatabaseInfo: vi.fn().mockResolvedValue({
    databasePath: "/Users/sam/Library/Application Support/ContractorCRM/contractorcrm.sqlite3",
    fileSizeBytes: 1024,
    lastBackupAt: null,
  }),
  getAgentHelperPath: vi
    .fn()
    .mockResolvedValue("/Applications/ContractorCRM.app/Contents/MacOS/contractorcrm-mcp"),
  addAttachment: vi.fn(),
  listAttachments: vi.fn().mockResolvedValue([]),
  removeAttachment: vi.fn().mockResolvedValue({ fileRemoved: true }),
  attachmentPath: vi.fn().mockResolvedValue({ path: "", exists: true }),
  getAiSettings: vi.fn().mockResolvedValue({
    version: 1,
    enabled: false,
    providerLabel: "Local model",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "",
    hasApiKey: false,
  }),
  setAiSettings: vi.fn(),
  setAiApiKey: vi.fn(),
  clearAiApiKey: vi.fn(),
  testAiProvider: vi.fn(),
  proposeRecord: vi.fn(),
  proposeUpdate: vi.fn(),
  applyProposal: vi.fn(),
  undoProposal: vi.fn(),
  explainAttentionFlag: vi.fn().mockResolvedValue(makeAttentionExplanation()),
  getFollowupTemplates: vi.fn().mockResolvedValue(makeFollowupTemplates()),
  setFollowupTemplates: vi.fn().mockResolvedValue(makeFollowupTemplates()),
  summarizeHistory: vi.fn().mockResolvedValue(makeHistorySummary()),
  proposeFollowup: vi.fn().mockResolvedValue(makeFollowupDraft()),
  previewContext: vi.fn().mockResolvedValue(makeContextPreview()),
  ...overrides,
});

// Record factories with sensible defaults, overridable per test.
export const makeCompany = (overrides: Partial<Company> = {}): Company => ({
  id: "company-1",
  name: "Coastal Fence Co",
  kind: "client",
  phone: "555-0100",
  email: "office@coastalfence.test",
  website: null,
  addressLine1: null,
  addressLine2: null,
  city: null,
  state: null,
  postalCode: null,
  serviceArea: "Central Florida",
  licenseNotes: null,
  notes: null,
  archivedAt: null,
  createdAt: "2026-08-14T12:00:00Z",
  updatedAt: "2026-08-14T12:00:00Z",
  version: 1,
  ...overrides,
});

export const makeContact = (overrides: Partial<Contact> = {}): Contact => ({
  id: "contact-1",
  companyId: null,
  firstName: "Dana",
  lastName: "Ruiz",
  displayName: "Dana Ruiz",
  role: "owner",
  kind: "client",
  preferredContactMethod: null,
  addressLine1: null,
  addressLine2: null,
  city: null,
  state: null,
  postalCode: null,
  propertyType: null,
  notes: null,
  favorite: false,
  archivedAt: null,
  createdAt: "2026-08-14T12:00:00Z",
  updatedAt: "2026-08-14T12:00:00Z",
  version: 1,
  channels: [],
  ...overrides,
});

// Default pipeline: three open stages, then won and lost.
export const makeStages = (): Stage[] =>
  (
    [
      ["stage-new", "New lead", 0, "open"],
      ["stage-estimating", "Estimating", 1, "open"],
      ["stage-quoted", "Quoted", 2, "open"],
      ["stage-won", "Won", 3, "won"],
      ["stage-lost", "Lost", 4, "lost"],
    ] as const
  ).map(([id, name, sortKey, kind]) => ({
    id,
    pipelineId: "pipeline-1",
    name,
    sortKey,
    kind,
    createdAt: "2026-08-14T12:00:00Z",
    updatedAt: "2026-08-14T12:00:00Z",
    version: 1,
  }));

export const makeLostReason = (overrides: Partial<LostReason> = {}): LostReason => ({
  id: "reason-1",
  label: "Price too high",
  sortKey: 0,
  active: true,
  ...overrides,
});

export const makeOpportunity = (
  overrides: Partial<OpportunityListItem> = {},
): OpportunityListItem => ({
  id: "opp-1",
  name: "Backyard fence",
  contactId: "contact-1",
  companyId: null,
  stageId: "stage-new",
  value: { valueMinor: 123456, currencyCode: "USD" },
  probabilityPercent: 50,
  expectedCloseDate: "2026-09-01",
  source: "referral",
  sourceLabel: null,
  lostReasonId: null,
  notes: null,
  quoteRef: null,
  jobRef: null,
  archivedAt: null,
  createdAt: "2026-08-14T12:00:00Z",
  updatedAt: "2026-08-14T12:00:00Z",
  version: 1,
  stageName: "New lead",
  contactDisplayName: "Dana Ruiz",
  companyName: null,
  ...overrides,
});

// Stored hand-off reference default — a quote in an external quoting tool.
export const makeHandoffRef = (overrides: Partial<HandoffRef> = {}): HandoffRef => ({
  tool: "quoter",
  externalId: "Q-123",
  label: null,
  linkedAt: "2026-08-14T17:55:00Z",
  ...overrides,
});

export const makeStageHistoryEntry = (
  overrides: Partial<StageHistoryEntry> = {},
): StageHistoryEntry => ({
  id: "history-1",
  opportunityId: "opp-1",
  fromStageId: null,
  toStageId: "stage-new",
  actor: "user",
  lostReasonId: null,
  createdAt: "2026-08-14T12:00:00Z",
  ...overrides,
});

export const makeActivity = (overrides: Partial<Activity> = {}): Activity => ({
  id: "act-1",
  parentType: "contact",
  parentId: "contact-1",
  kind: "call",
  direction: "outbound",
  occurredAt: "2026-08-14T15:00:00Z",
  summary: "Called about the estimate",
  body: null,
  actor: "user",
  createdAt: "2026-08-14T15:00:00Z",
  updatedAt: "2026-08-14T15:00:00Z",
  version: 1,
  ...overrides,
});

export const makeTask = (overrides: Partial<Task> = {}): Task => ({
  id: "task-1",
  title: "Follow up with Dana",
  body: null,
  parentType: "contact",
  parentId: "contact-1",
  dueAt: "2026-08-20T16:00:00Z",
  remindAt: null,
  priority: "normal",
  status: "open",
  completedAt: null,
  createdAt: "2026-08-14T12:00:00Z",
  updatedAt: "2026-08-14T12:00:00Z",
  version: 1,
  ...overrides,
});

export const makeAttachment = (overrides: Partial<Attachment> = {}): Attachment => ({
  id: "attachment-1",
  parentType: "contact",
  parentId: "contact-1",
  fileName: "site-plan.pdf",
  mediaType: "application/pdf",
  sizeBytes: 2048,
  sha256: "abc123",
  createdAt: "2026-08-14T12:00:00Z",
  version: 1,
  ...overrides,
});

export const makeAttentionFlag = (overrides: Partial<AttentionFlag> = {}): AttentionFlag => ({
  id: "flag-1",
  rule: "overdue_task",
  recordType: "task",
  recordId: "task-1",
  recordDisplayName: "Follow up with Dana",
  explanation: 'Task "Follow up with Dana" is overdue.',
  ...overrides,
});

export const makeAttentionExplanation = (
  overrides: Partial<AttentionExplanation> = {},
): AttentionExplanation => ({
  flagId: "flag-1",
  endpointHost: "127.0.0.1:11434",
  local: true,
  explanation: {
    purpose: "explain_attention_flag",
    model: "llama3.1",
    text: "This task is three days past due — call the county office today.",
    includedRecordRefs: [
      { entityType: "task", entityId: "task-1", label: "Follow up with Dana" },
    ],
  },
  ...overrides,
});

export const makeOpportunityDetail = (
  overrides: Partial<OpportunityDetail> = {},
): OpportunityDetail => {
  const { stageName: _s, contactDisplayName: _c, companyName: _n, ...record } = makeOpportunity();
  return { ...record, stageHistory: [makeStageHistoryEntry()], ...overrides };
};

// Built-in follow-up templates, matching the core's defaults closely enough
// for the editor and the drafting flow to be exercised.
export const makeFollowupTemplates = (
  overrides: Partial<FollowupTemplates> = {},
): FollowupTemplates => ({
  version: 1,
  templates: [
    { id: "call_followup", name: "Call follow-up", body: "Thanks for taking the time on the phone." },
    { id: "proposal_chaser", name: "Proposal follow-up", body: "Checking in on the proposal." },
    { id: "site_visit_note", name: "Post-site-visit note", body: "Thanks for having me out." },
  ],
  ...overrides,
});

export const makeContextPreview = (
  overrides: Partial<ContextPreview> = {},
): ContextPreview => ({
  purpose: "summarize_history",
  contextText:
    "Record: Dana Ruiz (contact contact-1)\nActivity (newest first):\n- 2026-06-02 call: Walked the back fence line",
  includedRecordRefs: [{ entityType: "contact", entityId: "contact-1", label: "Dana Ruiz" }],
  ...overrides,
});

export const makeHistorySummary = (overrides: Partial<HistorySummary> = {}): HistorySummary => ({
  parentType: "contact",
  parentId: "contact-1",
  endpointHost: "127.0.0.1:11434",
  local: true,
  model: "llama3.1",
  summary: "Dana asked for a gate quote in June and has gone quiet since.",
  suggestedNextActions: ["Call Dana this week", "Send the revised quote"],
  includedRecordRefs: [{ entityType: "contact", entityId: "contact-1", label: "Dana Ruiz" }],
  ...overrides,
});

export const makeFollowupProposal = (overrides: Partial<Proposal> = {}): Proposal => ({
  id: "proposal-1",
  kind: "create_followup_task",
  entityType: "task",
  entityId: null,
  summary: "Follow up with Dana Ruiz",
  changes: [
    { field: "title", before: null, after: "Follow up: Dana Ruiz" },
    { field: "body", before: null, after: "Checking in on the proposal." },
  ],
  warnings: [],
  affectedVersions: [],
  createdAt: "2026-08-19T12:00:00.000Z",
  expiresAt: "2026-08-19T12:15:00.000Z",
  ...overrides,
});

export const makeFollowupDraft = (overrides: Partial<FollowupDraft> = {}): FollowupDraft => ({
  parentType: "contact",
  parentId: "contact-1",
  templateId: "proposal_chaser",
  templateName: "Proposal follow-up",
  draftText: "Checking in on the proposal.",
  usedProvider: true,
  endpointHost: "127.0.0.1:11434",
  local: true,
  model: "llama3.1",
  includedRecordRefs: [{ entityType: "contact", entityId: "contact-1", label: "Dana Ruiz" }],
  warnings: [],
  proposal: makeFollowupProposal(),
  ...overrides,
});
