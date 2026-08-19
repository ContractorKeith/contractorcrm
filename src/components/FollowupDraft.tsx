import { useEffect, useState } from "react";

import type { CoreClient } from "../api/client";
import {
  isCommandError,
  type FollowupDraft,
  type FollowupTemplate,
  type HistorySummary,
  type ParentType,
} from "../api/types";
import { ContextDisclosure } from "./ContextDisclosure";
import { ProposalDialog } from "./ProposalDialog";

// ---------------------------------------------------------------------------
// Shared disclosure line — which model answered, which records it saw, and
// whether anything left this machine. Same treatment the explanation UI uses.
// ---------------------------------------------------------------------------

function disclosure(
  model: string | null,
  endpointHost: string | null,
  local: boolean,
  records: { label: string }[],
): string {
  const named = records.map((record) => record.label).join(", ") || "no records";
  const where = endpointHost
    ? local
      ? `stayed on this machine (${endpointHost})`
      : `sent to ${endpointHost}`
    : "stayed on this machine";
  return `${model ?? "template"} · ${named} ${where}`;
}

function errorText(rejection: unknown, fallback: string): string {
  if (!isCommandError(rejection)) return fallback;
  return rejection.message;
}

// ---------------------------------------------------------------------------
// Summarize — a recap of this record's recent history plus next actions.
// Only offered when the assistant is on; nothing here writes.
// ---------------------------------------------------------------------------

type SummaryState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; summary: HistorySummary }
  | { status: "error"; message: string };

interface HistorySummaryPanelProps {
  client: CoreClient;
  parentType: ParentType;
  parentId: string;
}

