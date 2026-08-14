import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import type { CoreClient } from "./api/health";

const stubClient = (): CoreClient => ({
  health: vi.fn().mockResolvedValue({ app: "ContractorCRM", version: "0.1.0", status: "ok" }),
});

describe("crm shell", () => {
  beforeEach(() => {
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
  });

  it("renders the themed shell with the empty state and core health", async () => {
    render(<App client={stubClient()} />);

    expect(screen.getByRole("link", { name: "ContractorCRM home" })).toBeVisible();
    expect(screen.getByRole("heading", { name: "No contacts yet" })).toBeVisible();
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
});
