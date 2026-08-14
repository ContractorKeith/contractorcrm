import { useCallback, useEffect, useState } from "react";

import type { CoreClient } from "../api/client";
import type {
  Company,
  Contact,
  LostReason,
  Money,
  Opportunity,
  OpportunityDetail,
  OpportunityListItem,
  OpportunityPatch,
  OpportunitySource,
  Stage,
} from "../api/types";
import { RecordTable, type ColumnDef, type SortState } from "../components/RecordTable";
import {
  ConflictBanner,
  Field,
  GeneralError,
  NO_SAVE_ERROR,
  saveErrorFrom,
  type SaveError,
} from "./form-support";

// Wire enum options with contractor-facing labels.
export const OPPORTUNITY_SOURCE_OPTIONS: { value: OpportunitySource; label: string }[] = [
  { value: "referral", label: "Referral" },
  { value: "repeat_client", label: "Repeat client" },
  { value: "website", label: "Website" },
  { value: "sign", label: "Sign" },
  { value: "other", label: "Other" },
];

function sourceLabel(opportunity: Opportunity): string {
  if (opportunity.sourceLabel) return opportunity.sourceLabel;
  if (!opportunity.source) return "—";
  return (
    OPPORTUNITY_SOURCE_OPTIONS.find((option) => option.value === opportunity.source)?.label ??
    opportunity.source
  );
}

// ---------------------------------------------------------------------------
// Money helpers — the stored value stays an integer of minor units throughout;
// division by 100 happens only at render time for display.
// ---------------------------------------------------------------------------

export function formatMoney(money: Money): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: money.currencyCode,
  }).format(money.valueMinor / 100);
}

// Integer minor units → editable dollars string ("123456" → "1234.56").
// Pure integer math so no float rounding sneaks into the form value.
export function minorToDollarsInput(minor: number): string {
  const sign = minor < 0 ? "-" : "";
  const abs = Math.abs(minor);
  return `${sign}${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")}`;
}

// Dollars input → integer minor units, or null when it does not parse.
// String parsing only — never float-multiplies the typed amount.
export function dollarsToMinor(input: string): number | null {
  const cleaned = input.trim().replace(/[$,\s]/g, "");
  if (cleaned === "") return 0;
  const match = /^(-?)(\d+)(?:\.(\d{1,2}))?$/.exec(cleaned);
  if (!match) return null;
  const [, sign, whole = "0", fraction = ""] = match;
  const minor = Number(whole) * 100 + Number(fraction.padEnd(2, "0") || "0");
  return sign === "-" ? -minor : minor;
}

// ---------------------------------------------------------------------------
// Pipeline board — read-only kanban summary (2026-08-14 decision: table is
// primary; the board is a summary view with no drag-and-drop).
// ---------------------------------------------------------------------------

type PipelineMode = "list" | "board";
const PIPELINE_MODE_KEY = "crm.pipelineMode";

// Restore the last List | Board choice; anything unexpected falls back to list.
function loadPipelineMode(): PipelineMode {
  return window.localStorage.getItem(PIPELINE_MODE_KEY) === "board" ? "board" : "list";
}

// Sum a column's values as integer minor units; currency comes from the rows.
function columnTotal(items: OpportunityListItem[]): Money {
  return {
    valueMinor: items.reduce((sum, item) => sum + item.value.valueMinor, 0),
    currencyCode: items[0]?.value.currencyCode ?? "USD",
  };
}

interface PipelineBoardProps {
  stages: Stage[];
  opportunities: OpportunityListItem[]; // archived rows already excluded
  onOpen: (opportunityId: string) => void;
}

