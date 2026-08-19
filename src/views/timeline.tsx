import { useCallback, useEffect, useState } from "react";

import type { CoreClient } from "../api/client";
import { HistorySummaryPanel } from "../components/FollowupDraft";
import type {
  Activity,
  ActivityDirection,
  ActivityKind,
  ActivityPatch,
  ParentType,
} from "../api/types";
import { Field, GeneralError, NO_SAVE_ERROR, saveErrorFrom, type SaveError } from "./form-support";

// Wire enum options with contractor-facing labels.
export const ACTIVITY_KIND_OPTIONS: { value: ActivityKind; label: string }[] = [
  { value: "call", label: "Call" },
  { value: "email", label: "Email" },
  { value: "text", label: "Text" },
  { value: "site_visit", label: "Site visit" },
  { value: "meeting", label: "Meeting" },
  { value: "note", label: "Note" },
];

export function activityKindLabel(kind: ActivityKind): string {
  return ACTIVITY_KIND_OPTIONS.find((option) => option.value === kind)?.label ?? kind;
}

// Direction only applies to two-way communication kinds.
export const hasDirection = (kind: ActivityKind) =>
  kind === "call" || kind === "email" || kind === "text";

// ---------------------------------------------------------------------------
// datetime-local helpers — inputs edit local minutes, the wire is UTC ISO.
// ---------------------------------------------------------------------------

