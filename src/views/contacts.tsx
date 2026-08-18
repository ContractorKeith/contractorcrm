import { useCallback, useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";

import type { CoreClient } from "../api/client";
import { isCommandError } from "../api/types";
import type {
  ChannelKind,
  Company,
  Contact,
  ContactListItem,
  ContactPatch,
  ContactRole,
  PartyKind,
  SavedViewCustomFieldPredicate,
  SavedViewDefinition,
  Tag,
  CustomFieldDefinition,
} from "../api/types";
import { CsvImportDialog } from "../components/CsvImportDialog";
import { RecordTable, type ColumnDef, type SortState } from "../components/RecordTable";
import { RecordMetadata } from "../components/RecordMetadata";
import { SavedViewFilters } from "../components/SavedViewFilters";
import { SavedViews } from "../components/SavedViews";
import { formatLocalDateTime } from "./date-format";
import { ActivityTimeline } from "./timeline";
import {
  CONTACT_ROLE_OPTIONS,
  ConflictBanner,
  Field,
  GeneralError,
  NO_SAVE_ERROR,
  PARTY_KIND_OPTIONS,
  contactRoleLabel,
  partyKindLabel,
  saveErrorFrom,
  type SaveError,
} from "./form-support";

// Preferred channel first, otherwise the first channel in sort order.
function bestChannel(contact: Contact): string {
  const channel = contact.channels.find((entry) => entry.preferred) ?? contact.channels[0];
  return channel ? channel.value : "—";
}

// ---------------------------------------------------------------------------
// Contact list
// ---------------------------------------------------------------------------

interface ContactsViewProps {
  client: CoreClient;
  onOpen: (contactId: string) => void;
  onCreate: () => void;
}

export function ContactsView({ client, onOpen, onCreate }: ContactsViewProps) {
  const [contacts, setContacts] = useState<ContactListItem[] | null>(null);
  const [companies, setCompanies] = useState<Company[]>([]);
  const [showArchived, setShowArchived] = useState(false);
  const [sort, setSort] = useState<SortState>({ key: "displayName", direction: "ascending" });
  const [savedViewApplied, setSavedViewApplied] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [tags, setTags] = useState<Tag[]>([]);
  const [fieldDefinitions, setFieldDefinitions] = useState<CustomFieldDefinition[]>([]);
  const [tagIdsAll, setTagIdsAll] = useState<string[]>([]);
  const [customFields, setCustomFields] = useState<SavedViewCustomFieldPredicate[]>([]);
  const [matchingIds, setMatchingIds] = useState<string[] | null>(null);
  const [filterError, setFilterError] = useState<string | null>(null);
  const [importPath, setImportPath] = useState<string | null>(null);
  const [csvStatus, setCsvStatus] = useState("");
  const [csvError, setCsvError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  useEffect(() => {
    let active = true;
    Promise.all([client.listContacts(showArchived), client.listCompanies(true)])
      .then(([contactRows, companyRows]) => {
        if (!active) return;
        setContacts(contactRows);
        setCompanies(companyRows);
        setLoadError(false);
      })
      .catch(() => {
        if (active) setLoadError(true);
      });
    return () => {
      active = false;
    };
  }, [client, showArchived, reloadToken]);

  useEffect(() => {
    void Promise.all([client.listTags(true), client.listCustomFieldDefs("contact", true)])
      .then(([nextTags, nextDefinitions]) => { setTags(nextTags); setFieldDefinitions(nextDefinitions); })
      .catch(() => setFilterError("Tags and custom-field filters could not be loaded."));
    // reloadToken re-runs this after an import, so imported tags show up in filters.
  }, [client, reloadToken]);

  const companyName = (companyId: string | null) =>
    companies.find((company) => company.id === companyId)?.name ?? "—";

  const definition: SavedViewDefinition = {
    schemaVersion: 2,
    filter: { includeArchived: showArchived, tagIdsAll, customFields },
    sort: { field: "displayName", direction: sort.direction },
  };
  const applyDefinition = (next: SavedViewDefinition) => {
    setShowArchived(next.filter.includeArchived);
    setTagIdsAll(next.filter.tagIdsAll ?? []);
    setCustomFields(next.filter.customFields ?? []);
    setSort({ key: "displayName", direction: next.sort.direction });
  };
  const rows = contacts
    ? contacts.filter((contact) => matchingIds === null || matchingIds.includes(contact.id)).sort((a, b) => {
        const compared = a.displayName.localeCompare(b.displayName) || a.id.localeCompare(b.id);
        return compared * (sort.direction === "ascending" ? 1 : -1);
      })
    : null;

  useEffect(() => {
    if (tagIdsAll.length === 0 && customFields.length === 0) { setMatchingIds(null); setFilterError(null); return; }
    void client.matchSavedView("contact", definition)
      .then((ids) => { setMatchingIds(ids); setFilterError(null); })
      .catch(() => { setMatchingIds([]); setFilterError("This filter references missing or invalid metadata and could not be applied."); });
  }, [client, definition.filter.includeArchived, tagIdsAll, customFields]);

  // Pick a CSV file, then hand it to the mapping wizard.
  const pickImportFile = async () => {
    setCsvError(null);
    setCsvStatus("");
    try {
      const picked = await open({
        multiple: false,
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (typeof picked === "string") setImportPath(picked);
    } catch (rejection) {
      setCsvError(
        isCommandError(rejection) ? rejection.message : "The file picker could not be opened.",
      );
    }
  };

  // Pick a destination, then write every contact to it. The native save dialog
  // already confirms replacement, so the export overwrites without asking again.
  const exportCsv = async () => {
    setCsvError(null);
    setCsvStatus("");
    try {
      const destination = await save({
        defaultPath: "contacts.csv",
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (typeof destination !== "string") return;
      const report = await client.exportContactsCsv(destination, true);
      setCsvStatus(`Exported ${report.rowCount} contacts to ${report.path}.`);
    } catch (rejection) {
      setCsvError(
        isCommandError(rejection) ? rejection.message : "The contacts could not be exported.",
      );
    }
  };

  const columns: ColumnDef<ContactListItem>[] = [
    {
      key: "displayName",
      header: "Name",
      sortable: true,
      render: (contact) => (
        <span className="cell-primary">
          {contact.displayName}
          {contact.archivedAt ? <span className="cell-flag">Archived</span> : null}
        </span>
      ),
    },
    { key: "company", header: "Company", render: (contact) => companyName(contact.companyId) },
    { key: "kind", header: "Kind", render: (contact) => partyKindLabel(contact.kind) },
    { key: "role", header: "Role", render: (contact) => contactRoleLabel(contact.role) },
    { key: "channel", header: "Preferred channel", render: bestChannel },
    // Read-time projections computed by the core from activities and tasks.
    {
      key: "lastContacted",
      header: "Last contacted",
      render: (contact) =>
        contact.lastContactedAt ? formatLocalDateTime(contact.lastContactedAt) : "—",
    },
    {
      key: "nextTask",
      header: "Next task",
      render: (contact) => contact.nextOpenTaskDueAt ?? "—",
    },
    {
      key: "favorite",
      header: "Favorite",
      render: (contact) => (contact.favorite ? "★" : "—"),
    },
  ];

  return (
    <section className="crm-section" aria-label="Contacts">
      <div className="section-rule">
        <h2>Contacts</h2>
        <div className="list-tools">
          <SavedViews
            client={client}
            entityType="contact"
            definition={definition}
            onApply={applyDefinition}
            onSelectionChange={setSavedViewApplied}
          />
          <SavedViewFilters
            entityType="contact"
            tags={tags}
            definitions={fieldDefinitions}
            definition={definition}
            onChange={applyDefinition}
          />
          {filterError ? <span role="alert" className="saved-views__error">{filterError}</span> : null}
          <label className="toggle">
            <input
              type="checkbox"
              checked={showArchived}
              onChange={(event) => setShowArchived(event.target.checked)}
            />
            <span>Show archived</span>
          </label>
          <span className="list-count">{contacts?.length ?? 0}</span>
          <button type="button" className="button" onClick={() => void pickImportFile()}>
            Import CSV…
          </button>
          <button type="button" className="button" onClick={() => void exportCsv()}>
            Export CSV…
          </button>
          <button type="button" className="button button--primary" onClick={onCreate}>
            New contact
          </button>
        </div>
      </div>

      <span className="saved-views__status" role="status" aria-live="polite">
        {csvStatus}
      </span>
      {csvError ? <span role="alert" className="saved-views__error">{csvError}</span> : null}

      {importPath ? (
        <CsvImportDialog
          client={client}
          path={importPath}
          onClose={(imported) => {
            setImportPath(null);
            if (imported) setReloadToken((token) => token + 1);
          }}
        />
      ) : null}

      {loadError ? (
        <GeneralError message="Could not read contacts from the local database." />
      ) : null}

      {rows && rows.length === 0 ? (
        <div className="empty-state">
          <span className="registration-mark" aria-hidden="true" />
          <p className="eyebrow">Ready when you are</p>
          <h2>{savedViewApplied ? "No contacts match this view" : "No contacts yet"}</h2>
          <p>{savedViewApplied ? "Change the current filters or choose another saved view." : "Add your first lead, client, sub, or vendor. Everything stays in this app's local database on this machine."}</p>
        </div>
      ) : null}

      {rows && rows.length > 0 ? (
        <RecordTable
          label="Contact list"
          columns={columns}
          rows={rows}
          onOpen={(contact) => onOpen(contact.id)}
          sort={sort}
          onSort={() =>
            setSort((current) => ({
              key: "displayName",
              direction: current.direction === "ascending" ? "descending" : "ascending",
            }))
          }
        />
      ) : null}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Contact detail
// ---------------------------------------------------------------------------

interface ContactDetailProps {
  client: CoreClient;
  contactId: string;
  onBack: () => void;
  onEdit: () => void;
}

export function ContactDetailView({ client, contactId, onBack, onEdit }: ContactDetailProps) {
  const [contact, setContact] = useState<Contact | null>(null);
  const [company, setCompany] = useState<Company | null>(null);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);

  const load = useCallback(() => {
    setError(NO_SAVE_ERROR);
    client
      .getContact(contactId)
      .then(async (record) => {
        setContact(record);
        setCompany(record.companyId ? await client.getCompany(record.companyId) : null);
      })
      .catch((rejection) => setError(saveErrorFrom(rejection)));
  }, [client, contactId]);

  useEffect(load, [load]);

  const toggleArchive = async () => {
    if (!contact) return;
    setError(NO_SAVE_ERROR);
    const request = { id: contact.id, expectedVersion: contact.version };
    try {
      const updated = contact.archivedAt
        ? await client.unarchiveContact(request)
        : await client.archiveContact(request);
      setContact(updated);
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  if (!contact) {
    return (
      <section className="crm-section" aria-label="Contact detail">
        <GeneralError message={error.general} />
      </section>
    );
  }

  const facts: [string, string][] = [
    ["Company", company?.name ?? "—"],
    ["Kind", partyKindLabel(contact.kind)],
    ["Role", contactRoleLabel(contact.role)],
    ["Preferred method", contact.preferredContactMethod ?? "—"],
    [
      "Address",
      [contact.addressLine1, contact.addressLine2, contact.city, contact.state, contact.postalCode]
        .filter(Boolean)
        .join(", ") || "—",
    ],
    ["Property type", contact.propertyType ?? "—"],
    ["Notes", contact.notes ?? "—"],
    ["Favorite", contact.favorite ? "★ Yes" : "No"],
  ];

  return (
    <section className="crm-section" aria-label="Contact detail">
      <div className="detail-head">
        <div>
          <p className="eyebrow">Contact</p>
          <h2 className="detail-title">
            {contact.displayName}
            {contact.archivedAt ? <span className="cell-flag">Archived</span> : null}
          </h2>
        </div>
        <div className="detail-actions">
          <button type="button" className="button" onClick={onBack}>
            Back
          </button>
          <button type="button" className="button" onClick={toggleArchive}>
            {contact.archivedAt ? "Unarchive" : "Archive"}
          </button>
          <button type="button" className="button button--primary" onClick={onEdit}>
            Edit
          </button>
        </div>
      </div>

      {error.conflict ? <ConflictBanner onReload={load} /> : null}
      <GeneralError message={error.general ?? Object.values(error.fields)[0] ?? null} />

      <dl className="detail-facts">
        {facts.map(([term, value]) => (
          <div key={term} className="detail-fact">
            <dt>{term}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>

      <RecordMetadata client={client} entityType="contact" recordId={contact.id} expectedVersion={contact.version} onSaved={load} />

      <h3 className="detail-subhead">Phone &amp; email</h3>
      {contact.channels.length === 0 ? (
        <p className="detail-empty">No phone or email on file.</p>
      ) : (
        <ul className="channel-list">
          {contact.channels.map((channel) => (
            <li key={channel.id}>
              <span className="channel-kind">{channel.kind === "phone" ? "Phone" : "Email"}</span>
              <span className="channel-value">{channel.value}</span>
              <span className="channel-label">{channel.label ?? ""}</span>
              {channel.preferred ? <span className="cell-flag">Preferred</span> : null}
            </li>
          ))}
        </ul>
      )}

      <ActivityTimeline client={client} parentType="contact" parentId={contact.id} />
    </section>
  );
}

// ---------------------------------------------------------------------------
// Contact form (create + edit)
// ---------------------------------------------------------------------------

interface ChannelDraft {
  kind: ChannelKind;
  label: string;
  value: string;
  preferred: boolean;
}

interface ContactDraft {
  firstName: string;
  lastName: string;
  displayName: string;
  kind: PartyKind;
  role: ContactRole | "";
  companyId: string;
  preferredContactMethod: string;
  addressLine1: string;
  addressLine2: string;
  city: string;
  state: string;
  postalCode: string;
  propertyType: string;
  notes: string;
  favorite: boolean;
  channels: ChannelDraft[];
}

const EMPTY_DRAFT: ContactDraft = {
  firstName: "",
  lastName: "",
  displayName: "",
  kind: "lead",
  role: "",
  companyId: "",
  preferredContactMethod: "",
  addressLine1: "",
  addressLine2: "",
  city: "",
  state: "",
  postalCode: "",
  propertyType: "",
  notes: "",
  favorite: false,
  channels: [],
};

function draftFrom(contact: Contact): ContactDraft {
  return {
    firstName: contact.firstName ?? "",
    lastName: contact.lastName ?? "",
    displayName: contact.displayName,
    kind: contact.kind,
    role: contact.role ?? "",
    companyId: contact.companyId ?? "",
    preferredContactMethod: contact.preferredContactMethod ?? "",
    addressLine1: contact.addressLine1 ?? "",
    addressLine2: contact.addressLine2 ?? "",
    city: contact.city ?? "",
    state: contact.state ?? "",
    postalCode: contact.postalCode ?? "",
    propertyType: contact.propertyType ?? "",
    notes: contact.notes ?? "",
    favorite: contact.favorite,
    channels: contact.channels.map((channel) => ({
      kind: channel.kind,
      label: channel.label ?? "",
      value: channel.value,
      preferred: channel.preferred,
    })),
  };
}

// Blank strings collapse to null so the wire shape matches the Rust patch.
const orNull = (value: string) => (value.trim() === "" ? null : value);

function patchFrom(draft: ContactDraft): ContactPatch {
  return {
    companyId: orNull(draft.companyId),
    firstName: orNull(draft.firstName),
    lastName: orNull(draft.lastName),
    displayName: orNull(draft.displayName),
    role: draft.role === "" ? null : draft.role,
    kind: draft.kind,
    preferredContactMethod: orNull(draft.preferredContactMethod),
    addressLine1: orNull(draft.addressLine1),
    addressLine2: orNull(draft.addressLine2),
    city: orNull(draft.city),
    state: orNull(draft.state),
    postalCode: orNull(draft.postalCode),
    propertyType: orNull(draft.propertyType),
    notes: orNull(draft.notes),
    favorite: draft.favorite,
    channels: draft.channels.map((channel) => ({
      kind: channel.kind,
      label: orNull(channel.label),
      value: channel.value,
      preferred: channel.preferred,
    })),
  };
}

interface ContactFormProps {
  client: CoreClient;
  contactId?: string; // present when editing
  onSaved: (contact: Contact) => void;
  onCancel: () => void;
}

export function ContactFormView({ client, contactId, onSaved, onCancel }: ContactFormProps) {
  const [draft, setDraft] = useState<ContactDraft>(EMPTY_DRAFT);
  const [expectedVersion, setExpectedVersion] = useState(1);
  const [companies, setCompanies] = useState<Company[]>([]);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);
  const [ready, setReady] = useState(!contactId);

  // Load companies for the link select, and the record itself when editing.
  const load = useCallback(() => {
    setError(NO_SAVE_ERROR);
    client
      .listCompanies(true)
      .then(setCompanies)
      .catch(() => {});
    if (!contactId) return;
    client
      .getContact(contactId)
      .then((contact) => {
        setDraft(draftFrom(contact));
        setExpectedVersion(contact.version);
        setReady(true);
      })
      .catch((rejection) => setError(saveErrorFrom(rejection)));
  }, [client, contactId]);

  useEffect(load, [load]);

  const set = <K extends keyof ContactDraft>(key: K, value: ContactDraft[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const setChannel = (index: number, patch: Partial<ChannelDraft>) =>
    setDraft((current) => ({
      ...current,
      channels: current.channels.map((channel, i) =>
        i === index ? { ...channel, ...patch } : channel,
      ),
    }));

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(NO_SAVE_ERROR);
    const patch = patchFrom(draft);
    try {
      const saved = contactId
        ? await client.updateContact({ contactId, expectedVersion, patch })
        : await client.createContact(patch);
      onSaved(saved);
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  if (!ready) {
    return (
      <section className="crm-section" aria-label="Contact form">
        <GeneralError message={error.general} />
      </section>
    );
  }

  const channelError = (index: number, part: string) =>
    error.fields[`channels[${index}].${part}`];

  return (
    <section className="crm-section" aria-label="Contact form">
      <div className="detail-head">
        <div>
          <p className="eyebrow">Contact</p>
          <h2 className="detail-title">{contactId ? "Edit contact" : "New contact"}</h2>
        </div>
      </div>

      {error.conflict ? <ConflictBanner onReload={load} /> : null}
      <GeneralError message={error.general} />

      <form className="record-form" onSubmit={submit}>
        <div className="form-grid">
          <Field label="First name" error={error.fields.firstName}>
            <input
              value={draft.firstName}
              onChange={(event) => set("firstName", event.target.value)}
            />
          </Field>
          <Field label="Last name" error={error.fields.lastName}>
            <input value={draft.lastName} onChange={(event) => set("lastName", event.target.value)} />
          </Field>
          <Field label="Display name" error={error.fields.displayName}>
            <input
              value={draft.displayName}
              onChange={(event) => set("displayName", event.target.value)}
              placeholder="Defaults to first + last"
            />
          </Field>
          <Field label="Kind" error={error.fields.kind}>
            <select
              value={draft.kind}
              onChange={(event) => set("kind", event.target.value as PartyKind)}
            >
              {PARTY_KIND_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Role" error={error.fields.role}>
            <select
              value={draft.role}
              onChange={(event) => set("role", event.target.value as ContactRole | "")}
            >
              <option value="">—</option>
              {CONTACT_ROLE_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Company" error={error.fields.companyId}>
            <select
              value={draft.companyId}
              onChange={(event) => set("companyId", event.target.value)}
            >
              <option value="">No company</option>
              {companies.map((company) => (
                <option key={company.id} value={company.id}>
                  {company.name}
                  {company.archivedAt ? " (archived)" : ""}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Preferred method" error={error.fields.preferredContactMethod}>
            <input
              value={draft.preferredContactMethod}
              onChange={(event) => set("preferredContactMethod", event.target.value)}
              placeholder="e.g. text after 4pm"
            />
          </Field>
          <Field label="Address line 1" error={error.fields.addressLine1}>
            <input
              value={draft.addressLine1}
              onChange={(event) => set("addressLine1", event.target.value)}
            />
          </Field>
          <Field label="Address line 2" error={error.fields.addressLine2}>
            <input
              value={draft.addressLine2}
              onChange={(event) => set("addressLine2", event.target.value)}
            />
          </Field>
          <Field label="City" error={error.fields.city}>
            <input value={draft.city} onChange={(event) => set("city", event.target.value)} />
          </Field>
          <Field label="State" error={error.fields.state}>
            <input value={draft.state} onChange={(event) => set("state", event.target.value)} />
          </Field>
          <Field label="Postal code" error={error.fields.postalCode}>
            <input
              value={draft.postalCode}
              onChange={(event) => set("postalCode", event.target.value)}
            />
          </Field>
          <Field label="Property type" error={error.fields.propertyType}>
            <input
              value={draft.propertyType}
              onChange={(event) => set("propertyType", event.target.value)}
            />
          </Field>
          <Field label="Notes" error={error.fields.notes}>
            <input value={draft.notes} onChange={(event) => set("notes", event.target.value)} />
          </Field>
          <label className="toggle toggle--form">
            <input
              type="checkbox"
              checked={draft.favorite}
              onChange={(event) => set("favorite", event.target.checked)}
            />
            <span>Favorite</span>
          </label>
        </div>
        {contactId ? <RecordMetadata client={client} entityType="contact" recordId={contactId} expectedVersion={expectedVersion} onSaved={load} /> : null}

        <h3 className="detail-subhead">Phone &amp; email</h3>
        {draft.channels.map((channel, index) => (
          <div key={index} className="channel-row">
            <Field label="Channel" error={channelError(index, "kind")}>
              <select
                value={channel.kind}
                onChange={(event) => setChannel(index, { kind: event.target.value as ChannelKind })}
              >
                <option value="phone">Phone</option>
                <option value="email">Email</option>
              </select>
            </Field>
            <Field label="Label" error={channelError(index, "label")}>
              <input
                value={channel.label}
                onChange={(event) => setChannel(index, { label: event.target.value })}
                placeholder="Mobile, office…"
              />
            </Field>
            <Field label="Value" error={channelError(index, "value")}>
              <input
                value={channel.value}
                onChange={(event) => setChannel(index, { value: event.target.value })}
              />
            </Field>
            <label className="toggle toggle--form">
              <input
                type="checkbox"
                checked={channel.preferred}
                onChange={(event) => setChannel(index, { preferred: event.target.checked })}
              />
              <span>Preferred</span>
            </label>
            {channelError(index, "preferred") ? (
              <span className="field__error" role="alert">
                {channelError(index, "preferred")}
              </span>
            ) : null}
            <button
              type="button"
              className="button"
              onClick={() =>
                setDraft((current) => ({
                  ...current,
                  channels: current.channels.filter((_, i) => i !== index),
                }))
              }
            >
              Remove
            </button>
          </div>
        ))}
        <button
          type="button"
          className="button"
          onClick={() =>
            setDraft((current) => ({
              ...current,
              channels: [
                ...current.channels,
                { kind: "phone", label: "", value: "", preferred: false },
              ],
            }))
          }
        >
          Add phone or email
        </button>

        <div className="form-actions">
          <button type="button" className="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="button button--primary">
            {contactId ? "Save contact" : "Create contact"}
          </button>
        </div>
      </form>
    </section>
  );
}
