import { invoke } from "@tauri-apps/api/core";

import type {
  Activity,
  ArchiveRequest,
  AttentionFlag,
  AttentionThresholds,
  Company,
  CompleteTaskRequest,
  Contact,
  ContactListItem,
  CreateCompanyRequest,
  CreateContactRequest,
  CreateOpportunityRequest,
  CreateSavedViewRequest,
  CreateTaskRequest,
  DeleteActivityRequest,
  DeleteSavedViewRequest,
  EnvelopeExportReport,
  HealthReport,
  SearchEntityType,
  SearchResult,
  SavedView,
  SavedViewEntityType,
  NavigationEntityType,
  LinkJobRequest,
  LinkQuoteRequest,
  ListTasksRequest,
  LogActivityRequest,
  LostReason,
  MoveOpportunityStageRequest,
  Opportunity,
  OpportunityDetail,
  OpportunityListItem,
  ParentType,
  SetAttentionThresholdsRequest,
  Stage,
  Task,
  TaskActionRequest,
  UnlinkHandoffRequest,
  UpdateActivityRequest,
  UpdateCompanyRequest,
  UpdateContactRequest,
  UpdateOpportunityRequest,
  UpdateSavedViewRequest,
  UpdateStageRequest,
  UpdateTaskRequest,
  Tag,
  CreateTagRequest,
  UpdateTagRequest,
  TagLifecycleRequest,
  CustomFieldDefinition,
  CreateCustomFieldDefinitionRequest,
  UpdateCustomFieldDefinitionRequest,
  CustomFieldDefinitionLifecycleRequest,
  RecordMetadata,
  SetRecordMetadataRequest,
  ContactImportMapping,
  ContactImportPreview,
  ContactImportSummary,
  CsvExportReport,
  ImportContactsRequest,
  ArchiveExportReport,
  ArchiveImportPreview,
  ArchiveImportReport,
  AddAttachmentRequest,
  Attachment,
  AttachmentLocation,
  AttachmentParentType,
  AttachmentRemoval,
  RemoveAttachmentRequest,
  AiSettings,
  SetAiSettingsRequest,
  ProviderCheck,
  AttentionExplanation,
} from "./types";

