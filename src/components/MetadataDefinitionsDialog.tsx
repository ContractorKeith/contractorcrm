import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import type { CoreClient } from "../api/client";
import {
  isCommandError,
  type CustomFieldDefinition,
  type CustomFieldOptionInput,
  type CustomFieldType,
  type SavedViewEntityType,
  type Tag,
  type TagColorRole,
} from "../api/types";

interface Props {
  client: CoreClient;
  entityType: SavedViewEntityType;
  onClose: () => void;
}

type Mode = "tag" | "field";
type PendingLifecycle = { kind: Mode; item: Tag | CustomFieldDefinition } | null;

export function MetadataDefinitionsDialog({ client, entityType, onClose }: Props) {
  const [mode, setMode] = useState<Mode>("tag");
  const [tags, setTags] = useState<Tag[]>([]);
  const [definitions, setDefinitions] = useState<CustomFieldDefinition[]>([]);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [label, setLabel] = useState("");
  const [colorRole, setColorRole] = useState<TagColorRole | "">("");
  const [fieldType, setFieldType] = useState<CustomFieldType>("text");
  const [options, setOptions] = useState<CustomFieldOptionInput[]>([]);
  const [pendingLifecycle, setPendingLifecycle] = useState<PendingLifecycle>(null);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const dialogRef = useRef<HTMLElement>(null);
  const confirmRef = useRef<HTMLDivElement>(null);
  const labelRef = useRef<HTMLInputElement>(null);
  const confirmCancelRef = useRef<HTMLButtonElement>(null);
  const lifecycleTriggerRef = useRef<HTMLButtonElement | null>(null);

  const load = useCallback(async () => {
    try {
      const [nextTags, nextDefinitions] = await Promise.all([
        client.listTags(true),
        client.listCustomFieldDefs(entityType, true),
      ]);
      setTags(nextTags);
      setDefinitions(nextDefinitions);
    } catch {
      setError("Definitions could not be read from the local database.");
    }
  }, [client, entityType]);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => { labelRef.current?.focus(); }, [mode]);
  useEffect(() => { if (pendingLifecycle) confirmCancelRef.current?.focus(); }, [pendingLifecycle]);

  const resetEditor = () => {
    setEditingId(null);
    setLabel("");
    setColorRole("");
    setFieldType("text");
    setOptions([]);
  };

  const failure = async (reason: unknown) => {
    if (isCommandError(reason) && reason.kind === "version_conflict") {
      setError("This definition changed elsewhere. The latest version has been reloaded.");
      resetEditor();
      await load();
    } else {
      setError(isCommandError(reason) ? reason.message : "The definition could not be changed.");
    }
  };

  const saveDefinition = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!label.trim()) {
      setError("A label is required.");
      labelRef.current?.focus();
      return;
    }
    try {
      if (mode === "tag") {
        const existing = tags.find((tag) => tag.id === editingId);
        if (existing) {
          await client.updateTag({
            tagId: existing.id,
            expectedVersion: existing.version,
            label: label.trim(),
            colorRole: colorRole || null,
          });
        } else {
          await client.createTag({ label: label.trim(), colorRole: colorRole || null });
        }
      } else {
        const existing = definitions.find((definition) => definition.id === editingId);
        if (existing) {
          await client.updateCustomFieldDef({
            definitionId: existing.id,
            expectedVersion: existing.version,
            label: label.trim(),
            sortKey: existing.sortKey,
            options,
          });
        } else {
          await client.createCustomFieldDef({ entityType, label: label.trim(), fieldType, options });
        }
      }
      setStatus(`${mode === "tag" ? "Tag" : "Custom field"} ${editingId ? "updated" : "created"}.`);
      setError("");
      resetEditor();
      await load();
      labelRef.current?.focus();
    } catch (reason) {
      await failure(reason);
    }
  };

  const beginTagEdit = (tag: Tag) => {
    setMode("tag");
    setEditingId(tag.id);
    setLabel(tag.label);
    setColorRole(tag.colorRole ?? "");
    setOptions([]);
    queueMicrotask(() => labelRef.current?.focus());
  };

  const beginFieldEdit = (definition: CustomFieldDefinition) => {
    setMode("field");
    setEditingId(definition.id);
    setLabel(definition.label);
    setFieldType(definition.fieldType);
    setOptions(definition.options.map((option) => ({ id: option.id, label: option.label })));
    queueMicrotask(() => labelRef.current?.focus());
  };

  const applyLifecycle = async () => {
    if (!pendingLifecycle) return;
    const { kind, item } = pendingLifecycle;
    try {
      if (kind === "tag") {
        const tag = item as Tag;
        await (tag.archivedAt
          ? client.unarchiveTag({ tagId: tag.id, expectedVersion: tag.version })
          : client.archiveTag({ tagId: tag.id, expectedVersion: tag.version }));
      } else {
        const definition = item as CustomFieldDefinition;
        await (definition.archivedAt
          ? client.unarchiveCustomFieldDef({ definitionId: definition.id, expectedVersion: definition.version })
          : client.archiveCustomFieldDef({ definitionId: definition.id, expectedVersion: definition.version }));
      }
      setStatus(`${item.label} ${item.archivedAt ? "restored" : "archived"}.`);
      setError("");
      setPendingLifecycle(null);
      queueMicrotask(() => lifecycleTriggerRef.current?.focus());
      resetEditor();
      await load();
    } catch (reason) {
      setPendingLifecycle(null);
      await failure(reason);
    }
  };

  const trapFocus = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      if (pendingLifecycle) {
        setPendingLifecycle(null);
        queueMicrotask(() => lifecycleTriggerRef.current?.focus());
      }
      else onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const focusRoot = pendingLifecycle ? confirmRef.current : dialogRef.current;
    const nodes = Array.from(focusRoot?.querySelectorAll<HTMLElement>(
      "button:not([disabled]),input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex='-1'])",
    ) ?? []);
    const first = nodes[0];
    const last = nodes.at(-1);
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last?.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first?.focus();
    }
  };

  const items = mode === "tag" ? tags : definitions;
  const editing = editingId !== null;

  return createPortal(<div className="global-search-backdrop">
    <section ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="metadata-definitions-title" className="saved-views__dialog" onKeyDown={trapFocus}>
      <h2 id="metadata-definitions-title">Manage tags and custom fields</h2>
      <div role="group" aria-label="Metadata definitions">
        <button type="button" aria-pressed={mode === "tag"} onClick={() => { setMode("tag"); resetEditor(); }}>Tags</button>
        <button type="button" aria-pressed={mode === "field"} onClick={() => { setMode("field"); resetEditor(); }}>Custom fields</button>
      </div>

      <form onSubmit={(event) => void saveDefinition(event)}>
        <label className="field">
          <span className="field__label">{mode === "tag" ? "Tag label" : "Field label"}</span>
          <input ref={labelRef} value={label} maxLength={mode === "tag" ? 80 : 120} onChange={(event) => setLabel(event.target.value)} />
        </label>
        {mode === "tag" ? <label className="field">
          <span className="field__label">Color role</span>
          <select value={colorRole} onChange={(event) => setColorRole(event.target.value as TagColorRole | "")}>
            <option value="">None</option><option value="neutral">Neutral</option><option value="accent">Accent</option><option value="attention">Attention</option>
          </select>
        </label> : <>
          <label className="field">
            <span className="field__label">Field type</span>
            <select value={fieldType} disabled={editing} onChange={(event) => setFieldType(event.target.value as CustomFieldType)}>
              {(["text", "number", "date", "select"] as const).map((type) => <option key={type}>{type}</option>)}
            </select>
          </label>
          {fieldType === "select" ? <div className="metadata-options">
            <button type="button" onClick={() => setOptions((current) => [...current, { label: "" }])}>Add option</button>
            {options.map((option, index) => <div key={option.id ?? `new-${index}`} className="metadata-option">
              <label className="field"><span className="field__label">Option {index + 1}</span><input value={option.label} onChange={(event) => setOptions((current) => current.map((item, itemIndex) => itemIndex === index ? { ...item, label: event.target.value } : item))} /></label>
              <button type="button" aria-label={`Remove option ${index + 1}`} onClick={() => setOptions((current) => current.filter((_, itemIndex) => itemIndex !== index))}>Remove</button>
            </div>)}
          </div> : null}
        </>}
        <button type="submit" className="button button--primary">{editing ? "Update" : "Create"} {mode === "tag" ? "tag" : "field"}</button>
        {editing ? <button type="button" className="button" onClick={() => { resetEditor(); labelRef.current?.focus(); }}>Cancel edit</button> : null}
      </form>

      {items.length === 0 ? <p>No {mode === "tag" ? "tags" : "custom fields"} yet.</p> : <ul aria-label={mode === "tag" ? "Tags" : "Custom fields"}>
        {items.map((item) => <li key={item.id}>
          <span>{item.label}{item.archivedAt ? " (archived)" : ""}</span>
          <button type="button" onClick={() => mode === "tag" ? beginTagEdit(item as Tag) : beginFieldEdit(item as CustomFieldDefinition)}>Edit {item.label}</button>
          <button type="button" onClick={(event) => { lifecycleTriggerRef.current = event.currentTarget; setPendingLifecycle({ kind: mode, item }); }}>{item.archivedAt ? "Restore" : "Archive"} {item.label}</button>
        </li>)}
      </ul>}

      {pendingLifecycle ? <div ref={confirmRef} role="alertdialog" aria-modal="true" aria-labelledby="metadata-lifecycle-title" className="metadata-confirm">
        <h3 id="metadata-lifecycle-title">{pendingLifecycle.item.archivedAt ? "Restore" : "Archive"} {pendingLifecycle.item.label}?</h3>
        <p>Existing assignments and values will be preserved.</p>
        <button ref={confirmCancelRef} type="button" className="button" onClick={() => { setPendingLifecycle(null); queueMicrotask(() => lifecycleTriggerRef.current?.focus()); }}>Cancel</button>
        <button type="button" className="button button--primary" onClick={() => void applyLifecycle()}>{pendingLifecycle.item.archivedAt ? "Restore" : "Archive"}</button>
      </div> : null}

      <button type="button" className="button" onClick={onClose}>Close</button>
      <p role="status" aria-live="polite">{status}</p>
      {error ? <p role="alert" className="form-error">{error}</p> : null}
    </section>
  </div>, document.body);
}
