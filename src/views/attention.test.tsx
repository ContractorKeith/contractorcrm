import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { makeAttentionFlag, makeContact, stubClient } from "../test/stub-client";

// Open the Attention tab from the app shell.
async function openAttention(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Attention" }));
}

describe("needs-attention view", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("shows the all-clear empty state when there are no flags", async () => {
    const user = userEvent.setup();
    const client = stubClient();

    render(<App client={client} />);
    await openAttention(user);

    expect(await screen.findByText("Nothing needs attention.")).toBeVisible();
  });

  it("renders flags in the order returned with their explanations", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getAttentionFlags: vi.fn().mockResolvedValue([
        makeAttentionFlag({
          id: "f1",
          rule: "overdue_task",
          recordType: "task",
          recordDisplayName: "Call inspector",
          explanation: 'Task "Call inspector" was due 3 days ago.',
        }),
        makeAttentionFlag({
          id: "f2",
          rule: "stale_lead",
          recordType: "contact",
          recordId: "contact-1",
          recordDisplayName: "Dana Ruiz",
          explanation: "No contact with Dana Ruiz in 25 days.",
        }),
      ]),
    });

    render(<App client={client} />);
    await openAttention(user);

    const list = await screen.findByRole("list", { name: "Attention flags" });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(within(items[0]!).getByText('Task "Call inspector" was due 3 days ago.')).toBeVisible();
    expect(within(items[1]!).getByText("No contact with Dana Ruiz in 25 days.")).toBeVisible();
  });

  it("opens the flagged record's detail view from the link", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getAttentionFlags: vi.fn().mockResolvedValue([
        makeAttentionFlag({
          id: "f1",
          rule: "stale_lead",
          recordType: "contact",
          recordId: "contact-1",
          recordDisplayName: "Dana Ruiz",
          explanation: "No contact with Dana Ruiz in 25 days.",
        }),
      ]),
      getContact: vi.fn().mockResolvedValue(makeContact()),
    });

    render(<App client={client} />);
    await openAttention(user);

    await user.click(await screen.findByRole("button", { name: "Dana Ruiz" }));

    expect(client.getContact).toHaveBeenCalledWith("contact-1");
    expect(await screen.findByRole("heading", { name: /Dana Ruiz/ })).toBeVisible();
  });

  it("saves edited thresholds through the client", async () => {
    const user = userEvent.setup();
    const client = stubClient();

    render(<App client={client} />);
    await openAttention(user);

    const staleInput = await screen.findByLabelText("Stale lead after (days)");
    await user.clear(staleInput);
    await user.type(staleInput, "30");
    await user.click(screen.getByRole("button", { name: "Save thresholds" }));

    expect(client.setAttentionThresholds).toHaveBeenCalledWith({
      staleLeadDays: 30,
      proposalNoResponseDays: 7,
      proposalStageName: "Proposal Sent",
    });
  });

  it("shows a threshold validation error inline near the field", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      setAttentionThresholds: vi.fn().mockRejectedValue({
        kind: "invalid_input",
        message: "staleLeadDays: must be at least 1",
        field: "staleLeadDays",
      }),
    });

    render(<App client={client} />);
    await openAttention(user);

    const staleField = (await screen.findByLabelText("Stale lead after (days)")).closest("label")!;
    await user.click(screen.getByRole("button", { name: "Save thresholds" }));

    expect(await within(staleField).findByText("must be at least 1")).toBeVisible();
  });
});
