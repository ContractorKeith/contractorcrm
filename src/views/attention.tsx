import { useCallback, useEffect, useState } from "react";

import type { CoreClient } from "../api/client";
import {
  isCommandError,
  type AttentionExplanation,
  type AttentionFlag,
  type AttentionRecordType,
  type AttentionThresholds,
} from "../api/types";
import { Field, GeneralError, NO_SAVE_ERROR, saveErrorFrom, type SaveError } from "./form-support";

// ---------------------------------------------------------------------------
// Needs-attention view — deterministic flags (severity pre-sorted by the
// core) plus the small thresholds editor. The optional AI explanation is
// layered on top: it explains a flag, it never creates or changes one.
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

// Per-flag explanation state; absent means the user never asked.
type ExplainState =
  | { status: "loading" }
  | { status: "ready"; explanation: AttentionExplanation }
  | { status: "error"; message: string };

// Plain-language failure text — no error codes in front of the user.
function explainErrorMessage(rejection: unknown): string {
  if (!isCommandError(rejection)) return "The explanation could not be created.";
  if (rejection.kind === "not_found")
    return "This flag is no longer current — refresh the list and try again.";
  return rejection.message;
}

// Where the flagged record's details went, using the call's own disclosure list.
function disclosure(explanation: AttentionExplanation): string {
  const records = explanation.explanation.includedRecordRefs
    .map((record) => record.label)
    .join(", ");
  const where = explanation.local
    ? `stayed on this machine (${explanation.endpointHost})`
    : `sent to ${explanation.endpointHost}`;
  return `${explanation.explanation.model} · ${records || "no records"} ${where}`;
}

export function AttentionView({ client, onOpenRecord }: AttentionViewProps) {
  const [flags, setFlags] = useState<AttentionFlag[] | null>(null);
  const [draft, setDraft] = useState<ThresholdDraft | null>(null);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);
  const [loadError, setLoadError] = useState(false);
  const [aiEnabled, setAiEnabled] = useState(false);
  const [explanations, setExplanations] = useState<Record<string, ExplainState>>({});

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

  // The Explain affordance only exists when the user has turned the assistant
  // on; an unreachable settings read simply leaves it hidden.
  useEffect(() => {
    let active = true;
    void client
      .getAiSettings()
      .then((settings) => {
        if (active) setAiEnabled(settings.enabled);
      })
      .catch(() => {
        if (active) setAiEnabled(false);
      });
    return () => {
      active = false;
    };
  }, [client]);

  // One explicit provider call for one flag — never automatic.
  const explain = async (flagId: string) => {
    setExplanations((current) => ({ ...current, [flagId]: { status: "loading" } }));
    try {
      const explanation = await client.explainAttentionFlag(flagId);
      setExplanations((current) => ({ ...current, [flagId]: { status: "ready", explanation } }));
    } catch (rejection) {
      setExplanations((current) => ({
        ...current,
        [flagId]: { status: "error", message: explainErrorMessage(rejection) },
      }));
    }
  };

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
          {flags.map((flag) => {
            const state = explanations[flag.id];
            return (
              <li key={flag.id} className="attention-flag">
                <div className="attention-flag__row">
                  <button
                    type="button"
                    className="attention-link"
                    onClick={() => onOpenRecord(flag.recordType, flag.recordId)}
                  >
                    {flag.recordDisplayName}
                  </button>
                  <span className="attention-explanation">{flag.explanation}</span>
                  {aiEnabled ? (
                    <button
                      type="button"
                      className="button attention-explain"
                      aria-label={`Explain ${flag.recordDisplayName}`}
                      disabled={state?.status === "loading"}
                      onClick={() => void explain(flag.id)}
                    >
                      {state?.status === "loading" ? "Explaining…" : "Explain"}
                    </button>
                  ) : null}
                </div>

                {state?.status === "loading" ? (
                  <p className="attention-ai__meta">Asking the model…</p>
                ) : null}
                {state?.status === "error" ? (
                  <p role="alert" className="attention-ai__error">
                    {state.message}
                  </p>
                ) : null}
                {state?.status === "ready" ? (
                  <div className="attention-ai">
                    <p className="attention-ai__text">{state.explanation.explanation.text}</p>
                    <p className="attention-ai__meta">{disclosure(state.explanation)}</p>
                  </div>
                ) : null}
              </li>
            );
          })}
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
