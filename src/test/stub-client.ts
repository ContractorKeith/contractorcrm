import { vi } from "vitest";

import type { CoreClient } from "../api/client";
import type { Company, Contact } from "../api/types";

// Fully-stubbed CoreClient; tests override the methods they care about.
export const stubClient = (overrides: Partial<CoreClient> = {}): CoreClient => ({
  health: vi.fn().mockResolvedValue({ app: "ContractorCRM", version: "0.1.0", status: "ok" }),
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
