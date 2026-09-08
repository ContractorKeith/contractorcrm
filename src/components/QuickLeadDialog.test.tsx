import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { QuickLeadDialog } from "./QuickLeadDialog";
import { makeContact, makeOpportunity, stubClient } from "../test/stub-client";

describe("QuickLeadDialog", () => {
  it("captures the short form", async () => {
    const user = userEvent.setup();
    const onSaved = vi.fn();
    const client = stubClient({ captureLead: vi.fn().mockResolvedValue({ contact: makeContact(), opportunity: makeOpportunity(), task: null }) });
    render(<QuickLeadDialog client={client} onSaved={onSaved} onClose={vi.fn()} />);
    await user.type(screen.getByLabelText("Lead name"), "Dana Ruiz");
    await user.type(screen.getByLabelText("Job or request"), "Replace side gate");
    await user.click(screen.getByRole("button", { name: "Save lead" }));
    expect(client.captureLead).toHaveBeenCalledWith(expect.objectContaining({ name: "Dana Ruiz", jobRequest: "Replace side gate" }));
    expect(onSaved).toHaveBeenCalled();
  });

  it("keeps keyboard focus inside while saving", async () => {
    const user = userEvent.setup();
    let finish: ((value: unknown) => void) | undefined;
    const client = stubClient({ captureLead: vi.fn().mockReturnValue(new Promise((resolve) => { finish = resolve; })) });
    render(<QuickLeadDialog client={client} onSaved={vi.fn()} onClose={vi.fn()} />);
    await user.type(screen.getByLabelText("Lead name"), "Dana Ruiz");
    await user.type(screen.getByLabelText("Job or request"), "Replace side gate");
    await user.click(screen.getByRole("button", { name: "Save lead" }));
    await user.tab();
    expect(screen.getByRole("dialog")).toHaveFocus();
    finish?.({ contact: makeContact(), opportunity: makeOpportunity(), task: null });
  });
});
