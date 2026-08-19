import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";

import type { CoreClient } from "../api/client";
import { isCommandError, type Attachment, type AttachmentParentType } from "../api/types";
import { formatLocalDateTime } from "../views/date-format";

interface RecordAttachmentsProps {
  client: CoreClient;
  parentType: AttachmentParentType;
  parentId: string;
}

// Managed files are copies, so the stored size is what the CRM holds on disk.
function formatSize(sizeBytes: number): string {
  const kb = sizeBytes / 1024;
  return kb < 1024 ? `${kb.toFixed(1)} KB` : `${(kb / 1024).toFixed(1)} MB`;
}

/** Files kept with a contact or opportunity: add, open, remove. */
export function RecordAttachments({ client, parentType, parentId }: RecordAttachmentsProps) {
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [confirmingId, setConfirmingId] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setAttachments(await client.listAttachments(parentType, parentId));
    } catch (rejection) {
      setError(
        isCommandError(rejection) ? rejection.message : "Attachments could not be read.",
      );
    }
  }, [client, parentType, parentId]);

  useEffect(() => {
    void load();
  }, [load]);

  // Native picker, then one managed copy per chosen file, in order.
  const addFiles = async () => {
    setError(null);
    try {
      const picked = await open({ multiple: true });
      const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
      for (const sourcePath of paths) {
        await client.addAttachment({ parentType, parentId, sourcePath });
      }
    } catch (rejection) {
      setError(isCommandError(rejection) ? rejection.message : "The file could not be attached.");
    }
    await load();
  };

  // The core owns the path; a missing managed file is reported, not opened.
  const openAttachment = async (attachment: Attachment) => {
    setError(null);
    try {
      const location = await client.attachmentPath(attachment.id);
      if (!location.exists) {
        setError(`${attachment.fileName} is missing from the attachments folder.`);
        return;
      }
      await openPath(location.path);
    } catch (rejection) {
      setError(isCommandError(rejection) ? rejection.message : "The file could not be opened.");
    }
  };

  // Two-step confirm inline; a version conflict just means reload and retry.
  const removeAttachment = async (attachment: Attachment) => {
    setError(null);
    setConfirmingId(null);
    try {
      await client.removeAttachment({
        attachmentId: attachment.id,
        expectedVersion: attachment.version,
      });
      await load();
    } catch (rejection) {
      if (isCommandError(rejection) && rejection.kind === "version_conflict") {
        await load();
        return;
      }
      setError(isCommandError(rejection) ? rejection.message : "The file could not be removed.");
    }
  };

  return (
    <section className="record-attachments" aria-label="Attachments">
      <div className="record-attachments__head">
        <h3 className="detail-subhead">Attachments</h3>
        <button type="button" className="button" onClick={() => void addFiles()}>
          Add file…
        </button>
      </div>

      {error ? (
        <p role="alert" className="form-error">
          {error}
        </p>
      ) : null}

      {attachments.length === 0 ? (
        <p className="detail-empty">No attachments yet.</p>
      ) : (
        <ul className="attachment-list" aria-label="Attached files">
          {attachments.map((attachment) => (
            <li key={attachment.id} className="attachment-row">
              <span className="attachment-name">{attachment.fileName}</span>
              <span className="attachment-meta">{formatSize(attachment.sizeBytes)}</span>
              <span className="attachment-meta">{formatLocalDateTime(attachment.createdAt)}</span>
              <div className="attachment-actions">
                <button
                  type="button"
                  className="button"
                  aria-label={`Open ${attachment.fileName}`}
                  onClick={() => void openAttachment(attachment)}
                >
                  Open
                </button>
                {confirmingId === attachment.id ? (
                  <>
                    <button
                      type="button"
                      className="button button--primary"
                      aria-label={`Confirm removing ${attachment.fileName}`}
                      onClick={() => void removeAttachment(attachment)}
                    >
                      Remove?
                    </button>
                    <button
                      type="button"
                      className="button"
                      aria-label={`Keep ${attachment.fileName}`}
                      onClick={() => setConfirmingId(null)}
                    >
                      Cancel
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    className="button"
                    aria-label={`Remove ${attachment.fileName}`}
                    onClick={() => setConfirmingId(attachment.id)}
                  >
                    Remove
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
