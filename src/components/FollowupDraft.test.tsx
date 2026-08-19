import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  makeFollowupDraft,
  makeFollowupTemplates,
  makeHistorySummary,
  stubClient,
} from "../test/stub-client";
import { FollowupDraftPanel, HistorySummaryPanel } from "./FollowupDraft";

const aiOn = { enabled: true } as const;

function clientWith(overrides: Parameters<typeof stubClient>[0] = {}, enabled = true) {
  return stubClient({
    getAiSettings: vi.fn().mockResolvedValue({
      version: 1,
      enabled,
      providerLabel: "Local model",
      baseUrl: "http://127.0.0.1:11434/v1",
      model: "llama3.1",
      hasApiKey: false,
    }),
    ...overrides,
  });
}

describe("HistorySummaryPanel", () => {
  it("summarizes only when asked and discloses the model and records included", async () => {
    const user = userEvent.setup();
    const client = clientWith({
      summarizeHistory: vi.fn().mockResolvedValue(makeHistorySummary()),
    });

    render(<HistorySummaryPanel client={client} parentType="contact" parentId="contact-1" />);

    const summarize = await screen.findByRole("button", { name: "Summarize" });
    expect(client.summarizeHistory).not.toHaveBeenCalled();
    await user.click(summarize);

    expect(client.summarizeHistory).toHaveBeenCalledWith("contact", "contact-1");
    expect(
      await screen.findByText("Dana asked for a gate quote in June and has gone quiet since."),
    ).toBeVisible();
    const actions = screen.getByRole("list", { name: "Suggested next actions" });
    expect(within(actions).getAllByRole("listitem")).toHaveLength(2);
    expect(
      screen.getByText("llama3.1 · Dana Ruiz stayed on this machine (127.0.0.1:11434)"),
    ).toBeVisible();
  });

  it("stays hidden while the assistant is off", async () => {
    const client = clientWith({}, false);
    render(<HistorySummaryPanel client={client} parentType="contact" parentId="contact-1" />);

    await vi.waitFor(() => expect(client.getAiSettings).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: "Summarize" })).toBeNull();
  });

  it("shows a plain failure message when the provider cannot be reached", async () => {
    const user = userEvent.setup();
    const client = clientWith({
      summarizeHistory: vi.fn().mockRejectedValue({
        kind: "provider_unavailable",
        message: "Couldn't reach 127.0.0.1:11434.",
        reason: "Couldn't reach 127.0.0.1:11434.",
      }),
    });

    render(<HistorySummaryPanel client={client} parentType="contact" parentId="contact-1" />);
    await user.click(await screen.findByRole("button", { name: "Summarize" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't reach 127.0.0.1:11434.");
  });
});

describe("FollowupDraftPanel", () => {
  it("drafts from the chosen template and files the task through the proposal dialog", async () => {
    const user = userEvent.setup();
    const client = clientWith({
      getFollowupTemplates: vi.fn().mockResolvedValue(makeFollowupTemplates()),
      proposeFollowup: vi.fn().mockResolvedValue(makeFollowupDraft()),
      applyProposal: vi.fn().mockResolvedValue({
        entityType: "task",
        entityId: "task-1",
        created: true,
        version: 1,
        undoToken: "undo-1",
        undoExpiresAt: "2026-08-19T12:15:00.000Z",
      }),
    });
    const onApplied = vi.fn();

    render(
      <FollowupDraftPanel
        client={client}
        parentType="contact"
        parentId="contact-1"
        onApplied={onApplied}
      />,
    );

    await screen.findByRole("heading", { name: "Draft follow-up" });
    await user.selectOptions(screen.getByRole("combobox"), "proposal_chaser");
    await user.type(screen.getByRole("textbox"), "chase the proposal");
    await user.click(screen.getByRole("button", { name: "Draft follow-up" }));

    expect(client.proposeFollowup).toHaveBeenCalledWith(
      "contact",
      "contact-1",
      "chase the proposal",
      "proposal_chaser",
    );
    expect(await screen.findByText("Checking in on the proposal.")).toBeVisible();
    expect(
      screen.getByText("llama3.1 · Dana Ruiz stayed on this machine (127.0.0.1:11434)"),
    ).toBeVisible();
    // Nothing is written by drafting — the task is created from the dialog.
    expect(client.applyProposal).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Review follow-up task" }));
    const dialog = await screen.findByRole("dialog", { name: "Review the assistant's draft" });
    expect(within(dialog).getByText("Follow up with Dana Ruiz")).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Apply" }));
    expect(client.applyProposal).toHaveBeenCalledWith({
      proposalId: "proposal-1",
      expectedVersions: [],
    });

    await user.click(await screen.findByRole("button", { name: "Done" }));
    expect(onApplied).toHaveBeenCalled();
  });

  it("works template-only when the assistant is off", async () => {
    const user = userEvent.setup();
    const client = clientWith(
      {
        getFollowupTemplates: vi.fn().mockResolvedValue(makeFollowupTemplates()),
        proposeFollowup: vi.fn().mockResolvedValue(
          makeFollowupDraft({
            usedProvider: false,
            model: null,
            endpointHost: null,
            local: false,
            includedRecordRefs: [],
            templateId: "call_followup",
            templateName: "Call follow-up",
            draftText: "Thanks for taking the time on the phone.",
          }),
        ),
      },
      false,
    );

    render(<FollowupDraftPanel client={client} parentType="contact" parentId="contact-1" />);

    // No dead end: the affordance stays, it just offers the template as written.
    await screen.findByRole("heading", { name: "Use a template" });
    await user.click(screen.getByRole("button", { name: "Use a template" }));

    expect(client.proposeFollowup).toHaveBeenCalledWith(
      "contact",
      "contact-1",
      undefined,
      undefined,
    );
    expect(await screen.findByText("Thanks for taking the time on the phone.")).toBeVisible();
    expect(
      screen.getByText("Template used as written — nothing was sent anywhere."),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Review follow-up task" })).toBeVisible();
  });

  it("reports a drafting failure without losing the panel", async () => {
    const user = userEvent.setup();
    const client = clientWith({
      getFollowupTemplates: vi.fn().mockResolvedValue(makeFollowupTemplates()),
      proposeFollowup: vi.fn().mockRejectedValue({
        kind: "provider_unavailable",
        message: "Couldn't reach 127.0.0.1:11434.",
        reason: "Couldn't reach 127.0.0.1:11434.",
      }),
    });

    render(<FollowupDraftPanel client={client} parentType="contact" parentId="contact-1" />);
    await user.click(await screen.findByRole("button", { name: "Draft follow-up" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't reach 127.0.0.1:11434.");
    expect(screen.getByRole("button", { name: "Draft follow-up" })).toBeVisible();
  });
});

// The AI-enabled flag is read once per panel; keep the shape honest.
it("reads the assistant setting from the core, not from local state", async () => {
  const client = clientWith({}, aiOn.enabled);
  render(<HistorySummaryPanel client={client} parentType="opportunity" parentId="opp-1" />);
  await screen.findByRole("button", { name: "Summarize" });
  expect(client.getAiSettings).toHaveBeenCalled();
});
