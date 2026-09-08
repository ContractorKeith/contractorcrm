import { useEffect, useId, useRef, useState } from "react";

import "./RecordPicker.css";

export interface RecordOption {
  value: string;
  label: string;
  archived?: boolean;
}

interface RecordPickerProps {
  label: string;
  error?: string | undefined;
  value: string;
  options: RecordOption[];
  onChange: (value: string) => void;
}

// Keep the selected identity separate from search text: typing never relinks a record.
export function RecordPicker({ label, error, value, options, onChange }: RecordPickerProps) {
  const id = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const selected = options.find((option) => option.value === value);
  const terms = (query ?? "").trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const matches = options.filter((option) => !option.archived &&
    terms.every((term) => option.label.toLocaleLowerCase().includes(term)));
  const shown = matches.slice(0, 20);
  const activeIndex = Math.min(active, shown.length - 1);

  useEffect(() => {
    if (open) document.getElementById(`${id}-${activeIndex}`)?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex, id, open]);

  const close = () => { setOpen(false); setQuery(null); setActive(0); };
  const choose = (next: string) => { onChange(next); close(); inputRef.current?.focus(); };

  return (
    <div className="field">
      <label className="field__label" htmlFor={`${id}-input`}>{label}</label>
      <div className="record-picker">
        <div className="record-picker__input">
          <input id={`${id}-input`} ref={inputRef} role="combobox" aria-autocomplete="list"
            aria-expanded={open} aria-controls={open ? `${id}-list` : undefined}
            aria-activedescendant={open && activeIndex >= 0 ? `${id}-${activeIndex}` : undefined}
            aria-describedby={[open ? `${id}-hint` : "", error ? `${id}-error` : ""].filter(Boolean).join(" ") || undefined}
            aria-invalid={error ? true : undefined} autoComplete="off"
            placeholder="Search records…" value={query ?? selected?.label ?? (value ? "Linked record" : "")}
            onFocus={(event) => event.currentTarget.select()}
            onClick={() => setOpen(true)}
            onBlur={close}
            onChange={(event) => { setQuery(event.target.value); setOpen(true); setActive(0); }}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                setOpen(true);
                setActive(!open ? 0 : Math.max(0, Math.min(shown.length - 1,
                  activeIndex + (event.key === "ArrowDown" ? 1 : -1))));
              } else if (event.key === "Enter" && open) {
                event.preventDefault();
                if (shown[activeIndex]) choose(shown[activeIndex]!.value);
              } else if (event.key === "Escape" && open) {
                event.preventDefault();
                event.stopPropagation();
                close();
              }
            }} />
          {value ? <button type="button" className="button" aria-label={`Clear ${label}`}
            onClick={() => choose("")}>Clear</button> : null}
        </div>
        {open ? <div className="record-picker__popup" onMouseDown={(event) => event.preventDefault()}>
          <ul id={`${id}-list`} role="listbox" aria-label={`${label} matches`}>
            {shown.map((option, index) => <li key={option.value} id={`${id}-${index}`}
              role="option" aria-selected={index === activeIndex}
              onMouseMove={() => setActive(index)} onClick={(event) => { event.preventDefault(); choose(option.value); }}>
              {option.label}
            </li>)}
          </ul>
          <p id={`${id}-hint`} role="status">{matches.length === 0 ? "No matching records." :
            matches.length > shown.length ? `Showing ${shown.length} of ${matches.length}. Keep typing to narrow the list.` :
              `${matches.length} ${matches.length === 1 ? "match" : "matches"}.`}</p>
        </div> : null}
      </div>
      {error ? <span id={`${id}-error`} className="field__error" role="alert">{error}</span> : null}
    </div>
  );
}
