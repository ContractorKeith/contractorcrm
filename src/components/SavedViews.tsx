import { useCallback, useEffect, useRef, useState } from "react";

import type { CoreClient } from "../api/client";
import {
  isCommandError,
  type SavedView,
  type SavedViewDefinition,
  type SavedViewEntityType,
} from "../api/types";

type DialogMode = "create" | "rename" | "delete" | null;

interface SavedViewsProps {
  client: CoreClient;
  entityType: SavedViewEntityType;
  definition: SavedViewDefinition;
  onApply: (definition: SavedViewDefinition) => void;
  onSelectionChange?: (selected: boolean) => void;
}

const surfaceLabel: Record<SavedViewEntityType, string> = {
  contact: "contact",
  company: "company",
  opportunity: "pipeline",
};

export function SavedViews({
  client,
  entityType,
  definition,
  onApply,
  onSelectionChange,
}: SavedViewsProps) {
  const [views, setViews] = useState<SavedView[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [dialog, setDialog] = useState<DialogMode>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const selectRef = useRef<HTMLSelectElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const closeFocusRef = useRef<HTMLElement | null>(null);

  const selected = views.find((view) => view.id === selectedId) ?? null;
  const modified = Boolean(
    selected && JSON.stringify(selected.definition) !== JSON.stringify(definition),
  );

  const load = useCallback(async () => {
    try {
      const next = await client.listSavedViews(entityType);
      setViews(next);
      setError(null);
      setSelectedId((current) => (next.some((view) => view.id === current) ? current : ""));
    } catch (rejection) {
      setViews([]);
      setSelectedId("");
      onSelectionChange?.(false);
      setError(
        isCommandError(rejection) && rejection.kind === "invalid_stored_data"
          ? "A saved view has an unsupported or damaged definition. It was not changed."
          : "Saved views could not be read from the local database.",
      );
    }
  }, [client, entityType, onSelectionChange]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (dialog) {
      (dialog === "delete" ? cancelRef.current : nameRef.current)?.focus();
    } else if (closeFocusRef.current) {
      closeFocusRef.current.focus();
      closeFocusRef.current = null;
    }
  }, [dialog]);

  const openDialog = (mode: Exclude<DialogMode, null>, trigger: HTMLElement) => {
    restoreFocusRef.current = trigger;
    setError(null);
    setName(mode === "rename" ? (selected?.name ?? "") : "");
    setDialog(mode);
  };

  const closeDialog = (focusTarget: HTMLElement | null = restoreFocusRef.current) => {
    closeFocusRef.current = focusTarget;
    setDialog(null);
    setName("");
  };

  const mutationError = async (rejection: unknown) => {
    if (isCommandError(rejection) && rejection.kind === "version_conflict") {
      setError("This saved view changed elsewhere. The latest version has been reloaded.");
      await load();
    } else if (
      isCommandError(rejection) &&
      (rejection.kind === "validation_failed" || rejection.kind === "invalid_input")
    ) {
      setError(rejection.message);
    } else {
      setError(isCommandError(rejection) ? rejection.message : "The saved view could not be changed.");
    }
  };

  const saveName = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim()) {
      setError("View name is required.");
      nameRef.current?.focus();
      return;
    }
    try {
      const saved =
        dialog === "create"
          ? await client.createSavedView({ name: name.trim(), entityType, definition })
          : selected
            ? await client.updateSavedView({
                savedViewId: selected.id,
                expectedVersion: selected.version,
                name: name.trim(),
                definition: selected.definition,
              })
            : null;
      if (!saved) return;
      await load();
      setSelectedId(saved.id);
      onSelectionChange?.(true);
      setStatus(dialog === "create" ? `${saved.name} saved.` : `${saved.name} renamed.`);
      closeDialog(selectRef.current);
    } catch (rejection) {
      await mutationError(rejection);
    }
  };

  const update = async () => {
    if (!selected) return;
    try {
      const saved = await client.updateSavedView({
        savedViewId: selected.id,
        expectedVersion: selected.version,
        name: selected.name,
        definition,
      });
      await load();
      setSelectedId(saved.id);
      setStatus(`${saved.name} updated.`);
      selectRef.current?.focus();
    } catch (rejection) {
      await mutationError(rejection);
    }
  };

  const remove = async () => {
    if (!selected) return;
    try {
      const deletedName = selected.name;
      await client.deleteSavedView({
        savedViewId: selected.id,
        expectedVersion: selected.version,
      });
      await load();
      setSelectedId("");
      onSelectionChange?.(false);
      setStatus(`${deletedName} deleted.`);
      closeDialog(selectRef.current);
    } catch (rejection) {
      await mutationError(rejection);
    }
  };

  const apply = (id: string) => {
    setSelectedId(id);
    onSelectionChange?.(id !== "");
    const view = views.find((candidate) => candidate.id === id);
    if (!view) {
      setStatus("Current view selected.");
      return;
    }
    onApply(view.definition);
    setStatus(`${view.name} applied.`);
  };

  const trapDialogFocus = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeDialog();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        "button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex='-1'])",
      ) ?? [],
    );
    const first = focusable[0];
    const last = focusable.at(-1);
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div className="saved-views">
      <label className="saved-views__picker">
        <span>Saved view</span>
        <select
          ref={selectRef}
          value={selectedId}
          onChange={(event) => apply(event.target.value)}
          aria-label={`Saved ${surfaceLabel[entityType]} view`}
        >
          <option value="">Current view</option>
          {views.map((view) => (
            <option key={view.id} value={view.id}>
              {view.name}
            </option>
          ))}
        </select>
      </label>
      <button type="button" className="button" onClick={(event) => openDialog("create", event.currentTarget)}>
        Save view
      </button>
      {selected ? (
        <>
          {modified ? <span className="cell-flag">Modified</span> : null}
          {/* Named after the selected view — "Update"/"Delete" alone say nothing
              once the control is read out of its visual context. */}
          <button
            type="button"
            className="button"
            aria-label={`Update ${selected.name}`}
            onClick={() => void update()}
          >
            Update
          </button>
          <button
            type="button"
            className="button"
            aria-label={`Rename ${selected.name}`}
            onClick={(event) => openDialog("rename", event.currentTarget)}
          >
            Rename
          </button>
          <button
            type="button"
            className="button"
            aria-label={`Delete ${selected.name}`}
            onClick={(event) => openDialog("delete", event.currentTarget)}
          >
            Delete
          </button>
        </>
      ) : null}
      <span className="saved-views__status" role="status" aria-live="polite">
        {status}
      </span>
      {error ? <span className="saved-views__error" role="alert">{error}</span> : null}

      {dialog ? (
        <div className="global-search-backdrop" onMouseDown={(event) => event.target === event.currentTarget && closeDialog()}>
          <section
            ref={dialogRef}
            className="saved-views__dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="saved-view-dialog-title"
            onKeyDown={trapDialogFocus}
          >
            {dialog === "delete" ? (
              <>
                <h2 id="saved-view-dialog-title">Delete {selected?.name}?</h2>
                <p>This removes the saved definition. It does not delete any records.</p>
                <div className="form-actions">
                  <button ref={cancelRef} type="button" className="button" onClick={() => closeDialog()}>Cancel</button>
                  <button type="button" className="button button--primary" onClick={() => void remove()}>Delete view</button>
                </div>
              </>
            ) : (
              <form onSubmit={(event) => void saveName(event)}>
                <h2 id="saved-view-dialog-title">{dialog === "create" ? "Save current view" : "Rename saved view"}</h2>
                <label className="field">
                  <span className="field__label">View name</span>
                  <input
                    ref={nameRef}
                    required
                    maxLength={120}
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                  />
                </label>
                {error ? <p className="form-error" role="alert">{error}</p> : null}
                <div className="form-actions">
                  <button type="button" className="button" onClick={() => closeDialog()}>Cancel</button>
                  <button type="submit" className="button button--primary">{dialog === "create" ? "Save view" : "Rename view"}</button>
                </div>
              </form>
            )}
          </section>
        </div>
      ) : null}
    </div>
  );
}
