import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { makeContact, makeTask, stubClient } from "../test/stub-client";

describe("today view", () => {
  it("is the default view, uses an offset-bearing reference, and completes in place", async () => {
    const user = userEvent.setup();
    const task = makeTask({ id: "task-1", version: 4, title: "Call Dana", dueAt: "2026-09-08T16:00:00-04:00" });
    const getWorkQueue = vi
      .fn()
      .mockResolvedValueOnce({
        referenceTime: "2026-09-08T08:00:00-04:00",
        localDate: "2026-09-08",
        truncated: false,
        items: [{ kind: "task", task, reason: "due_today", linkedRecord: { recordType: "contact", recordId: "contact-1", displayName: "Dana Ruiz" } }],
      })
      .mockResolvedValueOnce({ referenceTime: "2026-09-08T08:00:00-04:00", localDate: "2026-09-08", truncated: false, items: [] });
    const completeTask = vi.fn().mockResolvedValue({ ...task, status: "done", version: 5 });
    render(<App client={stubClient({ getWorkQueue, completeTask })} />);

    expect(await screen.findByRole("heading", { name: "Today" })).toBeVisible();
    expect(screen.getByText("Call Dana")).toBeVisible();
    expect(getWorkQueue.mock.calls[0]?.[0]).toMatch(/[+-]\d{2}:\d{2}$/);
    await user.click(screen.getByRole("button", { name: "Complete" }));
    expect(completeTask).toHaveBeenCalledWith({ taskId: "task-1", expectedVersion: 4 });
    expect(await screen.findByText("No follow-ups need work today.")).toBeVisible();
  });

  it("opens a linked record from the queue", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getWorkQueue: vi.fn().mockResolvedValue({
        referenceTime: "2026-09-08T08:00:00-04:00", localDate: "2026-09-08", truncated: false,
        items: [{ kind: "attention", rule: "stale_lead", recordType: "contact", recordId: "contact-1", recordDisplayName: "Dana Ruiz", explanation: "No contact in 21 days." }],
      }),
      getContact: vi.fn().mockResolvedValue(makeContact()),
    });
    render(<App client={client} />);
    await user.click(await screen.findByRole("button", { name: "Dana Ruiz" }));
    await waitFor(() => expect(client.getContact).toHaveBeenCalledWith("contact-1"));
  });

  it("shows a visible reload path for conflicts and can retry a failed load", async () => {
    const user = userEvent.setup();
    const task = makeTask({ id: "task-1", version: 4, title: "Call Dana" });
    const getWorkQueue = vi
      .fn()
      .mockRejectedValueOnce(new Error("locked"))
      .mockResolvedValue({ referenceTime: "2026-09-08T08:00:00-04:00", localDate: "2026-09-08", truncated: false, items: [{ kind: "task", task, reason: "due_today", linkedRecord: null }] });
    render(<App client={stubClient({ getWorkQueue, completeTask: vi.fn().mockRejectedValue({ kind: "version_conflict", message: "stale", resource: "task", recordId: "task-1", expectedVersion: 4, currentVersion: 5 }) })} />);
    await user.click(await screen.findByRole("button", { name: "Try again" }));
    await user.click(await screen.findByRole("button", { name: "Complete" }));
    expect(await screen.findByRole("button", { name: "Reload latest" })).toBeVisible();
  });
});
