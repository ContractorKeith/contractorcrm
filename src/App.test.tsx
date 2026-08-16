import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { makeContact, stubClient } from "./test/stub-client";

describe("crm shell", () => {
  beforeEach(() => {
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
  });

  it("renders the themed shell with the empty state and core health", async () => {
    render(<App client={stubClient()} />);

    expect(screen.getByRole("link", { name: "ContractorCRM home" })).toBeVisible();
    expect(await screen.findByRole("heading", { name: "No contacts yet" })).toBeVisible();
    expect(await screen.findByText("Core ready · v0.1.0")).toBeVisible();
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
