import { invoke } from "@tauri-apps/api/core";

import type {
  ArchiveRequest,
  Company,
  Contact,
  CreateCompanyRequest,
  CreateContactRequest,
  HealthReport,
  UpdateCompanyRequest,
  UpdateContactRequest,
} from "./types";

// Seam for talking to the Rust core; tests inject a fake, the app uses Tauri.
// Components never call invoke() directly — everything routes through here.
export interface CoreClient {
  health(): Promise<HealthReport>;
  createCompany(request: CreateCompanyRequest): Promise<Company>;
  updateCompany(request: UpdateCompanyRequest): Promise<Company>;
  archiveCompany(request: ArchiveRequest): Promise<Company>;
  unarchiveCompany(request: ArchiveRequest): Promise<Company>;
  listCompanies(includeArchived: boolean): Promise<Company[]>;
  getCompany(companyId: string): Promise<Company>;
  createContact(request: CreateContactRequest): Promise<Contact>;
  updateContact(request: UpdateContactRequest): Promise<Contact>;
  archiveContact(request: ArchiveRequest): Promise<Contact>;
  unarchiveContact(request: ArchiveRequest): Promise<Contact>;
  listContacts(includeArchived: boolean): Promise<Contact[]>;
  getContact(contactId: string): Promise<Contact>;
}

// Production client — one invoke per registered Tauri command.
export const tauriCoreClient: CoreClient = {
  health: () => invoke("health"),
  createCompany: (request) => invoke("create_company", { request }),
  updateCompany: (request) => invoke("update_company", { request }),
  archiveCompany: (request) => invoke("archive_company", { request }),
  unarchiveCompany: (request) => invoke("unarchive_company", { request }),
  listCompanies: (includeArchived) => invoke("list_companies", { includeArchived }),
  getCompany: (companyId) => invoke("get_company", { companyId }),
  createContact: (request) => invoke("create_contact", { request }),
  updateContact: (request) => invoke("update_contact", { request }),
  archiveContact: (request) => invoke("archive_contact", { request }),
  unarchiveContact: (request) => invoke("unarchive_contact", { request }),
  listContacts: (includeArchived) => invoke("list_contacts", { includeArchived }),
  getContact: (contactId) => invoke("get_contact", { contactId }),
};
