import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { makeContact, makeOpportunity, makeOpportunityDetail, stubClient } from "./test/stub-client";

describe("crm shell", () => {
  beforeEach(() => {
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
  });

  it("renders the themed shell with the empty state and core health", async () => {
    render(<App client={stubClient()} />);

    expect(screen.getByRole("link", { name: "ContractorCRM home" })).toBeVisible();
    expect(await screen.findByRole("heading", { name: "No follow-ups need work today." })).toBeVisible();
    expect(await screen.findByText("On this device")).toBeVisible();
  });

  it("captures an inquiry from Today and opens its saved opportunity", async () => {
    const user = userEvent.setup();
    const contact = makeContact({ displayName: "Avery Cole" });
    const opportunity = makeOpportunity({ name: "Kitchen repair", contactId: contact.id });
    const client = stubClient({
      captureLead: vi.fn().mockResolvedValue({ contact, opportunity, task: null }),
      getOpportunity: vi.fn().mockResolvedValue(makeOpportunityDetail({ ...opportunity })),
      getContact: vi.fn().mockResolvedValue(contact),
    });
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "New lead" }));
    expect(screen.getByRole("dialog", { name: "New lead" })).toBeVisible();
    expect(screen.getByLabelText("Lead name")).toHaveFocus();
    await user.type(screen.getByLabelText("Lead name"), "Avery Cole");
    await user.type(screen.getByLabelText("Job or request"), "Kitchen repair");
    await user.click(screen.getByRole("button", { name: "Save lead" }));
    expect(client.captureLead).toHaveBeenCalledWith(expect.objectContaining({
      name: "Avery Cole", jobRequest: "Kitchen repair",
    }));
    expect(await screen.findByRole("heading", { name: "Kitchen repair" })).toBeVisible();
    expect(screen.queryByRole("dialog", { name: "New lead" })).toBeNull();
    expect(screen.getByRole("main")).toHaveFocus();
  });

  it("lets the user override the system theme and persists the choice", async () => {
    const user = userEvent.setup();
    render(<App client={stubClient()} />);

    await user.selectOptions(screen.getByLabelText("Theme"), "dark");

    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(window.localStorage.getItem("contractorcrm.theme")).toBe("dark");

    await user.selectOptions(screen.getByLabelText("Theme"), "light");

    expect(document.documentElement).toHaveAttribute("data-theme", "light");
    expect(window.localStorage.getItem("contractorcrm.theme")).toBe("light");
  });

  it("switches between the contacts and companies sections", async () => {
    const user = userEvent.setup();
    render(<App client={stubClient()} />);

    await user.click(screen.getByRole("button", { name: "Companies" }));
    expect(await screen.findByRole("heading", { name: "No companies yet" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Contacts" }));
    expect(await screen.findByRole("heading", { name: "No contacts yet" })).toBeVisible();
  });

  it("routes activity search results to their parent and records the parent after navigation", async () => {
    const user = userEvent.setup();
    const parent = makeContact({ id: "contact-parent", displayName: "Avery Cole" });
    const recordRecent = vi.fn().mockResolvedValue(undefined);
    const client = stubClient({
      searchRecords: vi.fn().mockResolvedValue([
        {
          entityType: "activity",
          entityId: "activity-1",
          title: "Called Avery about the estimate",
          parentType: "contact",
          parentId: parent.id,
        },
      ]),
      getContact: vi.fn().mockResolvedValue(parent),
      recordRecent,
    });
    render(<App client={client} />);

    await user.click(screen.getByRole("button", { name: /Search/ }));
    fireEvent.change(screen.getByRole("combobox", { name: "Search records" }), {
      target: { value: "estimate" },
    });
    await user.click(await screen.findByRole("option", { name: /Called Avery/ }));

    expect(await screen.findByRole("heading", { name: "Avery Cole" })).toBeVisible();
    expect(recordRecent).toHaveBeenCalledWith("contact", "contact-parent");
  });

  it("does not record or leave search when the navigation target is unavailable", async () => {
    const user = userEvent.setup();
    const recordRecent = vi.fn();
    const client = stubClient({
      searchRecords: vi.fn().mockResolvedValue([
        {
          entityType: "contact",
          entityId: "missing-contact",
          title: "Missing contact",
          parentType: null,
          parentId: null,
        },
      ]),
      getContact: vi.fn().mockRejectedValue(new Error("not found")),
      recordRecent,
    });
    render(<App client={client} />);

    await user.click(screen.getByRole("button", { name: /Search/ }));
    fireEvent.change(screen.getByRole("combobox", { name: "Search records" }), {
      target: { value: "missing" },
    });
    await user.click(await screen.findByRole("option", { name: /Missing contact/ }));

    expect(screen.getByRole("dialog", { name: "Search ContractorCRM" })).toBeVisible();
    expect(recordRecent).not.toHaveBeenCalled();
  });
});
