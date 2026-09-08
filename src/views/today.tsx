import { useCallback, useEffect, useRef, useState } from "react";

import type { CoreClient } from "../api/client";
import type { ParentType, WorkQueue, WorkQueueItem } from "../api/types";
import { ConflictBanner, GeneralError, NO_SAVE_ERROR, saveErrorFrom, type SaveError } from "./form-support";
import "./today.css";

interface TodayViewProps {
  client: CoreClient;
  onOpenRecord: (recordType: ParentType, recordId: string) => void;
}

// `toISOString()` drops the local offset, which can move a task across a
// calendar boundary. This is the explicit local-day contract the core uses.
export function localReferenceTime(now = new Date()): string {
  const offsetMinutes = -now.getTimezoneOffset();
  const sign = offsetMinutes >= 0 ? "+" : "-";
  const absolute = Math.abs(offsetMinutes);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}T${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}${sign}${pad(Math.floor(absolute / 60))}:${pad(absolute % 60)}`;
}

function taskReason(item: Extract<WorkQueueItem, { kind: "task" }>) {
  return item.reason === "overdue" ? "Overdue" : "Due today";
}

function dueLabel(value: string, includeDate: boolean) {
  return new Intl.DateTimeFormat(undefined, {
    ...(includeDate ? { month: "short", day: "numeric" } : {}),
    hour: "numeric",
    minute: "2-digit",
  }).format(
    new Date(value),
  );
}

export function TodayView({ client, onOpenRecord }: TodayViewProps) {
  const [queue, setQueue] = useState<WorkQueue | null>(null);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);
  const [loadError, setLoadError] = useState(false);
  const [busyTaskId, setBusyTaskId] = useState<string | null>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const [focusAfterRefresh, setFocusAfterRefresh] = useState(false);

  const load = useCallback(() => {
    setError(NO_SAVE_ERROR);
    client
      .getWorkQueue(localReferenceTime())
      .then((next) => {
        setQueue(next);
        setLoadError(false);
      })
      .catch(() => setLoadError(true));
  }, [client]);

  useEffect(load, [load]);

  useEffect(() => {
    if (!focusAfterRefresh || busyTaskId !== null) return;
    const next = listRef.current?.querySelector<HTMLButtonElement>("button:not([disabled])");
    (next ?? headingRef.current)?.focus();
    setFocusAfterRefresh(false);
  }, [busyTaskId, focusAfterRefresh, queue]);

  const complete = async (item: Extract<WorkQueueItem, { kind: "task" }>) => {
    setError(NO_SAVE_ERROR);
    setBusyTaskId(item.task.id);
    try {
      await client.completeTask({ taskId: item.task.id, expectedVersion: item.task.version });
      await client.getWorkQueue(localReferenceTime()).then((next) => {
        setQueue(next);
        setLoadError(false);
      });
      setFocusAfterRefresh(true);
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    } finally {
      setBusyTaskId(null);
    }
  };

  return (
    <section className="crm-section today" aria-label="Today">
      <div className="section-rule">
        <div>
          <h2 ref={headingRef} tabIndex={-1}>Today</h2>
          <p className="today__intro">Follow up on what is due and the leads waiting on you.</p>
        </div>
        <span className="list-count">{queue?.items.length ?? 0}</span>
      </div>
      <GeneralError message={error.general} />
      {error.conflict ? <ConflictBanner onReload={load} /> : null}
      {loadError ? (
        <div className="today__load-error">
          <GeneralError message="Could not load today’s work from the local database." />
          <button type="button" className="button" onClick={load}>Try again</button>
        </div>
      ) : null}
      {queue?.items.length === 0 ? (
        <div className="empty-state">
          <span className="registration-mark" aria-hidden="true" />
          <p className="eyebrow">Clear for now</p>
          <h2>No follow-ups need work today.</h2>
        </div>
      ) : null}
      {queue?.items.length ? (
        <ul ref={listRef} className="today__list" aria-label="Today’s work">
          {queue.items.map((item) =>
            item.kind === "task" ? (
              <li key={`task:${item.task.id}`} className="today__item">
                <div>
                  <p className="today__reason">{taskReason(item)} · {dueLabel(item.task.dueAt, item.reason === "overdue")}</p>
                  <p className="today__title">{item.task.title}</p>
                  {item.linkedRecord ? (
                    <button
                      type="button"
                      className="today__record"
                      onClick={() => onOpenRecord(item.linkedRecord!.recordType, item.linkedRecord!.recordId)}
                    >
                      {item.linkedRecord.displayName}
                    </button>
                  ) : null}
                </div>
                <button
                  type="button"
                  className="button button--primary"
                  disabled={busyTaskId !== null}
                  onClick={() => void complete(item)}
                >
                  {busyTaskId === item.task.id ? "Completing…" : "Complete"}
                </button>
              </li>
            ) : (
              <li key={`${item.rule}:${item.recordId}`} className="today__item">
                <div>
                  <p className="today__reason">{item.rule === "proposal_no_response" ? "Proposal follow-up" : "Stale lead"}</p>
                  <button
                    type="button"
                    className="today__title today__title--button"
                    onClick={() => onOpenRecord(item.recordType, item.recordId)}
                  >
                    {item.recordDisplayName}
                  </button>
                  <p className="today__explanation">{item.explanation}</p>
                </div>
                <button
                  type="button"
                  className="button"
                  onClick={() => onOpenRecord(item.recordType, item.recordId)}
                >
                  Open
                </button>
              </li>
            ),
          )}
        </ul>
      ) : null}
      {queue?.truncated ? <p className="today__truncated">Showing the first 50 items. Open Tasks or Attention for the full lists.</p> : null}
    </section>
  );
}
