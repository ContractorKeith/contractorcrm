import { useEffect, useRef, useState } from "react";

import type { CoreClient } from "../api/client";
import { isCommandError, type CapturedLead, type SearchResult } from "../api/types";
import "./QuickLeadDialog.css";

type Props = {
  client: CoreClient;
  onSaved: (lead: CapturedLead) => void;
  onClose: () => void;
};

/** Focused intake for an inquiry; full record editors remain the place for detail work. */
export function QuickLeadDialog({ client, onSaved, onClose }: Props) {
  const [name, setName] = useState("");
  const [jobRequest, setJobRequest] = useState("");
  const [phone, setPhone] = useState("");
  const [email, setEmail] = useState("");
  const [note, setNote] = useState("");
  const [more, setMore] = useState(false);
  const [nextStepTitle, setNextStepTitle] = useState("");
  const [nextStepDueAt, setNextStepDueAt] = useState("");
  const [matches, setMatches] = useState<SearchResult[]>([]);
  const [contactId, setContactId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLFormElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const savedRef = useRef(false);

  useEffect(() => {
    restoreFocusRef.current = document.activeElement as HTMLElement | null;
    nameRef.current?.focus();
    return () => {
      if (!savedRef.current) {
        const invoker = restoreFocusRef.current;
        requestAnimationFrame(() => invoker?.focus());
      }
    };
  }, []);

  useEffect(() => {
    if (saving) dialogRef.current?.focus();
  }, [saving]);

  useEffect(() => {
    const query = name.trim();
    if (query.length < 2 || contactId) { setMatches([]); return; }
    let current = true;
    void client.searchRecords(query, ["contact"], 5).then(
      (results) => { if (current) setMatches(results); },
      () => { if (current) setMatches([]); },
    );
    return () => { current = false; };
  }, [client, name, contactId]);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (saving) return;
    setSaving(true); setError(null);
    try {
      const lead = await client.captureLead({
        name, jobRequest, phone: phone || null, email: email || null, note: note || null,
        contactId, nextStepTitle: nextStepTitle || null,
        nextStepDueAt: nextStepDueAt ? new Date(nextStepDueAt).toISOString() : null,
      });
      savedRef.current = true;
      onSaved(lead);
    } catch (reason) {
      setError(isCommandError(reason) ? reason.message : "Could not save this lead. Try again.");
    } finally { setSaving(false); }
  }

  function trapFocus(event: React.KeyboardEvent) {
    if (event.key === "Escape") { event.preventDefault(); if (!saving) onClose(); return; }
    if (event.key !== "Tab") return;
    if (saving) { event.preventDefault(); dialogRef.current?.focus(); return; }
    const nodes = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>("button:not(:disabled),input:not(:disabled),textarea:not(:disabled)") ?? []);
    const first = nodes[0], last = nodes.at(-1);
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }

  return <div className="quick-lead-backdrop" onMouseDown={(e) => e.target === e.currentTarget && !saving && onClose()}>
    <form ref={dialogRef} tabIndex={-1} className="quick-lead-dialog" onSubmit={submit} onKeyDown={trapFocus} role="dialog" aria-modal="true" aria-label="New lead" aria-describedby={error ? "quick-lead-error" : undefined}>
      <fieldset disabled={saving} className="quick-lead-fields"><div className="quick-lead-heading"><h2>New lead</h2><button type="button" className="button" onClick={onClose}>Cancel</button></div>
      <label className="field field__label">Lead name<input ref={nameRef} required value={name} onChange={(e) => { setName(e.target.value); setContactId(null); }} /></label>
      {matches.length > 0 && <div className="quick-lead-matches" aria-label="Existing contacts">
        <span>Existing contacts</span>{matches.map((match) => <button type="button" key={match.entityId} onClick={() => { setContactId(match.entityId); setName(match.title); setPhone(""); setEmail(""); setMatches([]); }}>Use {match.title}</button>)}
      </div>}
      {contactId && <p className="quick-lead-linked">Using an existing contact; their details will not change. <button type="button" onClick={() => setContactId(null)}>Choose another</button></p>}
      <label className="field field__label">Job or request<textarea required value={jobRequest} onChange={(e) => setJobRequest(e.target.value)} rows={3} /></label>
      {!contactId && <div className="quick-lead-channels"><label className="field field__label">Phone<input type="tel" value={phone} onChange={(e) => setPhone(e.target.value)} /></label><label className="field field__label">Email<input type="email" value={email} onChange={(e) => setEmail(e.target.value)} /></label></div>}
      <label className="field field__label">Note<textarea value={note} onChange={(e) => setNote(e.target.value)} rows={2} /></label>
      <button type="button" className="quick-lead-details" onClick={() => setMore(!more)} aria-expanded={more}> {more ? "Hide next step" : "Add a next step"}</button>
      {more && <div className="quick-lead-next"><label className="field field__label">Next step<input value={nextStepTitle} onChange={(e) => setNextStepTitle(e.target.value)} /></label><label className="field field__label">Due<input type="datetime-local" value={nextStepDueAt} onChange={(e) => setNextStepDueAt(e.target.value)} /></label></div>}
      {error && <p id="quick-lead-error" role="alert">{error}</p>}
      <button className="button button--primary">{saving ? "Saving…" : "Save lead"}</button></fieldset>
    </form>
  </div>;
}