// Seam for talking to the Rust core; tests inject a fake, the app uses Tauri.
// Components never call invoke() directly — everything routes through here.
export interface CoreClient {
  health(): Promise<HealthReport>;
  searchRecords(query: string, entityTypes?: SearchEntityType[], limit?: number): Promise<SearchResult[]>;
  listRecentRecords(): Promise<SearchResult[]>;
  recordRecent(entityType: NavigationEntityType, entityId: string): Promise<void>;
  listFavoriteContacts(): Promise<SearchResult[]>;
  listSavedViews(entityType: SavedViewEntityType): Promise<SavedView[]>;
  createSavedView(request: CreateSavedViewRequest): Promise<SavedView>;
  updateSavedView(request: UpdateSavedViewRequest): Promise<SavedView>;
  deleteSavedView(request: DeleteSavedViewRequest): Promise<void>;
  listTags(includeArchived: boolean): Promise<Tag[]>;
  createTag(request: CreateTagRequest): Promise<Tag>;
  updateTag(request: UpdateTagRequest): Promise<Tag>;
  archiveTag(request: TagLifecycleRequest): Promise<Tag>;
  unarchiveTag(request: TagLifecycleRequest): Promise<Tag>;
  listCustomFieldDefs(entityType: SavedViewEntityType, includeArchived: boolean): Promise<CustomFieldDefinition[]>;
  createCustomFieldDef(request: CreateCustomFieldDefinitionRequest): Promise<CustomFieldDefinition>;
  updateCustomFieldDef(request: UpdateCustomFieldDefinitionRequest): Promise<CustomFieldDefinition>;
  archiveCustomFieldDef(request: CustomFieldDefinitionLifecycleRequest): Promise<CustomFieldDefinition>;
  unarchiveCustomFieldDef(request: CustomFieldDefinitionLifecycleRequest): Promise<CustomFieldDefinition>;
  getRecordMetadata(entityType: SavedViewEntityType, recordId: string): Promise<RecordMetadata>;
  setRecordMetadata(request: SetRecordMetadataRequest): Promise<RecordMetadata>;
  matchSavedView(entityType: SavedViewEntityType, definition: import("./types").SavedViewDefinition): Promise<string[]>;
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
  listContacts(includeArchived: boolean): Promise<ContactListItem[]>;
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
  linkQuote(request: LinkQuoteRequest): Promise<Opportunity>;
  unlinkQuote(request: UnlinkHandoffRequest): Promise<Opportunity>;
  linkJob(request: LinkJobRequest): Promise<Opportunity>;
  unlinkJob(request: UnlinkHandoffRequest): Promise<Opportunity>;
  exportHandoffEnvelope(
    opportunityId: string,
    destinationPath: string,
    overwrite: boolean,
  ): Promise<EnvelopeExportReport>;
  logActivity(request: LogActivityRequest): Promise<Activity>;
  updateActivity(request: UpdateActivityRequest): Promise<Activity>;
  deleteActivity(request: DeleteActivityRequest): Promise<void>;
  getTimeline(parentType: ParentType, parentId: string, includeRelated: boolean): Promise<Activity[]>;
  createTask(request: CreateTaskRequest): Promise<Task>;
  updateTask(request: UpdateTaskRequest): Promise<Task>;
  completeTask(request: CompleteTaskRequest): Promise<Task>;
  reopenTask(request: TaskActionRequest): Promise<Task>;
  dropTask(request: TaskActionRequest): Promise<Task>;
  deleteTask(request: TaskActionRequest): Promise<void>;
  listTasks(request: ListTasksRequest): Promise<Task[]>;
  getAttentionFlags(referenceTime?: string): Promise<AttentionFlag[]>;
  getAttentionThresholds(): Promise<AttentionThresholds>;
  setAttentionThresholds(request: SetAttentionThresholdsRequest): Promise<AttentionThresholds>;
  previewContactImport(path: string, mapping?: ContactImportMapping | null): Promise<ContactImportPreview>;
  importContacts(request: ImportContactsRequest): Promise<ContactImportSummary>;
  exportContactsCsv(path: string, overwrite: boolean): Promise<CsvExportReport>;
  exportOpportunitiesCsv(path: string, overwrite: boolean): Promise<CsvExportReport>;
  exportArchive(path: string, overwrite: boolean): Promise<ArchiveExportReport>;
  previewArchiveImport(path: string): Promise<ArchiveImportPreview>;
  importArchive(path: string): Promise<ArchiveImportReport>;
  addAttachment(request: AddAttachmentRequest): Promise<Attachment>;
  listAttachments(parentType: AttachmentParentType, parentId: string): Promise<Attachment[]>;
  removeAttachment(request: RemoveAttachmentRequest): Promise<AttachmentRemoval>;
  attachmentPath(attachmentId: string): Promise<AttachmentLocation>;
  getAiSettings(): Promise<AiSettings>;
  setAiSettings(request: SetAiSettingsRequest): Promise<AiSettings>;
  setAiApiKey(apiKey: string): Promise<AiSettings>;
  clearAiApiKey(): Promise<AiSettings>;
  testAiProvider(): Promise<ProviderCheck>;
  explainAttentionFlag(flagId: string): Promise<AttentionExplanation>;
}