export function PipelineBoard({ stages, opportunities, onOpen }: PipelineBoardProps) {
  // Open stages in pipeline order, then Won and Lost as quiet summaries.
  const ordered = [...stages].sort((a, b) => a.sortKey - b.sortKey);
  const columns = [
    ...ordered.filter((stage) => stage.kind === "open"),
    ...ordered.filter((stage) => stage.kind !== "open"),
  ];

  return (
    <div className="pipeline-board" role="region" aria-label="Pipeline board">
      {columns.map((stage) => {
        const items = opportunities.filter((item) => item.stageId === stage.id);
        const closed = stage.kind !== "open";
        return (
          <section
            key={stage.id}
            className={closed ? "board-column board-column--closed" : "board-column"}
            aria-label={stage.name}
          >
            <header className="board-column__head">
              <span className="board-column__name">{stage.name}</span>
              <span className="board-column__count">{items.length}</span>
              <span className="board-column__total">{formatMoney(columnTotal(items))}</span>
            </header>
            {closed ? null : items.length === 0 ? (
              <p className="board-column__empty">Nothing in this stage.</p>
            ) : (
              <ul className="board-cards" aria-label={`${stage.name} opportunities`}>
                {items.map((item) => (
                  <li key={item.id}>
                    <button type="button" className="board-card" onClick={() => onOpen(item.id)}>
                      <span className="board-card__name">{item.name}</span>
                      <span className="board-card__value">{formatMoney(item.value)}</span>
                      {(item.contactDisplayName ?? item.companyName) ? (
                        <span className="board-card__party">
                          {item.contactDisplayName ?? item.companyName}
                        </span>
                      ) : null}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Pipeline table — the primary pipeline view (2026-08-14 decision).
// ---------------------------------------------------------------------------

interface PipelineViewProps {
  client: CoreClient;
  onOpen: (opportunityId: string) => void;
  onCreate: () => void;
}

export function PipelineView({ client, onOpen, onCreate }: PipelineViewProps) {
  const [opportunities, setOpportunities] = useState<OpportunityListItem[] | null>(null);
  const [stages, setStages] = useState<Stage[]>([]);
  const [showArchived, setShowArchived] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [sort, setSort] = useState<SortState | null>(null);
  const [mode, setModeState] = useState<PipelineMode>(loadPipelineMode);

  // Persist the List | Board choice across sessions.
  const setMode = (next: PipelineMode) => {
    window.localStorage.setItem(PIPELINE_MODE_KEY, next);
    setModeState(next);
  };

  useEffect(() => {
    let active = true;
    Promise.all([client.listOpportunities(showArchived), client.listStages()])
      .then(([opportunityRows, stageRows]) => {
        if (!active) return;
        setOpportunities(opportunityRows);
        setStages(stageRows);
        setLoadError(false);
      })
      .catch(() => {
        if (active) setLoadError(true);
      });
    return () => {
      active = false;
    };
  }, [client, showArchived]);

  // Toggle direction on the active column, ascending on a new one.
  const handleSort = (key: string) =>
    setSort((current) =>
      current?.key === key
        ? { key, direction: current.direction === "ascending" ? "descending" : "ascending" }
        : { key, direction: "ascending" },
    );

  // Stage sorts by pipeline order, never alphabetically.
  const stageOrder = (stageId: string) => stages.findIndex((stage) => stage.id === stageId);

  const compare = (a: OpportunityListItem, b: OpportunityListItem, key: string): number => {
    switch (key) {
      case "name":
        return a.name.localeCompare(b.name);
      case "stage":
        return stageOrder(a.stageId) - stageOrder(b.stageId);
      case "value":
        return a.value.valueMinor - b.value.valueMinor;
      case "expectedClose": {
        // ISO dates compare as strings; missing dates sort last.
        if (a.expectedCloseDate === b.expectedCloseDate) return 0;
        if (a.expectedCloseDate === null) return 1;
        if (b.expectedCloseDate === null) return -1;
        return a.expectedCloseDate < b.expectedCloseDate ? -1 : 1;
      }
      default:
        return 0;
    }
  };

  const rows = opportunities
    ? sort
      ? [...opportunities].sort(
          (a, b) => compare(a, b, sort.key) * (sort.direction === "ascending" ? 1 : -1),
        )
      : opportunities
    : null;

  const columns: ColumnDef<OpportunityListItem>[] = [
    {
      key: "name",
      header: "Name",
      sortable: true,
      render: (opportunity) => (
        <span className="cell-primary">
          {opportunity.name}
          {opportunity.archivedAt ? <span className="cell-flag">Archived</span> : null}
        </span>
      ),
    },
    { key: "stage", header: "Stage", sortable: true, render: (o) => o.stageName },
    {
      key: "party",
      header: "Contact / company",
      render: (o) => o.contactDisplayName ?? o.companyName ?? "—",
    },
    {
      key: "value",
      header: "Value",
      numeric: true,
      sortable: true,
      render: (o) => formatMoney(o.value),
    },
    {
      key: "probability",
      header: "Probability",
      numeric: true,
      render: (o) => (o.probabilityPercent === null ? "—" : `${o.probabilityPercent}%`),
    },
    {
      key: "expectedClose",
      header: "Expected close",
      numeric: true,
      sortable: true,
      render: (o) => o.expectedCloseDate ?? "—",
    },
    { key: "source", header: "Source", render: sourceLabel },
  ];

  return (
    <section className="crm-section" aria-label="Pipeline">
      <div className="section-rule">
        <h2>Pipeline</h2>
        <div className="list-tools">
          <div className="mode-switch" role="group" aria-label="Pipeline view">
            <button type="button" aria-pressed={mode === "list"} onClick={() => setMode("list")}>
              List
            </button>
            <button type="button" aria-pressed={mode === "board"} onClick={() => setMode("board")}>
              Board
            </button>
          </div>
          {mode === "list" ? (
            <label className="toggle">
              <input
                type="checkbox"
                checked={showArchived}
                onChange={(event) => setShowArchived(event.target.checked)}
              />
              <span>Show archived</span>
            </label>
          ) : null}
          <span className="list-count">{opportunities?.length ?? 0}</span>
          <button type="button" className="button button--primary" onClick={onCreate}>
            New opportunity
          </button>
        </div>
      </div>

      {loadError ? (
        <GeneralError message="Could not read the pipeline from the local database." />
      ) : null}

      {rows && rows.length === 0 ? (
        <div className="empty-state">
          <span className="registration-mark" aria-hidden="true" />
          <p className="eyebrow">Ready when you are</p>
          <h2>No opportunities yet</h2>
          <p>
            Track a lead from first call to won job. Everything stays in this app&apos;s local
            database on this machine.
          </p>
        </div>
      ) : null}

      {rows && rows.length > 0 ? (
        mode === "board" ? (
          <PipelineBoard
            stages={stages}
            opportunities={rows.filter((opportunity) => !opportunity.archivedAt)}
            onOpen={onOpen}
          />
        ) : (
          <RecordTable
            label="Pipeline list"
            columns={columns}
            rows={rows}
            onOpen={(opportunity) => onOpen(opportunity.id)}
            {...(sort ? { sort } : {})}
            onSort={handleSort}
          />
        )
      ) : null}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Opportunity detail — facts, stage move, and the append-only history.
// ---------------------------------------------------------------------------

interface OpportunityDetailProps {
  client: CoreClient;
  opportunityId: string;
  onBack: () => void;
  onEdit: () => void;
}

export function OpportunityDetailView({
  client,
  opportunityId,
  onBack,
  onEdit,
}: OpportunityDetailProps) {
  const [detail, setDetail] = useState<OpportunityDetail | null>(null);
  const [stages, setStages] = useState<Stage[]>([]);
  const [lostReasons, setLostReasons] = useState<LostReason[]>([]);
  const [contactName, setContactName] = useState<string | null>(null);
  const [companyName, setCompanyName] = useState<string | null>(null);
  const [toStageId, setToStageId] = useState("");
  const [lostReasonId, setLostReasonId] = useState("");
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);

  const load = useCallback(() => {
    setError(NO_SAVE_ERROR);
    Promise.all([client.getOpportunity(opportunityId), client.listStages(), client.listLostReasons()])
      .then(async ([record, stageRows, reasonRows]) => {
        setDetail(record);
        setStages(stageRows);
        setLostReasons(reasonRows);
        setToStageId("");
        setLostReasonId("");
        setContactName(
          record.contactId ? (await client.getContact(record.contactId)).displayName : null,
        );
        setCompanyName(record.companyId ? (await client.getCompany(record.companyId)).name : null);
      })
      .catch((rejection) => setError(saveErrorFrom(rejection)));
  }, [client, opportunityId]);

  useEffect(load, [load]);

  const stageById = (stageId: string | null) => stages.find((stage) => stage.id === stageId);
  const stageName = (stageId: string | null) =>
    stageId === null ? "—" : (stageById(stageId)?.name ?? stageId);
  const reasonLabel = (reasonId: string | null) =>
    reasonId === null ? null : (lostReasons.find((reason) => reason.id === reasonId)?.label ?? reasonId);

  const targetStage = stageById(toStageId || null);
  const needsLostReason = targetStage?.kind === "lost";

  const moveStage = async () => {
    if (!detail || !toStageId) return;
    // Client-side guard: a lost move without a reason never reaches the core.
    if (needsLostReason && lostReasonId === "") {
      setError({
        fields: { lostReasonId: "Select a lost reason before moving to a lost stage." },
        general: null,
        conflict: false,
      });
      return;
    }
    setError(NO_SAVE_ERROR);
    try {
      await client.moveOpportunityStage({
        opportunityId: detail.id,
        toStageId,
        lostReasonId: lostReasonId === "" ? null : lostReasonId,
        expectedVersion: detail.version,
      });
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  const toggleArchive = async () => {
    if (!detail) return;
    setError(NO_SAVE_ERROR);
    const request = { id: detail.id, expectedVersion: detail.version };
    try {
      detail.archivedAt
        ? await client.unarchiveOpportunity(request)
        : await client.archiveOpportunity(request);
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  if (!detail) {
    return (
      <section className="crm-section" aria-label="Opportunity detail">
        <GeneralError message={error.general} />
      </section>
    );
  }

  const facts: [string, string][] = [
    ["Value", formatMoney(detail.value)],
    ["Probability", detail.probabilityPercent === null ? "—" : `${detail.probabilityPercent}%`],
    ["Expected close", detail.expectedCloseDate ?? "—"],
    ["Source", sourceLabel(detail)],
    ["Contact", contactName ?? "—"],
    ["Company", companyName ?? "—"],
    ["Notes", detail.notes ?? "—"],
  ];
  if (detail.lostReasonId) facts.push(["Lost reason", reasonLabel(detail.lostReasonId) ?? "—"]);

  // Reverse chronological history; the newest move reads first.
  const history = [...detail.stageHistory].sort((a, b) =>
    a.createdAt === b.createdAt ? (a.id < b.id ? 1 : -1) : a.createdAt < b.createdAt ? 1 : -1,
  );

  return (
    <section className="crm-section" aria-label="Opportunity detail">
      <div className="detail-head">
        <div>
          <p className="eyebrow">Opportunity</p>
          <h2 className="detail-title">
            {detail.name}
            {detail.archivedAt ? <span className="cell-flag">Archived</span> : null}
          </h2>
          <p className="detail-stage">
            Stage: <strong>{stageName(detail.stageId)}</strong>
          </p>
        </div>
        <div className="detail-actions">
          <button type="button" className="button" onClick={onBack}>
            Back
          </button>
          <button type="button" className="button" onClick={toggleArchive}>
            {detail.archivedAt ? "Unarchive" : "Archive"}
          </button>
          <button type="button" className="button button--primary" onClick={onEdit}>
            Edit
          </button>
        </div>
      </div>

      {error.conflict ? <ConflictBanner onReload={load} /> : null}
      <GeneralError message={error.general} />

      <dl className="detail-facts">
        {facts.map(([term, value]) => (
          <div key={term} className="detail-fact">
            <dt>{term}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>

      <h3 className="detail-subhead">Move stage</h3>
      <div className="stage-move">
        <Field label="Move to stage" error={error.fields.toStageId}>
          <select value={toStageId} onChange={(event) => setToStageId(event.target.value)}>
            <option value="">Choose a stage</option>
            {stages
              .filter((stage) => stage.id !== detail.stageId)
              .map((stage) => (
                <option key={stage.id} value={stage.id}>
                  {stage.name}
                </option>
              ))}
          </select>
        </Field>
        {needsLostReason ? (
          <Field label="Lost reason" error={error.fields.lostReasonId}>
            <select value={lostReasonId} onChange={(event) => setLostReasonId(event.target.value)}>
              <option value="">Choose a reason</option>
              {lostReasons.map((reason) => (
                <option key={reason.id} value={reason.id}>
                  {reason.label}
                </option>
              ))}
            </select>
          </Field>
        ) : null}
        <button
          type="button"
          className="button button--primary"
          disabled={toStageId === ""}
          onClick={moveStage}
        >
          Move
        </button>
      </div>

      <h3 className="detail-subhead">Stage history</h3>
      {history.length === 0 ? (
        <p className="detail-empty">No stage changes recorded.</p>
      ) : (
        <ul className="history-list">
          {history.map((entry) => (
            <li key={entry.id}>
              <span className="history-move">
                {stageName(entry.fromStageId)} → {stageName(entry.toStageId)}
              </span>
              <span className="history-meta">
                {entry.actor} · {entry.createdAt}
                {entry.lostReasonId ? ` · ${reasonLabel(entry.lostReasonId)}` : ""}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Opportunity form (create + edit)
// ---------------------------------------------------------------------------

interface OpportunityDraft {
  name: string;
  contactId: string;
  companyId: string;
  stageId: string; // create only; moves handle stage changes after that
  valueDollars: string;
  probability: string;
  expectedCloseDate: string;
  source: OpportunitySource | "";
  sourceLabel: string;
  notes: string;
}

const EMPTY_DRAFT: OpportunityDraft = {
  name: "",
  contactId: "",
  companyId: "",
  stageId: "",
  valueDollars: "",
  probability: "",
  expectedCloseDate: "",
  source: "",
  sourceLabel: "",
  notes: "",
};

function draftFrom(opportunity: Opportunity): OpportunityDraft {
  return {
    name: opportunity.name,
    contactId: opportunity.contactId ?? "",
    companyId: opportunity.companyId ?? "",
    stageId: opportunity.stageId,
    valueDollars: minorToDollarsInput(opportunity.value.valueMinor),
    probability: opportunity.probabilityPercent === null ? "" : String(opportunity.probabilityPercent),
    expectedCloseDate: opportunity.expectedCloseDate ?? "",
    source: opportunity.source ?? "",
    sourceLabel: opportunity.sourceLabel ?? "",
    notes: opportunity.notes ?? "",
  };
}

// Blank strings collapse to null so the wire shape matches the Rust patch.
const orNull = (value: string) => (value.trim() === "" ? null : value);

interface OpportunityFormProps {
  client: CoreClient;
  opportunityId?: string; // present when editing
  onSaved: (opportunity: Opportunity) => void;
  onCancel: () => void;
}

export function OpportunityFormView({
  client,
  opportunityId,
  onSaved,
  onCancel,
}: OpportunityFormProps) {
  const [draft, setDraft] = useState<OpportunityDraft>(EMPTY_DRAFT);
  const [expectedVersion, setExpectedVersion] = useState(1);
  const [currencyCode, setCurrencyCode] = useState("USD");
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [companies, setCompanies] = useState<Company[]>([]);
  const [stages, setStages] = useState<Stage[]>([]);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);
  const [ready, setReady] = useState(!opportunityId);

  // Load the link selects and stages, plus the record itself when editing.
  const load = useCallback(() => {
    setError(NO_SAVE_ERROR);
    client.listContacts(true).then(setContacts).catch(() => {});
    client.listCompanies(true).then(setCompanies).catch(() => {});
    client
      .listStages()
      .then((stageRows) => {
        setStages(stageRows);
        // Create starts in the first open stage unless the user picks another.
        if (!opportunityId) {
          const firstOpen = stageRows.find((stage) => stage.kind === "open");
          if (firstOpen) setDraft((current) => ({ ...current, stageId: current.stageId || firstOpen.id }));
        }
      })
      .catch(() => {});
    if (!opportunityId) return;
    client
      .getOpportunity(opportunityId)
      .then((record) => {
        setDraft(draftFrom(record));
        setExpectedVersion(record.version);
        setCurrencyCode(record.value.currencyCode);
        setReady(true);
      })
      .catch((rejection) => setError(saveErrorFrom(rejection)));
  }, [client, opportunityId]);

  useEffect(load, [load]);

  const set = <K extends keyof OpportunityDraft>(key: K, value: OpportunityDraft[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(NO_SAVE_ERROR);

    // Dollars parse to integer minor units before anything hits the wire.
    const valueMinor = dollarsToMinor(draft.valueDollars);
    if (valueMinor === null) {
      setError({
        fields: { valueMinor: "Enter a dollar amount like 1250 or 1,250.50." },
        general: null,
        conflict: false,
      });
      return;
    }
    const probability = draft.probability.trim() === "" ? null : Number(draft.probability);
    if (probability !== null && !Number.isInteger(probability)) {
      setError({
        fields: { probabilityPercent: "Enter a whole percent from 0 to 100." },
        general: null,
        conflict: false,
      });
      return;
    }

    const patch: OpportunityPatch = {
      name: draft.name,
      contactId: orNull(draft.contactId),
      companyId: orNull(draft.companyId),
      valueMinor,
      currencyCode,
      probabilityPercent: probability,
      expectedCloseDate: orNull(draft.expectedCloseDate),
      source: draft.source === "" ? null : draft.source,
      sourceLabel: orNull(draft.sourceLabel),
      notes: orNull(draft.notes),
    };

    try {
      const saved = opportunityId
        ? await client.updateOpportunity({ opportunityId, expectedVersion, patch })
        : await client.createOpportunity({ stageId: orNull(draft.stageId), ...patch });
      onSaved(saved);
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  if (!ready) {
    return (
      <section className="crm-section" aria-label="Opportunity form">
        <GeneralError message={error.general} />
      </section>
    );
  }

  return (
    <section className="crm-section" aria-label="Opportunity form">
      <div className="detail-head">
        <div>
          <p className="eyebrow">Opportunity</p>
          <h2 className="detail-title">{opportunityId ? "Edit opportunity" : "New opportunity"}</h2>
        </div>
      </div>

      {error.conflict ? <ConflictBanner onReload={load} /> : null}
      <GeneralError message={error.general} />

      <form className="record-form" onSubmit={submit}>
        <div className="form-grid">
          <Field label="Name" error={error.fields.name}>
            <input value={draft.name} onChange={(event) => set("name", event.target.value)} />
          </Field>
          <Field label="Contact" error={error.fields.contactId}>
            <select
              value={draft.contactId}
              onChange={(event) => set("contactId", event.target.value)}
            >
              <option value="">No contact</option>
              {contacts.map((contact) => (
                <option key={contact.id} value={contact.id}>
                  {contact.displayName}
                  {contact.archivedAt ? " (archived)" : ""}
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
          {!opportunityId ? (
            <Field label="Stage" error={error.fields.stageId}>
              <select value={draft.stageId} onChange={(event) => set("stageId", event.target.value)}>
                {stages.map((stage) => (
                  <option key={stage.id} value={stage.id}>
                    {stage.name}
                  </option>
                ))}
              </select>
            </Field>
          ) : null}
          <Field label="Value ($)" error={error.fields.valueMinor}>
            <input
              value={draft.valueDollars}
              onChange={(event) => set("valueDollars", event.target.value)}
              placeholder="e.g. 4,850.00"
              inputMode="decimal"
            />
          </Field>
          <Field label="Probability (%)" error={error.fields.probabilityPercent}>
            <input
              value={draft.probability}
              onChange={(event) => set("probability", event.target.value)}
              placeholder="0–100"
              inputMode="numeric"
            />
          </Field>
          <Field label="Expected close" error={error.fields.expectedCloseDate}>
            <input
              type="date"
              value={draft.expectedCloseDate}
              onChange={(event) => set("expectedCloseDate", event.target.value)}
            />
          </Field>
          <Field label="Source" error={error.fields.source}>
            <select
              value={draft.source}
              onChange={(event) => set("source", event.target.value as OpportunitySource | "")}
            >
              <option value="">—</option>
              {OPPORTUNITY_SOURCE_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Source detail" error={error.fields.sourceLabel}>
            <input
              value={draft.sourceLabel}
              onChange={(event) => set("sourceLabel", event.target.value)}
              placeholder="e.g. referred by Dana"
            />
          </Field>
          <Field label="Notes" error={error.fields.notes}>
            <input value={draft.notes} onChange={(event) => set("notes", event.target.value)} />
          </Field>
        </div>

        <div className="form-actions">
          <button type="button" className="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="button button--primary">
            {opportunityId ? "Save opportunity" : "Create opportunity"}
          </button>
        </div>
      </form>
    </section>
  );
}
