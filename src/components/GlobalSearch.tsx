import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { CoreClient } from "../api/client";
import type { SearchResult } from "../api/types";

interface GlobalSearchProps {
  client: CoreClient;
  onOpenResult: (result: SearchResult) => Promise<boolean>;
}

type ResultGroup = {
  label: string;
  results: SearchResult[];
};

const entityLabels: Record<SearchResult["entityType"], string> = {
  contact: "Contact",
  company: "Company",
  opportunity: "Opportunity",
  activity: "Activity",
};

const isEditable = (target: EventTarget | null) => {
  if (!(target instanceof Element)) return false;
  return Boolean(
    target.closest("input, textarea, select, [contenteditable]:not([contenteditable='false'])"),
  );
};

const isMac = () => /Mac|iPhone|iPad|iPod/.test(navigator.platform);

const optionId = (result: SearchResult, index: number) =>
  `global-search-option-${index}-${result.entityType}-${result.entityId.replace(/[^a-zA-Z0-9_-]/g, "-")}`;

export function GlobalSearch({ client, onOpenResult }: GlobalSearchProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [groups, setGroups] = useState<ResultGroup[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const requestRef = useRef(0);

  const results = useMemo(() => groups.flatMap((group) => group.results), [groups]);
  const selected = results[selectedIndex];

  const show = useCallback((invoker?: HTMLElement | null) => {
    restoreFocusRef.current = invoker ?? (document.activeElement as HTMLElement | null);
    setOpen(true);
  }, []);

  const close = useCallback(() => {
    setOpen(false);
    setQuery("");
    setGroups([]);
    requestAnimationFrame(() => restoreFocusRef.current?.focus());
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const shortcut =
        event.key.toLowerCase() === "k" &&
        !event.altKey &&
        !event.shiftKey &&
        (isMac() ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey);

      if (shortcut && (!isEditable(event.target) || open)) {
        event.preventDefault();
        if (open) inputRef.current?.focus();
        else show(event.target instanceof HTMLElement ? event.target : null);
      } else if (event.key === "Escape" && open) {
        event.preventDefault();
        close();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [close, open, show]);

  useEffect(() => {
    if (!open) return;
    inputRef.current?.focus();
    const request = ++requestRef.current;
    setLoading(true);
    setError(false);

    const trimmed = query.trim();
    const load = trimmed
      ? client.searchRecords(trimmed, undefined, 25).then((searchResults) => [
          { label: "Search results", results: searchResults },
        ])
      : Promise.all([client.listRecentRecords(), client.listFavoriteContacts()]).then(
          ([recent, favorites]) => [
            { label: "Recent records", results: recent },
            { label: "Favorite contacts", results: favorites },
          ],
        );

    load
      .then((nextGroups) => {
        if (request !== requestRef.current) return;
        setGroups(nextGroups);
        setSelectedIndex(0);
      })
      .catch(() => {
        if (request !== requestRef.current) return;
        setGroups([]);
        setError(true);
      })
      .finally(() => {
        if (request === requestRef.current) setLoading(false);
      });
  }, [client, open, query]);

  const openResult = async (result: SearchResult | undefined) => {
    if (!result) return;
    if (await onOpenResult(result)) close();
  };

  const onInputKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (results.length === 0) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIndex((index) => (index + 1) % results.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIndex((index) => (index - 1 + results.length) % results.length);
    } else if (event.key === "Home") {
      event.preventDefault();
      setSelectedIndex(0);
    } else if (event.key === "End") {
      event.preventDefault();
      setSelectedIndex(results.length - 1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      void openResult(selected);
    }
  };

  const trapDialogFocus = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key !== "Tab") return;
    const focusable = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        "button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex='-1'])",
      ) ?? [],
    );
    if (focusable.length === 0) return;
    const first = focusable[0]!;
    const last = focusable.at(-1)!;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const status = loading
    ? "Searching"
    : error
      ? "Search unavailable"
      : query.trim()
        ? `${results.length} ${results.length === 1 ? "result" : "results"}`
        : `${results.length} ${results.length === 1 ? "suggestion" : "suggestions"}`;

  return (
    <>
      <button
        type="button"
        className="global-search-trigger"
        aria-keyshortcuts={isMac() ? "Meta+K" : "Control+K"}
        onClick={(event) => show(event.currentTarget)}
      >
        <span>Search</span>
        <kbd>{isMac() ? "⌘K" : "Ctrl K"}</kbd>
      </button>

      {open ? (
        <div
          className="global-search-backdrop"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) close();
          }}
        >
          <section
            ref={dialogRef}
            className="global-search-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="global-search-title"
            onKeyDown={trapDialogFocus}
          >
            <div className="global-search-head">
              <h2 id="global-search-title">Search ContractorCRM</h2>
              <button type="button" className="global-search-close" onClick={close}>
                Close
              </button>
            </div>
            <input
              ref={inputRef}
              className="global-search-input"
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={onInputKeyDown}
              placeholder="Search contacts, companies, opportunities, and activities"
              aria-label="Search records"
              role="combobox"
              aria-autocomplete="list"
              aria-expanded="true"
              aria-controls="global-search-results"
              aria-activedescendant={selected ? optionId(selected, selectedIndex) : undefined}
            />
            <p className="global-search-status" role="status" aria-live="polite">
              {status}
            </p>

            <div id="global-search-results" className="global-search-results" role="listbox">
              {!loading && !error && results.length === 0 ? (
                <p className="global-search-empty">
                  {query.trim() ? "No matching records." : "No recent records or favorite contacts."}
                </p>
              ) : null}
              {error ? <p className="global-search-empty">Search is unavailable. Try again.</p> : null}
              {groups.map((group) => {
                if (group.results.length === 0) return null;
                const groupStart = results.indexOf(group.results[0]!);
                return (
                  <section
                    className="global-search-group"
                    key={group.label}
                    role="group"
                    aria-label={group.label}
                  >
                    <h3>{group.label}</h3>
                    {group.results.map((result, groupIndex) => {
                      const index = groupStart + groupIndex;
                      const active = index === selectedIndex;
                      return (
                        <div
                          id={optionId(result, index)}
                          className="global-search-option"
                          role="option"
                          aria-selected={active}
                          key={`${group.label}-${result.entityType}-${result.entityId}`}
                          onMouseEnter={() => setSelectedIndex(index)}
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={() => void openResult(result)}
                        >
                          <span>{result.title}</span>
                          <small>{entityLabels[result.entityType]}</small>
                        </div>
                      );
                    })}
                  </section>
                );
              })}
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
}
