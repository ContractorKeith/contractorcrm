import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { App } from "./App";
import { stubClient } from "./test/stub-client";

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
});
