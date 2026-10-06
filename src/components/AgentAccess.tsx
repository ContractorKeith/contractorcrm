import { useEffect, useRef, useState } from "react";

import type { CoreClient } from "../api/client";
import { Field } from "../views/form-support";

interface AgentPaths {
  helper: string;
  database: string;
}

// Commands are a reference for a shell; MCP clients use the structured JSON.
function shellCommand(paths: AgentPaths, readWrite: boolean): string {
  const windows = /^[a-z]:[\\/]|^\\\\/i.test(paths.helper);
  const quote = (value: string) =>
    windows ? `'${value.replaceAll("'", "''")}'` : `'${value.replaceAll("'", "'\"'\"'")}'`;
  return `${windows ? "& " : ""}${quote(paths.helper)} --database ${quote(paths.database)}${readWrite ? "" : " --read-only"}`;
}

/** Connect the user's own agent without enabling the built-in assistant. */
export function AgentAccess({ client }: { client: CoreClient }) {
  const [paths, setPaths] = useState<AgentPaths | null>(null);
  const [readWrite, setReadWrite] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copyStatus, setCopyStatus] = useState("");
  const [attempt, setAttempt] = useState(0);
  const configRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    let active = true;
    setPaths(null);
    setError(null);
    setCopyStatus("");
    void Promise.all([client.getAgentHelperPath(), client.getDatabaseInfo()])
      .then(([helper, info]) => {
        if (!helper.trim() || !info.databasePath.trim()) throw new Error("Missing paths");
        if (active) setPaths({ helper, database: info.databasePath });
      })
      .catch(() => {
        if (active) setError("Could not find the agent helper and database. Open the installed app and try again.");
      });
    return () => { active = false; };
  }, [client, attempt]);

  const config = paths
    ? JSON.stringify({
        mcpServers: {
          contractorcrm: {
            command: paths.helper,
            args: ["--database", paths.database, ...(readWrite ? [] : ["--read-only"])],
          },
        },
      }, null, 2)
    : "";

  const copy = async () => {
    if (!config) return;
    try {
      await navigator.clipboard.writeText(config);
      setCopyStatus("Configuration copied.");
    } catch {
      configRef.current?.focus();
      configRef.current?.select();
      setCopyStatus("Copy was unavailable. The configuration is selected; copy it with your keyboard.");
    }
  };

  return (
    <section className="data-section" aria-labelledby="agent-access-heading">
      <h3 id="agent-access-heading">Connect your agent</h3>
      <p>Use your own MCP-capable agent with this CRM. No CRM AI setup or API key required.</p>
      <Field label="Agent access">
        <select value={readWrite ? "write" : "read"} onChange={(event) => {
          setReadWrite(event.target.value === "write");
          setCopyStatus("");
        }}>
          <option value="write">Read and write</option>
          <option value="read">Read-only</option>
        </select>
      </Field>
      <p>{readWrite
        ? "Your agent can create leads and update follow-ups. Changes are recorded in the audit log; only applied AI proposals have undo."
        : "Your agent can read records and prepare follow-ups. It cannot change your CRM."}</p>
      <Field label="MCP configuration (JSON)">
        <textarea ref={configRef} readOnly rows={12} spellCheck={false} value={config}
          placeholder={error ? "Configuration unavailable" : "Finding this device’s paths…"} />
      </Field>
      <div className="data-section__actions">
        <button type="button" className="button button--primary" disabled={!paths}
          onClick={() => void copy()}>Copy configuration</button>
        {error ? <button type="button" className="button"
          onClick={() => setAttempt((value) => value + 1)}>Try again</button> : null}
      </div>
      <p className="data-section__result" role="status">{copyStatus}</p>
      {error ? <p role="alert">{error}</p> : null}
      <p>Paste this into a client that accepts <code>mcpServers</code> JSON, adding this entry to any existing servers. Restart the client to connect or change access.</p>
      <details>
        <summary>Connection details</summary>
        <p>The helper runs locally over stdio. Your agent controls where it sends the records it reads. File imports and backups stay in the app.</p>
        <Field label={paths && /^[a-z]:[\\/]|^\\\\/i.test(paths.helper) ? "PowerShell command" : "Shell command"}>
          <input readOnly value={paths ? shellCommand(paths, readWrite) : ""} />
        </Field>
        <p>Other clients may use a different config format: use the same command and argument list shown above.</p>
      </details>
    </section>
  );
}
