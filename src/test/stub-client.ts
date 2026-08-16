import { vi } from "vitest";

import type { CoreClient } from "../api/client";
import type {
  Activity,
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

export const makeAttentionFlag = (overrides: Partial<AttentionFlag> = {}): AttentionFlag => ({
  id: "flag-1",
  rule: "overdue_task",
  recordType: "task",
  recordId: "task-1",
  recordDisplayName: "Follow up with Dana",
  explanation: 'Task "Follow up with Dana" is overdue.',
  ...overrides,
});

export const makeOpportunityDetail = (
  overrides: Partial<OpportunityDetail> = {},
): OpportunityDetail => {
  const { stageName: _s, contactDisplayName: _c, companyName: _n, ...record } = makeOpportunity();
  return { ...record, stageHistory: [makeStageHistoryEntry()], ...overrides };
};