// Production client — one invoke per registered Tauri command.
export const tauriCoreClient: CoreClient = {
  health: () => invoke("health"),
  searchRecords: (query, entityTypes, limit) =>
    invoke("search_records", { query, entityTypes, limit }),
  listRecentRecords: () => invoke("list_recent_records"),
  recordRecent: (entityType, entityId) => invoke("record_recent", { entityType, entityId }),
  listFavoriteContacts: () => invoke("list_favorite_contacts"),
  listSavedViews: (entityType) => invoke("list_saved_views", { entityType }),
  createSavedView: (request) => invoke("create_saved_view", { request }),
  updateSavedView: (request) => invoke("update_saved_view", { request }),
  deleteSavedView: (request) => invoke("delete_saved_view", { request }),
  listTags: (includeArchived) => invoke("list_tags", { includeArchived }),
  createTag: (request) => invoke("create_tag", { request }),
  updateTag: (request) => invoke("update_tag", { request }),
  archiveTag: (request) => invoke("archive_tag", { request }),
  unarchiveTag: (request) => invoke("unarchive_tag", { request }),
  listCustomFieldDefs: (entityType, includeArchived) => invoke("list_custom_field_defs", { entityType, includeArchived }),
  createCustomFieldDef: (request) => invoke("create_custom_field_def", { request }),
  updateCustomFieldDef: (request) => invoke("update_custom_field_def", { request }),
  archiveCustomFieldDef: (request) => invoke("archive_custom_field_def", { request }),
  unarchiveCustomFieldDef: (request) => invoke("unarchive_custom_field_def", { request }),
  getRecordMetadata: (entityType, recordId) => invoke("get_record_metadata", { entityType, recordId }),
  setRecordMetadata: (request) => invoke("set_record_metadata", { request }),
  matchSavedView: (entityType, definition) => invoke("match_saved_view", { entityType, definition }),
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
  linkQuote: (request) => invoke("link_quote", { request }),
  unlinkQuote: (request) => invoke("unlink_quote", { request }),
  linkJob: (request) => invoke("link_job", { request }),
  unlinkJob: (request) => invoke("unlink_job", { request }),
  exportHandoffEnvelope: (opportunityId, destinationPath, overwrite) =>
    invoke("export_handoff_envelope", { opportunityId, destinationPath, overwrite }),
  logActivity: (request) => invoke("log_activity", { request }),
  updateActivity: (request) => invoke("update_activity", { request }),
  deleteActivity: (request) => invoke("delete_activity", { request }),
  getTimeline: (parentType, parentId, includeRelated) =>
    invoke("get_timeline", { parentType, parentId, includeRelated }),
  createTask: (request) => invoke("create_task", { request }),
  updateTask: (request) => invoke("update_task", { request }),
  completeTask: (request) => invoke("complete_task", { request }),
  reopenTask: (request) => invoke("reopen_task", { request }),
  dropTask: (request) => invoke("drop_task", { request }),
  deleteTask: (request) => invoke("delete_task", { request }),
  listTasks: (request) => invoke("list_tasks", { request }),
  getAttentionFlags: (referenceTime) => invoke("get_attention_flags", { referenceTime }),
  getAttentionThresholds: () => invoke("get_attention_thresholds"),
  setAttentionThresholds: (request) => invoke("set_attention_thresholds", { request }),
  previewContactImport: (path, mapping) => invoke("preview_contact_import", { path, mapping }),
  importContacts: (request) => invoke("import_contacts", { request }),
  exportContactsCsv: (path, overwrite) => invoke("export_contacts_csv", { path, overwrite }),
  exportOpportunitiesCsv: (path, overwrite) =>
    invoke("export_opportunities_csv", { path, overwrite }),
  exportArchive: (path, overwrite) => invoke("export_archive", { path, overwrite }),
  previewArchiveImport: (path) => invoke("preview_archive_import", { path }),
  importArchive: (path) => invoke("import_archive", { path }),
  addAttachment: (request) => invoke("add_attachment", { request }),
  listAttachments: (parentType, parentId) => invoke("list_attachments", { parentType, parentId }),
  removeAttachment: (request) => invoke("remove_attachment", { request }),
  attachmentPath: (attachmentId) => invoke("attachment_path", { attachmentId }),
  getAiSettings: () => invoke("get_ai_settings"),
  setAiSettings: (request) => invoke("set_ai_settings", { request }),
  setAiApiKey: (apiKey) => invoke("set_ai_api_key", { apiKey }),
  clearAiApiKey: () => invoke("clear_ai_api_key"),
  testAiProvider: () => invoke("test_ai_provider"),
  explainAttentionFlag: (flagId) => invoke("explain_attention_flag", { flagId }),
};
