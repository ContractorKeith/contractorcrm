import { useCallback, useEffect, useState } from "react";

import type { CoreClient } from "../api/client";
import type {
  Company,
  ContactListItem,
  ListTasksRequest,
  OpportunityListItem,
  ParentType,
  Task,
  TaskPatch,
  TaskPriority,
} from "../api/types";
import { RecordTable, type ColumnDef } from "../components/RecordTable";
import { Field, GeneralError, NO_SAVE_ERROR, saveErrorFrom, type SaveError } from "./form-support";
import { isoToLocalInput, localInputToIso } from "./timeline";

// Wire enum options with plain labels.
const PRIORITY_OPTIONS: { value: TaskPriority; label: string }[] = [
  { value: "low", label: "Low" },
  { value: "normal", label: "Normal" },
  { value: "high", label: "High" },
];

const priorityLabel = (priority: TaskPriority) =>
  PRIORITY_OPTIONS.find((option) => option.value === priority)?.label ?? priority;

const STATUS_LABELS: Record<Task["status"], string> = {
  open: "Open",
  done: "Done",
  dropped: "Dropped",
};

// Open with a due date in the past — gets the attention treatment.
export const isOverdue = (task: Task) =>
  task.status === "open" && task.dueAt !== null && new Date(task.dueAt).getTime() < Date.now();

type TaskFilter = "open" | "all" | "overdue";

// Filter choice → list_tasks request shape.
function filterRequest(filter: TaskFilter): ListTasksRequest {
  if (filter === "open") return { status: "open" };
  if (filter === "overdue") return { overdueOnly: true };
  return {};
}

// One selectable parent option; value encodes type + id for the single select.
interface ParentOption {
  value: string; // "contact:contact-1"
  label: string;
}

const parentValue = (parentType: ParentType | null, parentId: string | null) =>
  parentType && parentId ? `${parentType}:${parentId}` : "";

// ---------------------------------------------------------------------------
// Task form draft
// ---------------------------------------------------------------------------

interface TaskDraft {
  title: string;
  parent: string; // ParentOption value or ""
  dueAt: string; // datetime-local values
  remindAt: string;
  priority: TaskPriority;
}

const EMPTY_DRAFT: TaskDraft = {
  title: "",
  parent: "",
  dueAt: "",
  remindAt: "",
  priority: "normal",
};

const draftFrom = (task: Task): TaskDraft => ({
  title: task.title,
  parent: parentValue(task.parentType, task.parentId),
  dueAt: task.dueAt ? isoToLocalInput(task.dueAt) : "",
  remindAt: task.remindAt ? isoToLocalInput(task.remindAt) : "",
  priority: task.priority,
});

// Draft → wire patch; body carries over unchanged (the form does not edit it).
function patchFrom(draft: TaskDraft, existing: Task | null): TaskPatch {
  const separator = draft.parent.indexOf(":");
  const linked = draft.parent !== "" && separator > 0;
  return {
    title: draft.title,
    body: existing?.body ?? null,
    parentType: linked ? (draft.parent.slice(0, separator) as ParentType) : null,
    parentId: linked ? draft.parent.slice(separator + 1) : null,
    dueAt: draft.dueAt === "" ? null : localInputToIso(draft.dueAt),
    remindAt: draft.remindAt === "" ? null : localInputToIso(draft.remindAt),
    priority: draft.priority,
  };
}

// ---------------------------------------------------------------------------
// Tasks view — table with filters, create/edit form, and lifecycle actions.
// ---------------------------------------------------------------------------

interface TasksViewProps {
  client: CoreClient;
}

