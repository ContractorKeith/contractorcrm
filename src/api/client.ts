import { invoke } from "@tauri-apps/api/core";

import type {
  ArchiveRequest,
  Company,
  Contact,
  CreateCompanyRequest,
  CreateContactRequest,
  CreateOpportunityRequest,
  HealthReport,
  LostReason,
  MoveOpportunityStageRequest,
  Opportunity,
  OpportunityDetail,
  OpportunityListItem,
  Stage,
  UpdateCompanyRequest,
  UpdateContactRequest,
  UpdateOpportunityRequest,
  UpdateStageRequest,
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
  listStages(): Promise<Stage[]>;
  updateStage(request: UpdateStageRequest): Promise<Stage>;
  listLostReasons(): Promise<LostReason[]>;
  createOpportunity(request: CreateOpportunityRequest): Promise<Opportunity>;
  updateOpportunity(request: UpdateOpportunityRequest): Promise<Opportunity>;
  archiveOpportunity(request: ArchiveRequest): Promise<Opportunity>;
  unarchiveOpportunity(request: ArchiveRequest): Promise<Opportunity>;
  listOpportunities(includeArchived: boolean): Promise<OpportunityListItem[]>;
  getOpportunity(opportunityId: string): Promise<OpportunityDetail>;
  moveOpportunityStage(request: MoveOpportunityStageRequest): Promise<Opportunity>;
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
  listStages: () => invoke("list_stages"),
  updateStage: (request) => invoke("update_stage", { request }),
  listLostReasons: () => invoke("list_lost_reasons"),
  createOpportunity: (request) => invoke("create_opportunity", { request }),
  updateOpportunity: (request) => invoke("update_opportunity", { request }),
  archiveOpportunity: (request) => invoke("archive_opportunity", { request }),
  unarchiveOpportunity: (request) => invoke("unarchive_opportunity", { request }),
  listOpportunities: (includeArchived) => invoke("list_opportunities", { includeArchived }),
  getOpportunity: (opportunityId) => invoke("get_opportunity", { opportunityId }),
  moveOpportunityStage: (request) => invoke("move_opportunity_stage", { request }),
};
