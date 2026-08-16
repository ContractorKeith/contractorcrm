import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { makeActivity, makeContact, stubClient } from "../test/stub-client";

// Open the seeded contact's detail view, where the timeline lives.
async function openContactDetail(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByText("Dana Ruiz"));
  await screen.findByRole("heading", { name: /Dana Ruiz/ });
}

function timelineClient(overrides: Parameters<typeof stubClient>[0] = {}) {
  return stubClient({
    listContacts: vi.fn().mockResolvedValue([makeContact()]),
    getContact: vi.fn().mockResolvedValue(makeContact()),
    ...overrides,
  });
}

describe("activity timeline", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders entries newest-first with kind, direction, and summary", async () => {
    const user = userEvent.setup();
    const client = timelineClient({
      getTimeline: vi.fn().mockResolvedValue([
        makeActivity({
          id: "a2",
          kind: "call",
          direction: "inbound",
          occurredAt: "2026-08-13T10:00:00Z",
          summary: "Homeowner called back",
        }),
        makeActivity({
          id: "a1",
          kind: "note",
          direction: "none",
          occurredAt: "2026-08-10T10:00:00Z",
          summary: "First walkthrough notes",
          body: "Backyard slopes toward the canal.",
        }),
      ]),
    });

    render(<App client={client} />);
    await openContactDetail(user);

    const list = await screen.findByRole("list", { name: "Activity entries" });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(within(items[0]!).getByText("Call · Inbound")).toBeVisible();
    expect(within(items[0]!).getByText("Homeowner called back")).toBeVisible();
    expect(within(items[0]!).getByText("2026-08-13T10:00:00Z")).toBeVisible();
    expect(within(items[1]!).getByText("Note")).toBeVisible();
    expect(within(items[1]!).getByText("Backyard slopes toward the canal.")).toBeVisible();
  });

  it("reloads with includeRelated when the opportunity-activity toggle flips", async () => {
    const user = userEvent.setup();
    const client = timelineClient();

    render(<App client={client} />);
    await openContactDetail(user);

    expect(client.getTimeline).toHaveBeenCalledWith("contact", "contact-1", false);

    await user.click(screen.getByLabelText("Include opportunity activity"));
    expect(client.getTimeline).toHaveBeenCalledWith("contact", "contact-1", true);
  });

  it("logs a note without a direction key so the core defaults it to none", async () => {
    const user = userEvent.setup();
    const client = timelineClient({
      logActivity: vi.fn().mockResolvedValue(makeActivity({ kind: "note" })),
    });

    render(<App client={client} />);
    await openContactDetail(user);

    // Kind defaults to Note; the direction select is not shown for notes.
    expect(screen.queryByLabelText("Direction")).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Summary"), "Left a voicemail follow-up note");
    await user.click(screen.getByRole("button", { name: "Log activity" }));

    expect(client.logActivity).toHaveBeenCalledWith(
      expect.objectContaining({
        parentType: "contact",
        parentId: "contact-1",
        kind: "note",
        summary: "Left a voicemail follow-up note",
        body: null,
      }),
    );
    const payload = vi.mocked(client.logActivity).mock.calls[0]![0];
    expect("direction" in payload).toBe(false);
  });

  it("includes the direction for communication kinds", async () => {
    const user = userEvent.setup();
    const client = timelineClient({
      logActivity: vi.fn().mockResolvedValue(makeActivity()),
    });

    render(<App client={client} />);
    await openContactDetail(user);

    await user.selectOptions(screen.getByLabelText("Type"), "call");
    await user.selectOptions(screen.getByLabelText("Direction"), "inbound");
    await user.type(screen.getByLabelText("Summary"), "Talked through gate options");
    await user.click(screen.getByRole("button", { name: "Log activity" }));

    expect(client.logActivity).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "call", direction: "inbound" }),
    );
  });

  it.each([
    ["call", "Call", true],
    ["email", "Email", true],
    ["text", "Text", true],
    ["site_visit", "Site visit", false],
    ["meeting", "Meeting", false],
    ["note", "Note", false],
  ] as const)("persists and renders the %s activity kind", async (kind, label, hasDirection) => {
    const user = userEvent.setup();
    const client = timelineClient({
      logActivity: vi.fn().mockResolvedValue(makeActivity({ kind })),
      getTimeline: vi.fn().mockResolvedValue([makeActivity({ kind, direction: "none" })]),
    });

    render(<App client={client} />);
    await openContactDetail(user);

    await user.selectOptions(screen.getByLabelText("Type"), kind);
    if (hasDirection) {
      expect(screen.getByLabelText("Direction")).toBeVisible();
    } else {
      expect(screen.queryByLabelText("Direction")).not.toBeInTheDocument();
    }
    await user.type(screen.getByLabelText("Summary"), `${label} follow-up`);
    await user.click(screen.getByRole("button", { name: "Log activity" }));

    expect(client.logActivity).toHaveBeenCalledWith(expect.objectContaining({ kind }));
    const list = await screen.findByRole("list", { name: "Activity entries" });
    expect(within(list).getByText(label)).toBeVisible();
  });

  it("requires a summary before anything reaches the core", async () => {
    const user = userEvent.setup();
    const client = timelineClient();

    render(<App client={client} />);
    await openContactDetail(user);

    await user.click(screen.getByRole("button", { name: "Log activity" }));

    expect(await screen.findByText("Enter a short summary.")).toBeVisible();
    expect(client.logActivity).not.toHaveBeenCalled();
  });

  it("edits an entry through update_activity with the expected version", async () => {
    const user = userEvent.setup();
    const activity = makeActivity({ id: "a1", version: 3, summary: "Old summary" });
    const client = timelineClient({
      getTimeline: vi.fn().mockResolvedValue([activity]),
      updateActivity: vi.fn().mockResolvedValue({ ...activity, version: 4 }),
    });

    render(<App client={client} />);
    await openContactDetail(user);

    const list = await screen.findByRole("list", { name: "Activity entries" });
    await user.click(within(list).getByRole("button", { name: "Edit" }));

    const summary = screen.getAllByLabelText("Summary")[1]!; // entry form, not log form
    await user.clear(summary);
    await user.type(summary, "Corrected summary");
    await user.click(screen.getByRole("button", { name: "Save entry" }));

    expect(client.updateActivity).toHaveBeenCalledWith(
      expect.objectContaining({
        activityId: "a1",
        expectedVersion: 3,
        patch: expect.objectContaining({ summary: "Corrected summary" }),
      }),
    );
  });

  it("deletes an entry only after the user confirms", async () => {
    const user = userEvent.setup();
    const activity = makeActivity({ id: "a1", version: 2 });
    const client = timelineClient({
      getTimeline: vi.fn().mockResolvedValue([activity]),
      deleteActivity: vi.fn().mockResolvedValue(undefined),
    });
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);

    render(<App client={client} />);
    await openContactDetail(user);

    const list = await screen.findByRole("list", { name: "Activity entries" });
    await user.click(within(list).getByRole("button", { name: "Delete" }));
    expect(client.deleteActivity).not.toHaveBeenCalled();

    confirmSpy.mockReturnValue(true);
    await user.click(within(list).getByRole("button", { name: "Delete" }));
    expect(client.deleteActivity).toHaveBeenCalledWith({ activityId: "a1", expectedVersion: 2 });
  });
});
