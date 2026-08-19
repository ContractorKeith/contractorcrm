import { useCallback, useEffect, useRef, useState } from "react";

import type { CoreClient } from "../api/client";
import { isCommandError, type CustomFieldDefinition, type RecordCustomFieldValue, type RecordMetadata as Metadata, type SavedViewEntityType, type Tag } from "../api/types";
import { MetadataDefinitionsDialog } from "./MetadataDefinitionsDialog";

interface RecordMetadataProps {
  client: CoreClient;
  entityType: SavedViewEntityType;
  recordId: string;
  expectedVersion: number;
  /** Call with the updated owner version after a successful metadata write. */
  onSaved?: () => void;
}

const emptyValue = (definition: CustomFieldDefinition): RecordCustomFieldValue => ({
  definitionId: definition.id, textValue: null, numberValue: null, dateValue: null, optionId: null,
});

export function RecordMetadata({ client, entityType, recordId, expectedVersion, onSaved }: RecordMetadataProps) {
  const [tags, setTags] = useState<Tag[]>([]);
  const [definitions, setDefinitions] = useState<CustomFieldDefinition[]>([]);
  const [metadata, setMetadata] = useState<Metadata>({ tagIds: [], values: [] });
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [managing, setManaging] = useState(false);
  const manageButtonRef = useRef<HTMLButtonElement>(null);

  const load = useCallback(async () => {
    try {
      const [nextTags, nextDefinitions, nextMetadata] = await Promise.all([
        client.listTags(true),
        client.listCustomFieldDefs(entityType, true),
        client.getRecordMetadata(entityType, recordId),
      ]);
      setTags(nextTags);
      setDefinitions(nextDefinitions);
      setMetadata(nextMetadata);
      setError(null);
    } catch {
      setError("Metadata could not be read from the local database.");
    }
  }, [client, entityType, recordId]);

  useEffect(() => { void load(); }, [load]);

  const assignTag = (id: string) => setMetadata((current) => ({ ...current, tagIds: current.tagIds.includes(id) ? current.tagIds : [...current.tagIds, id] }));
  const removeTag = (id: string) => setMetadata((current) => ({ ...current, tagIds: current.tagIds.filter((tagId) => tagId !== id) }));
  const setValue = (definition: CustomFieldDefinition, value: string) => setMetadata((current) => {
    const next = emptyValue(definition);
    if (definition.fieldType === "text") next.textValue = value || null;
    if (definition.fieldType === "number") next.numberValue = value === "" ? null : Number(value);
    if (definition.fieldType === "date") next.dateValue = value || null;
    if (definition.fieldType === "select") next.optionId = value || null;
    return { ...current, values: [...current.values.filter((item) => item.definitionId !== definition.id), ...(value === "" ? [] : [next])] };
  });
  const save = async () => {
    try {
      const values = metadata.values.map(({ definitionId, textValue, numberValue, dateValue, optionId }) => ({
        definitionId,
        textValue,
        numberValue,
        dateValue,
        optionId,
      }));
      const saved = await client.setRecordMetadata({ entityType, recordId, expectedVersion, tagIds: metadata.tagIds, values });
      setMetadata(saved); setError(null); setStatus("Metadata saved."); onSaved?.();
    } catch (reason) {
      if (isCommandError(reason) && reason.kind === "version_conflict") {
        setError("This record changed elsewhere. The latest metadata has been reloaded.");
        onSaved?.();
        await load();
      } else {
        setError(isCommandError(reason) ? reason.message : "Metadata could not be saved.");
      }
    }
  };
  const valueFor = (definition: CustomFieldDefinition) => metadata.values.find((item) => item.definitionId === definition.id);
  const activeTags = tags.filter((tag) => metadata.tagIds.includes(tag.id));
  const availableTags = tags.filter((tag) => !tag.archivedAt && !metadata.tagIds.includes(tag.id));
  return <section className="record-metadata" aria-labelledby="record-metadata-title">
    <div className="record-metadata__heading">
      <h3 id="record-metadata-title">Tags and custom fields</h3>
      <button ref={manageButtonRef} type="button" className="button" onClick={() => setManaging(true)}>Manage tags and fields</button>
    </div>
    {/* role=group gives the aria-label a home; a bare <div> would drop it. */}
    <div className="record-metadata__tags" role="group" aria-label="Assigned tags">
      {activeTags.map((tag) => <span key={tag.id} className="metadata-tag">{tag.label}{tag.archivedAt ? " (archived)" : null}<button type="button" aria-label={`Remove ${tag.label} tag`} onClick={() => removeTag(tag.id)}>×</button></span>)}
      <label> Add tag <select value="" onChange={(event) => assignTag(event.target.value)}><option value="">Select a tag</option>{availableTags.map((tag) => <option key={tag.id} value={tag.id}>{tag.label}</option>)}</select></label>
    </div>
    {definitions.map((definition) => {
      const value = valueFor(definition);
      const common = { id: `metadata-${definition.id}`, "aria-label": definition.label };
      return <div key={definition.id} className="field"><label><span className="field__label">{definition.label}{definition.archivedAt ? " (archived)" : null}</span>
        {definition.fieldType === "select" ? <select {...common} value={value?.optionId ?? ""} disabled={Boolean(definition.archivedAt)} onChange={(event) => setValue(definition, event.target.value)}><option value="">No value</option>{definition.options.map((option) => <option key={option.id} value={option.id}>{option.label}</option>)}</select> :
          <input {...common} type={definition.fieldType === "number" ? "number" : definition.fieldType === "date" ? "date" : "text"} disabled={Boolean(definition.archivedAt)} value={definition.fieldType === "text" ? value?.textValue ?? "" : definition.fieldType === "number" ? value?.numberValue ?? "" : value?.dateValue ?? ""} onChange={(event) => setValue(definition, event.target.value)} />}
        </label>{definition.archivedAt && value ? <button type="button" className="button" aria-label={`Clear archived ${definition.label}`} onClick={() => setValue(definition, "")}>Clear value</button> : null}</div>;
    })}
    <button type="button" className="button button--primary" onClick={() => void save()}>Save metadata</button>
    <span role="status" aria-live="polite">{status}</span>{error ? <p role="alert" className="form-error">{error}</p> : null}
    {managing ? <MetadataDefinitionsDialog client={client} entityType={entityType} onClose={() => {
      setManaging(false);
      void load();
      queueMicrotask(() => manageButtonRef.current?.focus());
    }} /> : null}
  </section>;
}
