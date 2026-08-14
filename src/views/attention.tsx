import { useCallback, useEffect, useState } from "react";

import type { CoreClient } from "../api/client";
import type { AttentionFlag, AttentionRecordType, AttentionThresholds } from "../api/types";
import { Field, GeneralError, NO_SAVE_ERROR, saveErrorFrom, type SaveError } from "./form-support";

// ---------------------------------------------------------------------------
// Needs-attention view — deterministic flags (severity pre-sorted by the
// core) plus the small thresholds editor.
// ---------------------------------------------------------------------------

interface AttentionViewProps {
  client: CoreClient;
  onOpenRecord: (recordType: AttentionRecordType, recordId: string) => void;
}

interface ThresholdDraft {
  staleLeadDays: string;
  proposalNoResponseDays: string;
  proposalStageName: string;
}

export function AttentionView({ client, onOpenRecord }: AttentionViewProps) {
  const [flags, setFlags] = useState<AttentionFlag[] | null>(null);
  const [draft, setDraft] = useState<ThresholdDraft | null>(null);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);
  const [loadError, setLoadError] = useState(false);

  const load = useCallback(() => {
    Promise.all([client.getAttentionFlags(), client.getAttentionThresholds()])
      .then(([flagRows, thresholds]: [AttentionFlag[], AttentionThresholds]) => {
        setFlags(flagRows);
        setDraft({
          staleLeadDays: String(thresholds.staleLeadDays),
          proposalNoResponseDays: String(thresholds.proposalNoResponseDays),
          proposalStageName: thresholds.proposalStageName,
        });
        setLoadError(false);
      })
      .catch(() => setLoadError(true));
  }, [client]);

  useEffect(load, [load]);

  const set = <K extends keyof ThresholdDraft>(key: K, value: string) =>
    setDraft((current) => (current ? { ...current, [key]: value } : current));

  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!draft) return;
    setError(NO_SAVE_ERROR);
    try {
      await client.setAttentionThresholds({
        staleLeadDays: Number(draft.staleLeadDays),
        proposalNoResponseDays: Number(draft.proposalNoResponseDays),
        proposalStageName: draft.proposalStageName,
      });
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  return (
    <section className="crm-section" aria-label="Needs attention">
      <div className="section-rule">
        <h2>Needs attention</h2>
        <span className="list-count">{flags?.length ?? 0}</span>
      </div>

      {loadError ? (
        <GeneralError message="Could not read attention flags from the local database." />
      ) : null}

      {flags && flags.length === 0 ? (
        <div className="empty-state">
          <span className="registration-mark" aria-hidden="true" />
          <p className="eyebrow">All clear</p>
          <h2>Nothing needs attention.</h2>
        </div>
      ) : null}

      {flags && flags.length > 0 ? (
        <ul className="attention-list" aria-label="Attention flags">
          {flags.map((flag) => (
            <li key={flag.id} className="attention-flag">
              <button
                type="button"
                className="attention-link"
                onClick={() => onOpenRecord(flag.recordType, flag.recordId)}
              >
                {flag.recordDisplayName}
              </button>
              <span className="attention-explanation">{flag.explanation}</span>
            </li>
          ))}
        </ul>
      ) : null}

      <h3 className="detail-subhead">Thresholds</h3>
      <GeneralError message={error.general} />
      {draft ? (
        <form className="record-form" onSubmit={save} aria-label="Attention thresholds">
          <div className="form-grid">
            <Field label="Stale lead after (days)" error={error.fields.staleLeadDays}>
              <input
                inputMode="numeric"
                value={draft.staleLeadDays}
                onChange={(event) => set("staleLeadDays", event.target.value)}
              />
            </Field>
            <Field
              label="Proposal follow-up after (days)"
              error={error.fields.proposalNoResponseDays}
            >
              <input
                inputMode="numeric"
                value={draft.proposalNoResponseDays}
                onChange={(event) => set("proposalNoResponseDays", event.target.value)}
              />
            </Field>
            <Field label="Proposal stage name" error={error.fields.proposalStageName}>
              <input
                value={draft.proposalStageName}
                onChange={(event) => set("proposalStageName", event.target.value)}
              />
            </Field>
          </div>
          <div className="form-actions">
            <button type="submit" className="button button--primary">
              Save thresholds
            </button>
          </div>
        </form>
      ) : null}
    </section>
  );
}