export function HistorySummaryPanel({ client, parentType, parentId }: HistorySummaryPanelProps) {
  const [enabled, setEnabled] = useState(false);
  const [state, setState] = useState<SummaryState>({ status: "idle" });

  useEffect(() => {
    let active = true;
    void client
      .getAiSettings()
      .then((settings) => {
        if (active) setEnabled(settings.enabled);
      })
      .catch(() => {
        if (active) setEnabled(false);
      });
    return () => {
      active = false;
    };
  }, [client]);

  // Reset when the panel moves to another record.
  useEffect(() => setState({ status: "idle" }), [parentType, parentId]);

  if (!enabled) return null;

  const summarize = async () => {
    setState({ status: "loading" });
    try {
      setState({ status: "ready", summary: await client.summarizeHistory(parentType, parentId) });
    } catch (rejection) {
      setState({
        status: "error",
        message: errorText(rejection, "The summary could not be created."),
      });
    }
  };

  return (
    <div className="followup" aria-label="History summary">
      <button
        type="button"
        className="button"
        disabled={state.status === "loading"}
        onClick={() => void summarize()}
      >
        {state.status === "loading" ? "Summarizing…" : "Summarize"}
      </button>
      <ContextDisclosure
        client={client}
        request={{ tool: "summarize_history", parentType, parentId }}
      />

      {state.status === "loading" ? <p className="attention-ai__meta">Asking the model…</p> : null}
      {state.status === "error" ? (
        <p role="alert" className="attention-ai__error">
          {state.message}
        </p>
      ) : null}
      {state.status === "ready" ? (
        <div className="attention-ai">
          <p className="attention-ai__text">{state.summary.summary}</p>
          {state.summary.suggestedNextActions.length > 0 ? (
            <ul className="followup__actions" aria-label="Suggested next actions">
              {state.summary.suggestedNextActions.map((action) => (
                <li key={action}>{action}</li>
              ))}
            </ul>
          ) : null}
          <p className="attention-ai__meta">
            {disclosure(
              state.summary.model,
              state.summary.endpointHost,
              state.summary.local,
              state.summary.includedRecordRefs,
            )}
          </p>
        </div>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Draft follow-up — template wording (personalized by the model when it is on)
// plus a follow-up task proposal the user reviews and applies.
// ---------------------------------------------------------------------------

interface FollowupDraftPanelProps {
  client: CoreClient;
  parentType: ParentType;
  parentId: string;
  /** Called after the follow-up task was applied, so the caller can reload. */
  onApplied?: () => void;
}

export function FollowupDraftPanel({
  client,
  parentType,
  parentId,
  onApplied,
}: FollowupDraftPanelProps) {
  const [enabled, setEnabled] = useState(false);
  const [templates, setTemplates] = useState<FollowupTemplate[]>([]);
  const [templateId, setTemplateId] = useState("");
  const [objective, setObjective] = useState("");
  const [draft, setDraft] = useState<FollowupDraft | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void client
      .getAiSettings()
      .then((settings) => {
        if (active) setEnabled(settings.enabled);
      })
      .catch(() => {
        if (active) setEnabled(false);
      });
    void client
      .getFollowupTemplates()
      .then((stored) => {
        if (active) setTemplates(stored.templates);
      })
      .catch(() => {
        if (active) setTemplates([]);
      });
    return () => {
      active = false;
    };
  }, [client]);

  const drafted = async () => {
    setBusy(true);
    try {
      setDraft(
        await client.proposeFollowup(
          parentType,
          parentId,
          objective.trim() === "" ? undefined : objective,
          templateId === "" ? undefined : templateId,
        ),
      );
      setError(null);
    } catch (rejection) {
      setError(errorText(rejection, "The follow-up could not be drafted."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="followup" aria-label="Draft follow-up">
      <h3 className="detail-subhead">{enabled ? "Draft follow-up" : "Use a template"}</h3>

      <div className="followup__controls">
        <label className="assistant-prompt__field">
          <span className="assistant-prompt__label">Template</span>
          <select value={templateId} onChange={(event) => setTemplateId(event.target.value)}>
            <option value="">Choose for me</option>
            {templates.map((template) => (
              <option key={template.id} value={template.id}>
                {template.name}
              </option>
            ))}
          </select>
        </label>
        <label className="assistant-prompt__field">
          <span className="assistant-prompt__label">What it needs to do</span>
          <input
            type="text"
            value={objective}
            placeholder="Chase the proposal…"
            disabled={busy}
            onChange={(event) => setObjective(event.target.value)}
          />
        </label>
        <button type="button" className="button" disabled={busy} onClick={() => void drafted()}>
          {busy ? "Drafting…" : enabled ? "Draft follow-up" : "Use a template"}
        </button>
      </div>

      <ContextDisclosure
        client={client}
        request={{
          tool: "propose_followup",
          parentType,
          parentId,
          ...(objective.trim() === "" ? {} : { objective }),
          ...(templateId === "" ? {} : { templateId }),
        }}
        label={enabled ? "See what will be sent" : "See what would be sent if the assistant is on"}
      />

      {error ? (
        <p role="alert" className="saved-views__error">
          {error}
        </p>
      ) : null}

      {draft ? (
        <div className="attention-ai">
          <p className="attention-ai__meta">{draft.templateName}</p>
          <p className="followup__text">{draft.draftText}</p>
          {draft.warnings.length > 0 ? (
            <ul className="followup__actions" aria-label="Draft warnings">
              {draft.warnings.map((warning) => (
                <li key={warning}>{warning}</li>
              ))}
            </ul>
          ) : null}
          <p className="attention-ai__meta">
            {draft.usedProvider
              ? disclosure(
                  draft.model,
                  draft.endpointHost,
                  draft.local,
                  draft.includedRecordRefs,
                )
              : "Template used as written — nothing was sent anywhere."}
          </p>
          <div className="form-actions">
            <button type="button" className="button" onClick={() => setReviewing(true)}>
              Review follow-up task
            </button>
          </div>
        </div>
      ) : null}

      {draft && reviewing ? (
        <ProposalDialog
          client={client}
          proposal={draft.proposal}
          onClose={(applied) => {
            setReviewing(false);
            if (applied) {
              setDraft(null);
              setObjective("");
              onApplied?.();
            }
          }}
        />
      ) : null}
    </section>
  );
}