// Now (minute precision) as a datetime-local input value.
export function nowLocalInput(): string {
  const now = new Date();
  now.setSeconds(0, 0);
  return new Date(now.getTime() - now.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
}

// Stored UTC ISO timestamp → datetime-local input value.
export function isoToLocalInput(iso: string): string {
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) return "";
  return new Date(parsed.getTime() - parsed.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
}

// datetime-local input value → UTC ISO for the wire.
export function localInputToIso(value: string): string {
  return new Date(value).toISOString();
}

// ---------------------------------------------------------------------------
// Activity draft + shared field set (log form and per-entry edit form)
// ---------------------------------------------------------------------------

interface ActivityDraft {
  kind: ActivityKind;
  direction: ActivityDirection;
  occurredAt: string; // datetime-local value
  summary: string;
  body: string;
}

const emptyDraft = (): ActivityDraft => ({
  kind: "note",
  direction: "outbound",
  occurredAt: nowLocalInput(),
  summary: "",
  body: "",
});

const draftFrom = (activity: Activity): ActivityDraft => ({
  kind: activity.kind,
  direction: activity.direction === "none" ? "outbound" : activity.direction,
  occurredAt: isoToLocalInput(activity.occurredAt),
  summary: activity.summary,
  body: activity.body ?? "",
});

// Direction is omitted for one-way kinds so the core defaults it to "none".
function patchFrom(draft: ActivityDraft): ActivityPatch {
  return {
    kind: draft.kind,
    ...(hasDirection(draft.kind) ? { direction: draft.direction } : {}),
    ...(draft.occurredAt ? { occurredAt: localInputToIso(draft.occurredAt) } : {}),
    summary: draft.summary,
    body: draft.body.trim() === "" ? null : draft.body,
  };
}

// Shared kind/direction/when/summary/body fields for logging and editing.
function ActivityFields({
  draft,
  onChange,
  errors,
}: {
  draft: ActivityDraft;
  onChange: (draft: ActivityDraft) => void;
  errors: Record<string, string>;
}) {
  const set = <K extends keyof ActivityDraft>(key: K, value: ActivityDraft[K]) =>
    onChange({ ...draft, [key]: value });

  return (
    <div className="form-grid">
      <Field label="Type" error={errors.kind}>
        <select
          value={draft.kind}
          onChange={(event) => set("kind", event.target.value as ActivityKind)}
        >
          {ACTIVITY_KIND_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </Field>
      {hasDirection(draft.kind) ? (
        <Field label="Direction" error={errors.direction}>
          <select
            value={draft.direction}
            onChange={(event) => set("direction", event.target.value as ActivityDirection)}
          >
            <option value="outbound">Outbound</option>
            <option value="inbound">Inbound</option>
          </select>
        </Field>
      ) : null}
      <Field label="When" error={errors.occurredAt}>
        <input
          type="datetime-local"
          value={draft.occurredAt}
          onChange={(event) => set("occurredAt", event.target.value)}
        />
      </Field>
      <Field label="Summary" error={errors.summary}>
        <input value={draft.summary} onChange={(event) => set("summary", event.target.value)} />
      </Field>
      <Field label="Details" error={errors.body}>
        <textarea
          rows={3}
          value={draft.body}
          onChange={(event) => set("body", event.target.value)}
        />
      </Field>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Activity timeline — newest-first entries plus the inline log form. Shared
// by the contact, company, and opportunity detail views.
// ---------------------------------------------------------------------------

interface ActivityTimelineProps {
  client: CoreClient;
  parentType: ParentType;
  parentId: string;
}

export function ActivityTimeline({ client, parentType, parentId }: ActivityTimelineProps) {
  const [entries, setEntries] = useState<Activity[] | null>(null);
  const [includeRelated, setIncludeRelated] = useState(false);
  const [draft, setDraft] = useState<ActivityDraft>(emptyDraft);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState<ActivityDraft>(emptyDraft);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);

  // Opportunities are the leaf — related roll-up only applies upward.
  const allowIncludeRelated = parentType !== "opportunity";

  const load = useCallback(() => {
    client
      .getTimeline(parentType, parentId, includeRelated)
      .then(setEntries)
      .catch((rejection) => setError(saveErrorFrom(rejection)));
  }, [client, parentType, parentId, includeRelated]);

  useEffect(load, [load]);

  // Summary is required — keep an empty one from ever reaching the core.
  const requireSummary = (value: string): boolean => {
    if (value.trim() !== "") return true;
    setError({ fields: { summary: "Enter a short summary." }, general: null, conflict: false });
    return false;
  };

  const submitLog = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!requireSummary(draft.summary)) return;
    setError(NO_SAVE_ERROR);
    try {
      await client.logActivity({ parentType, parentId, ...patchFrom(draft) });
      setDraft(emptyDraft());
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  const saveEdit = async (activity: Activity) => {
    if (!requireSummary(editDraft.summary)) return;
    setError(NO_SAVE_ERROR);
    try {
      await client.updateActivity({
        activityId: activity.id,
        expectedVersion: activity.version,
        patch: patchFrom(editDraft),
      });
      setEditingId(null);
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  const remove = async (activity: Activity) => {
    if (!window.confirm(`Delete this ${activityKindLabel(activity.kind).toLowerCase()} entry?`)) {
      return;
    }
    setError(NO_SAVE_ERROR);
    try {
      await client.deleteActivity({ activityId: activity.id, expectedVersion: activity.version });
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  return (
    <section className="timeline" aria-label="Activity timeline">
      <div className="timeline-head">
        <h3 className="detail-subhead">Activity</h3>
        {allowIncludeRelated ? (
          <label className="toggle">
            <input
              type="checkbox"
              checked={includeRelated}
              onChange={(event) => setIncludeRelated(event.target.checked)}
            />
            <span>Include opportunity activity</span>
          </label>
        ) : null}
      </div>

      <GeneralError message={error.general} />

      {/* Recap of this record's history — reads only, and only when asked. */}
      <HistorySummaryPanel client={client} parentType={parentType} parentId={parentId} />

      <form className="timeline-form" onSubmit={submitLog} aria-label="Log activity">
        <ActivityFields draft={draft} onChange={setDraft} errors={error.fields} />
        <div className="form-actions">
          <button type="submit" className="button button--primary">
            Log activity
          </button>
        </div>
      </form>

      {entries && entries.length === 0 ? (
        <p className="detail-empty">No activity logged yet.</p>
      ) : null}

      {entries && entries.length > 0 ? (
        <ul className="timeline-list" aria-label="Activity entries">
          {entries.map((activity) =>
            editingId === activity.id ? (
              <li key={activity.id} className="timeline-entry timeline-entry--editing">
                <ActivityFields draft={editDraft} onChange={setEditDraft} errors={error.fields} />
                <div className="form-actions">
                  <button type="button" className="button" onClick={() => setEditingId(null)}>
                    Cancel
                  </button>
                  <button
                    type="button"
                    className="button button--primary"
                    onClick={() => saveEdit(activity)}
                  >
                    Save entry
                  </button>
                </div>
              </li>
            ) : (
              <li key={activity.id} className="timeline-entry">
                <span className="timeline-kind">
                  {activityKindLabel(activity.kind)}
                  {activity.direction !== "none"
                    ? ` · ${activity.direction === "inbound" ? "Inbound" : "Outbound"}`
                    : ""}
                </span>
                <div className="timeline-main">
                  <span className="timeline-summary">{activity.summary}</span>
                  {activity.body ? <p className="timeline-body">{activity.body}</p> : null}
                </div>
                <span className="timeline-meta">{activity.occurredAt}</span>
                <div className="timeline-actions">
                  <button
                    type="button"
                    className="button"
                    onClick={() => {
                      setError(NO_SAVE_ERROR);
                      setEditDraft(draftFrom(activity));
                      setEditingId(activity.id);
                    }}
                  >
                    Edit
                  </button>
                  <button type="button" className="button" onClick={() => remove(activity)}>
                    Delete
                  </button>
                </div>
              </li>
            ),
          )}
        </ul>
      ) : null}
    </section>
  );
}
