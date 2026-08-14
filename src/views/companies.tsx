import { useCallback, useEffect, useState } from "react";

import type { CoreClient } from "../api/client";
import type { Company, CompanyPatch, PartyKind } from "../api/types";
import { RecordTable, type ColumnDef } from "../components/RecordTable";
import {
  ConflictBanner,
  Field,
  GeneralError,
  NO_SAVE_ERROR,
  PARTY_KIND_OPTIONS,
  partyKindLabel,
  saveErrorFrom,
  type SaveError,
} from "./form-support";

// ---------------------------------------------------------------------------
// Company list
// ---------------------------------------------------------------------------

interface CompaniesViewProps {
  client: CoreClient;
  onOpen: (companyId: string) => void;
  onCreate: () => void;
}

export function CompaniesView({ client, onOpen, onCreate }: CompaniesViewProps) {
  const [companies, setCompanies] = useState<Company[] | null>(null);
  const [showArchived, setShowArchived] = useState(false);
  const [loadError, setLoadError] = useState(false);

  useEffect(() => {
    let active = true;
    client
      .listCompanies(showArchived)
      .then((rows) => {
        if (!active) return;
        setCompanies(rows);
        setLoadError(false);
      })
      .catch(() => {
        if (active) setLoadError(true);
      });
    return () => {
      active = false;
    };
  }, [client, showArchived]);

  const columns: ColumnDef<Company>[] = [
    {
      key: "name",
      header: "Name",
      render: (company) => (
        <span className="cell-primary">
          {company.name}
          {company.archivedAt ? <span className="cell-flag">Archived</span> : null}
        </span>
      ),
    },
    { key: "kind", header: "Kind", render: (company) => partyKindLabel(company.kind) },
    { key: "phone", header: "Phone", render: (company) => company.phone ?? "—" },
    { key: "email", header: "Email", render: (company) => company.email ?? "—" },
    {
      key: "serviceArea",
      header: "Service area",
      render: (company) => company.serviceArea ?? "—",
    },
  ];

  return (
    <section className="crm-section" aria-label="Companies">
      <div className="section-rule">
        <h2>Companies</h2>
        <div className="list-tools">
          <label className="toggle">
            <input
              type="checkbox"
              checked={showArchived}
              onChange={(event) => setShowArchived(event.target.checked)}
            />
            <span>Show archived</span>
          </label>
          <span className="list-count">{companies?.length ?? 0}</span>
          <button type="button" className="button button--primary" onClick={onCreate}>
            New company
          </button>
        </div>
      </div>

      {loadError ? (
        <GeneralError message="Could not read companies from the local database." />
      ) : null}

      {companies && companies.length === 0 ? (
        <div className="empty-state">
          <span className="registration-mark" aria-hidden="true" />
          <p className="eyebrow">Ready when you are</p>
          <h2>No companies yet</h2>
          <p>
            Add the outfits you work with — GCs, subs, vendors, and suppliers — and link their
            people to them.
          </p>
        </div>
      ) : null}

      {companies && companies.length > 0 ? (
        <RecordTable
          label="Company list"
          columns={columns}
          rows={companies}
          onOpen={(company) => onOpen(company.id)}
        />
      ) : null}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Company detail
// ---------------------------------------------------------------------------

interface CompanyDetailProps {
  client: CoreClient;
  companyId: string;
  onBack: () => void;
  onEdit: () => void;
}

export function CompanyDetailView({ client, companyId, onBack, onEdit }: CompanyDetailProps) {
  const [company, setCompany] = useState<Company | null>(null);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);

  const load = useCallback(() => {
    setError(NO_SAVE_ERROR);
    client
      .getCompany(companyId)
      .then(setCompany)
      .catch((rejection) => setError(saveErrorFrom(rejection)));
  }, [client, companyId]);

  useEffect(load, [load]);

  const toggleArchive = async () => {
    if (!company) return;
    setError(NO_SAVE_ERROR);
    const request = { id: company.id, expectedVersion: company.version };
    try {
      const updated = company.archivedAt
        ? await client.unarchiveCompany(request)
        : await client.archiveCompany(request);
      setCompany(updated);
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  if (!company) {
    return (
      <section className="crm-section" aria-label="Company detail">
        <GeneralError message={error.general} />
      </section>
    );
  }

  const facts: [string, string][] = [
    ["Kind", partyKindLabel(company.kind)],
    ["Phone", company.phone ?? "—"],
    ["Email", company.email ?? "—"],
    ["Website", company.website ?? "—"],
    [
      "Address",
      [company.addressLine1, company.addressLine2, company.city, company.state, company.postalCode]
        .filter(Boolean)
        .join(", ") || "—",
    ],
    ["Service area", company.serviceArea ?? "—"],
    ["License notes", company.licenseNotes ?? "—"],
    ["Notes", company.notes ?? "—"],
  ];

  return (
    <section className="crm-section" aria-label="Company detail">
      <div className="detail-head">
        <div>
          <p className="eyebrow">Company</p>
          <h2 className="detail-title">
            {company.name}
            {company.archivedAt ? <span className="cell-flag">Archived</span> : null}
          </h2>
        </div>
        <div className="detail-actions">
          <button type="button" className="button" onClick={onBack}>
            Back
          </button>
          <button type="button" className="button" onClick={toggleArchive}>
            {company.archivedAt ? "Unarchive" : "Archive"}
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
    </section>
  );
}

// ---------------------------------------------------------------------------
// Company form (create + edit)
// ---------------------------------------------------------------------------

interface CompanyDraft {
  name: string;
  kind: PartyKind;
  phone: string;
  email: string;
  website: string;
  addressLine1: string;
  addressLine2: string;
  city: string;
  state: string;
  postalCode: string;
  serviceArea: string;
  licenseNotes: string;
  notes: string;
}

const EMPTY_DRAFT: CompanyDraft = {
  name: "",
  kind: "client",
  phone: "",
  email: "",
  website: "",
  addressLine1: "",
  addressLine2: "",
  city: "",
  state: "",
  postalCode: "",
  serviceArea: "",
  licenseNotes: "",
  notes: "",
};

const orNull = (value: string) => (value.trim() === "" ? null : value);

function patchFrom(draft: CompanyDraft): CompanyPatch {
  return {
    name: draft.name,
    kind: draft.kind,
    phone: orNull(draft.phone),
    email: orNull(draft.email),
    website: orNull(draft.website),
    addressLine1: orNull(draft.addressLine1),
    addressLine2: orNull(draft.addressLine2),
    city: orNull(draft.city),
    state: orNull(draft.state),
    postalCode: orNull(draft.postalCode),
    serviceArea: orNull(draft.serviceArea),
    licenseNotes: orNull(draft.licenseNotes),
    notes: orNull(draft.notes),
  };
}

function draftFrom(company: Company): CompanyDraft {
  return {
    name: company.name,
    kind: company.kind,
    phone: company.phone ?? "",
    email: company.email ?? "",
    website: company.website ?? "",
    addressLine1: company.addressLine1 ?? "",
    addressLine2: company.addressLine2 ?? "",
    city: company.city ?? "",
    state: company.state ?? "",
    postalCode: company.postalCode ?? "",
    serviceArea: company.serviceArea ?? "",
    licenseNotes: company.licenseNotes ?? "",
    notes: company.notes ?? "",
  };
}

interface CompanyFormProps {
  client: CoreClient;
  companyId?: string; // present when editing
  onSaved: (company: Company) => void;
  onCancel: () => void;
}

export function CompanyFormView({ client, companyId, onSaved, onCancel }: CompanyFormProps) {
  const [draft, setDraft] = useState<CompanyDraft>(EMPTY_DRAFT);
  const [expectedVersion, setExpectedVersion] = useState(1);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);
  const [ready, setReady] = useState(!companyId);

  const load = useCallback(() => {
    setError(NO_SAVE_ERROR);
    if (!companyId) return;
    client
      .getCompany(companyId)
      .then((company) => {
        setDraft(draftFrom(company));
        setExpectedVersion(company.version);
        setReady(true);
      })
      .catch((rejection) => setError(saveErrorFrom(rejection)));
  }, [client, companyId]);

  useEffect(load, [load]);

  const set = <K extends keyof CompanyDraft>(key: K, value: CompanyDraft[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(NO_SAVE_ERROR);
    const patch = patchFrom(draft);
    try {
      const saved = companyId
        ? await client.updateCompany({ companyId, expectedVersion, patch })
        : await client.createCompany(patch);
      onSaved(saved);
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  if (!ready) {
    return (
      <section className="crm-section" aria-label="Company form">
        <GeneralError message={error.general} />
      </section>
    );
  }

  // Simple text inputs share one shape; name and kind render separately.
  const textFields: [keyof CompanyDraft, string][] = [
    ["phone", "Phone"],
    ["email", "Email"],
    ["website", "Website"],
    ["addressLine1", "Address line 1"],
    ["addressLine2", "Address line 2"],
    ["city", "City"],
    ["state", "State"],
    ["postalCode", "Postal code"],
    ["serviceArea", "Service area"],
    ["licenseNotes", "License notes"],
    ["notes", "Notes"],
  ];

  return (
    <section className="crm-section" aria-label="Company form">
      <div className="detail-head">
        <div>
          <p className="eyebrow">Company</p>
          <h2 className="detail-title">{companyId ? "Edit company" : "New company"}</h2>
        </div>
      </div>

      {error.conflict ? <ConflictBanner onReload={load} /> : null}
      <GeneralError message={error.general} />

      <form className="record-form" onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name" error={error.fields.name}>
            <input value={draft.name} onChange={(event) => set("name", event.target.value)} />
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
          {textFields.map(([key, label]) => (
            <Field key={key} label={label} error={error.fields[key]}>
              <input value={draft[key]} onChange={(event) => set(key, event.target.value)} />
            </Field>
          ))}
        </div>

        <div className="form-actions">
          <button type="button" className="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="button button--primary">
            {companyId ? "Save company" : "Create company"}
          </button>
        </div>
      </form>
    </section>
  );
}