export function TasksView({ client }: TasksViewProps) {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [filter, setFilter] = useState<TaskFilter>("open");
  const [contacts, setContacts] = useState<ContactListItem[]>([]);
  const [companies, setCompanies] = useState<Company[]>([]);
  const [opportunities, setOpportunities] = useState<OpportunityListItem[]>([]);
  // null = closed, "new" = creating, otherwise the task being edited.
  const [editing, setEditing] = useState<Task | "new" | null>(null);
  const [draft, setDraft] = useState<TaskDraft>(EMPTY_DRAFT);
  const [logToTimeline, setLogToTimeline] = useState(false);
  const [error, setError] = useState<SaveError>(NO_SAVE_ERROR);
  const [loadError, setLoadError] = useState(false);

  const load = useCallback(() => {
    client
      .listTasks(filterRequest(filter))
      .then((rows) => {
        setTasks(rows);
        setLoadError(false);
      })
      .catch(() => setLoadError(true));
  }, [client, filter]);

  useEffect(load, [load]);

  // Parent picker sources — loaded once, archived included so links resolve.
  useEffect(() => {
    client.listContacts(true).then(setContacts).catch(() => {});
    client.listCompanies(true).then(setCompanies).catch(() => {});
    client.listOpportunities(true).then(setOpportunities).catch(() => {});
  }, [client]);

  const parentOptions: ParentOption[] = [
    ...contacts.map((contact) => ({
      value: `contact:${contact.id}`,
      label: `Contact — ${contact.displayName}`,
    })),
    ...companies.map((company) => ({
      value: `company:${company.id}`,
      label: `Company — ${company.name}`,
    })),
    ...opportunities.map((opportunity) => ({
      value: `opportunity:${opportunity.id}`,
      label: `Opportunity — ${opportunity.name}`,
    })),
  ];

  const parentLabel = (task: Task): string => {
    if (!task.parentType || !task.parentId) return "—";
    const option = parentOptions.find(
      (candidate) => candidate.value === parentValue(task.parentType, task.parentId),
    );
    return option?.label ?? task.parentId;
  };

  const openEditor = (target: Task | "new") => {
    setError(NO_SAVE_ERROR);
    setLogToTimeline(false);
    setDraft(target === "new" ? EMPTY_DRAFT : draftFrom(target));
    setEditing(target);
  };

  const closeEditor = () => {
    setEditing(null);
    setError(NO_SAVE_ERROR);
  };

  const set = <K extends keyof TaskDraft>(key: K, value: TaskDraft[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(NO_SAVE_ERROR);
    const existing = editing !== null && editing !== "new" ? editing : null;
    const patch = patchFrom(draft, existing);
    try {
      existing
        ? await client.updateTask({ taskId: existing.id, expectedVersion: existing.version, patch })
        : await client.createTask(patch);
      closeEditor();
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  // Lifecycle actions run against the loaded version and reload the list.
  const act = async (action: () => Promise<unknown>) => {
    setError(NO_SAVE_ERROR);
    try {
      await action();
      closeEditor();
      load();
    } catch (rejection) {
      setError(saveErrorFrom(rejection));
    }
  };

  const editingTask = editing !== null && editing !== "new" ? editing : null;

  const columns: ColumnDef<Task>[] = [
    {
      key: "title",
      header: "Title",
      render: (task) => <span className="cell-primary">{task.title}</span>,
    },
    { key: "parent", header: "Linked to", render: parentLabel },
    {
      key: "due",
      header: "Due",
      render: (task) =>
        task.dueAt === null ? (
          "—"
        ) : isOverdue(task) ? (
          // Attention hue plus a text label — never hue alone.
          <span className="cell-attention">
            {task.dueAt}
            <span className="cell-flag cell-flag--attention">Overdue</span>
          </span>
        ) : (
          task.dueAt
        ),
    },
    { key: "priority", header: "Priority", render: (task) => priorityLabel(task.priority) },
    { key: "status", header: "Status", render: (task) => STATUS_LABELS[task.status] },
  ];

  return (
    <section className="crm-section" aria-label="Tasks">
      <div className="section-rule">
        <h2>Tasks</h2>
        <div className="list-tools">
          <div className="mode-switch" role="group" aria-label="Task filter">
            <button type="button" aria-pressed={filter === "open"} onClick={() => setFilter("open")}>
              Open
            </button>
            <button type="button" aria-pressed={filter === "all"} onClick={() => setFilter("all")}>
              All
            </button>
            <button
              type="button"
              aria-pressed={filter === "overdue"}
              onClick={() => setFilter("overdue")}
            >
              Overdue
            </button>
          </div>
          <span className="list-count">{tasks?.length ?? 0}</span>
          <button
            type="button"
            className="button button--primary"
            onClick={() => openEditor("new")}
          >
            New task
          </button>
        </div>
      </div>

      {loadError ? <GeneralError message="Could not read tasks from the local database." /> : null}

      {editing !== null ? (
        <form className="record-form task-editor" onSubmit={submit} aria-label="Task form">
          <h3 className="detail-subhead">{editingTask ? "Edit task" : "New task"}</h3>
          <GeneralError message={error.general} />
          <div className="form-grid">
            <Field label="Title" error={error.fields.title}>
              <input value={draft.title} onChange={(event) => set("title", event.target.value)} />
            </Field>
            <Field label="Linked to" error={error.fields.parentId ?? error.fields.parentType}>
              <select value={draft.parent} onChange={(event) => set("parent", event.target.value)}>
                <option value="">No linked record</option>
                {parentOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Due" error={error.fields.dueAt}>
              {draft.dueAt === "" ? (
                <button
                  type="button"
                  className="button"
                  aria-label="Set due date"
                  onClick={() => set("dueAt", isoToLocalInput(new Date().toISOString()))}
                >
                  Set due date
                </button>
              ) : (
                <>
                  <input
                    type="datetime-local"
                    value={draft.dueAt}
                    onChange={(event) => set("dueAt", event.target.value)}
                  />
                  <button
                    type="button"
                    className="button"
                    aria-label="Clear due date"
                    onClick={() => set("dueAt", "")}
                  >
                    Clear due date
                  </button>
                </>
              )}
            </Field>
            <Field label="Remind" error={error.fields.remindAt}>
              {draft.remindAt === "" ? (
                <button
                  type="button"
                  className="button"
                  aria-label="Set reminder"
                  onClick={() => set("remindAt", isoToLocalInput(new Date().toISOString()))}
                >
                  Set reminder
                </button>
              ) : (
                <>
                  <input
                    type="datetime-local"
                    value={draft.remindAt}
                    onChange={(event) => set("remindAt", event.target.value)}
                  />
                  <button
                    type="button"
                    className="button"
                    aria-label="Clear reminder"
                    onClick={() => set("remindAt", "")}
                  >
                    Clear reminder
                  </button>
                </>
              )}
            </Field>
            <Field label="Priority" error={error.fields.priority}>
              <select
                value={draft.priority}
                onChange={(event) => set("priority", event.target.value as TaskPriority)}
              >
                {PRIORITY_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </Field>
          </div>

          {editingTask ? (
            <div className="task-actions">
              {editingTask.status === "open" ? (
                <>
                  <label className="toggle">
                    <input
                      type="checkbox"
                      checked={logToTimeline}
                      disabled={!editingTask.parentId}
                      onChange={(event) => setLogToTimeline(event.target.checked)}
                    />
                    <span>Log to timeline</span>
                  </label>
                  <button
                    type="button"
                    className="button"
                    onClick={() =>
                      act(() =>
                        client.completeTask({
                          taskId: editingTask.id,
                          expectedVersion: editingTask.version,
                          logActivity: logToTimeline,
                        }),
                      )
                    }
                  >
                    Complete
                  </button>
                  <button
                    type="button"
                    className="button"
                    onClick={() =>
                      act(() =>
                        client.dropTask({
                          taskId: editingTask.id,
                          expectedVersion: editingTask.version,
                        }),
                      )
                    }
                  >
                    Drop
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="button"
                  onClick={() =>
                    act(() =>
                      client.reopenTask({
                        taskId: editingTask.id,
                        expectedVersion: editingTask.version,
                      }),
                    )
                  }
                >
                  Reopen
                </button>
              )}
              <button
                type="button"
                className="button"
                onClick={() => {
                  if (!window.confirm(`Delete task "${editingTask.title}"?`)) return;
                  void act(() =>
                    client.deleteTask({
                      taskId: editingTask.id,
                      expectedVersion: editingTask.version,
                    }),
                  );
                }}
              >
                Delete
              </button>
            </div>
          ) : null}

          <div className="form-actions">
            <button type="button" className="button" onClick={closeEditor}>
              Cancel
            </button>
            <button type="submit" className="button button--primary">
              {editingTask ? "Save task" : "Create task"}
            </button>
          </div>
        </form>
      ) : null}

      {tasks && tasks.length === 0 ? (
        <div className="empty-state">
          <span className="registration-mark" aria-hidden="true" />
          <p className="eyebrow">Ready when you are</p>
          <h2>No tasks here</h2>
          <p>Follow-ups, reminders, and to-dos — linked to a record or on their own.</p>
        </div>
      ) : null}

      {tasks && tasks.length > 0 ? (
        <RecordTable label="Task list" columns={columns} rows={tasks} onOpen={openEditor} />
      ) : null}
    </section>
  );
}
