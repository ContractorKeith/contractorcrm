import { useState } from "react";

import type { CoreClient } from "../api/client";
import { isCommandError, type ContextPreview, type PreviewContextRequest } from "../api/types";

interface ContextDisclosureProps {
  client: CoreClient;
  /** The call the user is about to make, described exactly as it will run. */
  request: PreviewContextRequest;
  /** Override for surfaces where nothing is sent unless the assistant is on. */
  label?: string;
  /**
   * What this disclosure is about — appended to the accessible name so a list
   * of otherwise identical "See what will be sent" toggles stays telling apart.
   */
  about?: string;
}

/**
 * "See what will be sent" — a collapsed disclosure that fetches the bounded
 * projection an AI-backed feature would send, before the user triggers it.
 * Nothing is sent to a provider to build this, so it works with the assistant
 * switched off too.
 */
export function ContextDisclosure({
  client,
  request,
  label = "See what will be sent",
  about,
}: ContextDisclosureProps) {
  const [preview, setPreview] = useState<ContextPreview | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Fetch on first open only; re-opening shows what was already fetched.
  const load = async () => {
    if (preview) return;
    try {
      setPreview(await client.previewContext(request));
      setError(null);
    } catch (rejection) {
      setError(
        isCommandError(rejection) ? rejection.message : "The preview could not be built.",
      );
    }
  };

  return (
    <details
      className="context-disclosure"
      onToggle={(event) => {
        if (event.currentTarget.open) void load();
      }}
    >
      <summary
        className="context-disclosure__summary"
        // Keeps the visible text inside the accessible name (WCAG 2.5.3).
        aria-label={about ? `${label} for ${about}` : undefined}
      >
        {label}
      </summary>
      {error ? (
        <p role="alert" className="attention-ai__error">
          {error}
        </p>
      ) : null}
      {preview ? (
        <>
          <p className="attention-ai__meta">
            {preview.includedRecordRefs.map((record) => record.label).join(", ") || "no records"}
          </p>
          {/* The preview box scrolls, so it needs to be a focusable named region
              or keyboard-only users cannot reach the clipped text (WCAG 2.1.1). */}
          <pre
            className="context-disclosure__text"
            tabIndex={0}
            role="region"
            aria-label={about ? `Context to be sent for ${about}` : "Context to be sent"}
          >
            {preview.contextText}
          </pre>
        </>
      ) : null}
    </details>
  );
}
