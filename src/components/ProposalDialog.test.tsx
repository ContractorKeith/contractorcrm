import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Proposal } from "../api/types";
import { stubClient } from "../test/stub-client";
import { AssistantPrompt, ProposalDialog } from "./ProposalDialog";

const updateProposal = (overrides: Partial<Proposal> = {}): Proposal => ({
  id: "proposal-1",
  kind: "update_opportunity",
  entityType: "opportunity",
  entityId: "opp-1",
  summary: 'Update opportunity "Backyard fence"',
  changes: [
    { field: "valueMinor", before: "100000", after: "450000" },
    { field: "notes", before: null, after: "Ready to sign" },
  ],
  warnings: [],
  affectedVersions: [{ entityType: "opportunity", entityId: "opp-1", version: 3 }],
  createdAt: "2026-08-19T12:00:00.000Z",
  expiresAt: "2026-08-19T12:15:00.000Z",
  ...overrides,
});

const applied = {
  entityType: "opportunity" as const,
  entityId: "opp-1",
  created: false,
  version: 4,
  undoToken: "undo-1",
  undoExpiresAt: "2026-08-19T12:15:00.000Z",
};

describe("ProposalDialog", () => {
  it("shows the field-level diff in contractor-facing terms", async () => {
    render(
      <ProposalDialog client={stubClient()} proposal={updateProposal()} onClose={vi.fn()} />,
    );

    const dialog = await screen.findByRole("dialog", { name: "Review the assistant's draft" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(screen.getByText('Update opportunity "Backyard fence"')).toBeVisible();
    const changes = screen.getByRole("table", { name: /Proposed changes/ });
    const value = within(changes).getByRole("row", { name: /Value/ });
    expect(within(value).getByText("1000.00")).toBeVisible();
    expect(within(value).getByText("4500.00")).toBeVisible();
    // A field with no stored value reads as empty, not as "null".
    expect(within(changes).getByRole("row", { name: /Notes/ })).toHaveTextContent("—");
  });

  it("lists warnings and never writes until Apply is pressed", async () => {
    const user = userEvent.setup();
    const client = stubClient({ applyProposal: vi.fn().mockResolvedValue(applied) });
    render(
      <ProposalDialog
        client={client}
        proposal={updateProposal({ warnings: ["No company named \"Nowhere Inc\" is on file."] })}
        onClose={vi.fn()}
      />,
    );

    expect(
      within(screen.getByRole("list", { name: "Draft warnings" })).getByText(
        'No company named "Nowhere Inc" is on file.',
      ),
    ).toBeVisible();
    expect(client.applyProposal).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(client.applyProposal).toHaveBeenCalledWith({
      proposalId: "proposal-1",
      expectedVersions: [{ entityType: "opportunity", entityId: "opp-1", version: 3 }],
    });
    expect(await screen.findByText("Draft applied — the record was updated.")).toBeVisible();
  });

  it("discards without applying and reports back that nothing changed", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const client = stubClient();
    render(<ProposalDialog client={client} proposal={updateProposal()} onClose={onClose} />);

    await user.click(screen.getByRole("button", { name: "Discard" }));

    expect(client.applyProposal).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledWith(false);
  });

  it("offers an undo after applying and confirms what it did", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      applyProposal: vi.fn().mockResolvedValue({ ...applied, created: true }),
      undoProposal: vi.fn().mockResolvedValue({
        entityType: "opportunity",
        entityId: "opp-1",
        action: "archived",
        version: 5,
      }),
    });
    render(
      <ProposalDialog
        client={client}
        proposal={updateProposal({ kind: "create_opportunity", entityId: null })}
        onClose={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Apply" }));
    expect(await screen.findByText("Draft applied — the record was created.")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Undo" }));

    expect(client.undoProposal).toHaveBeenCalledWith({
      undoToken: "undo-1",
      expectedVersions: [{ entityType: "opportunity", entityId: "opp-1", version: 4 }],
    });
    expect(await screen.findByText("Undone — the new record was archived.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
  });

  it("explains a version conflict in plain language", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      applyProposal: vi.fn().mockRejectedValue({
        kind: "version_conflict",
        message: "opportunity opp-1 changed: expected version 3, current version 4",
        resource: "opportunity",
        recordId: "opp-1",
        expectedVersion: 3,
        currentVersion: 4,
      }),
    });
    render(<ProposalDialog client={client} proposal={updateProposal()} onClose={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(
      await screen.findByText("This record changed since the draft was made — ask the assistant again."),
    ).toBeVisible();
  });

  it("explains an expired draft in plain language", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      applyProposal: vi.fn().mockRejectedValue({
        kind: "proposal_expired",
        message: "that draft is no longer available; ask the assistant again",
        proposalId: "proposal-1",
      }),
    });
    render(<ProposalDialog client={client} proposal={updateProposal()} onClose={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(await screen.findByText("This draft expired — ask the assistant again.")).toBeVisible();
  });

  it("cannot apply a draft with nothing in it", () => {
    render(
      <ProposalDialog
        client={stubClient()}
        proposal={updateProposal({ changes: [], warnings: ["Nothing in this record needed to change."] })}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    expect(screen.getByText(/didn't find anything to change/)).toBeVisible();
  });
});

describe("AssistantPrompt", () => {
  const enabledSettings = {
    version: 1,
    enabled: true,
    providerLabel: "Local model",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "llama3.1",
    hasApiKey: false,
  };

  it("stays hidden while the assistant is switched off", async () => {
    const client = stubClient();
    render(
      <AssistantPrompt
        client={client}
        entityType="contact"
        label="Ask the assistant"
        placeholder="Describe the new contact…"
      />,
    );

    expect(client.getAiSettings).toHaveBeenCalled();
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByRole("button", { name: "Ask" })).toBeNull();
  });

  it("drafts a new record and opens the review dialog", async () => {
    const user = userEvent.setup();
    const proposal = updateProposal({
      kind: "create_contact",
      entityType: "contact",
      entityId: null,
      summary: 'Create contact "Dana Ruiz"',
      affectedVersions: [],
    });
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(enabledSettings),
      proposeRecord: vi.fn().mockResolvedValue(proposal),
    });
    render(
      <AssistantPrompt
        client={client}
        entityType="contact"
        label="Ask the assistant"
        placeholder="Describe the new contact…"
      />,
    );

    const input = await screen.findByPlaceholderText("Describe the new contact…");
    await user.type(input, "New client Dana Ruiz");
    await user.click(screen.getByRole("button", { name: "Ask" }));

    expect(client.proposeRecord).toHaveBeenCalledWith("contact", "New client Dana Ruiz");
    expect(await screen.findByRole("dialog", { name: "Review the assistant's draft" })).toBeVisible();
  });

  it("drafts an update against the record's current version and reloads after applying", async () => {
    const user = userEvent.setup();
    const onApplied = vi.fn();
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(enabledSettings),
      proposeUpdate: vi.fn().mockResolvedValue(updateProposal()),
      applyProposal: vi.fn().mockResolvedValue(applied),
    });
    render(
      <AssistantPrompt
        client={client}
        entityType="opportunity"
        target={{ entityId: "opp-1", expectedVersion: 3 }}
        label="Ask the assistant"
        placeholder="Describe the change…"
        onApplied={onApplied}
      />,
    );

    await user.type(await screen.findByPlaceholderText("Describe the change…"), "Bump to $4,500");
    await user.click(screen.getByRole("button", { name: "Ask" }));
    expect(client.proposeUpdate).toHaveBeenCalledWith("opportunity", "opp-1", "Bump to $4,500", 3);

    await user.click(await screen.findByRole("button", { name: "Apply" }));
    await user.click(await screen.findByRole("button", { name: "Done" }));

    expect(onApplied).toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("surfaces a provider failure instead of pretending it drafted something", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue(enabledSettings),
      proposeRecord: vi.fn().mockRejectedValue({
        kind: "provider_unavailable",
        message: "Couldn't reach 127.0.0.1:11434.",
        reason: "Couldn't reach 127.0.0.1:11434.",
      }),
    });
    render(
      <AssistantPrompt
        client={client}
        entityType="contact"
        label="Ask the assistant"
        placeholder="Describe the new contact…"
      />,
    );

    await user.type(await screen.findByPlaceholderText("Describe the new contact…"), "New client");
    await user.click(screen.getByRole("button", { name: "Ask" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't reach 127.0.0.1:11434.");
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
