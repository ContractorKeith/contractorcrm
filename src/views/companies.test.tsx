import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { makeCompany, stubClient } from "../test/stub-client";

describe("company list and form", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("renders company rows with kind, phone, email, and service area", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listCompanies: vi.fn().mockResolvedValue([
        makeCompany({
          id: "co1",
          name: "Coastal Fence Co",
          kind: "sub",
          phone: "555-0100",
          email: "office@coastalfence.test",
          serviceArea: "Central Florida",
        }),
      ]),
    });

    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "Companies" }));

    const table = await screen.findByRole("table", { name: "Company list" });
    const row = within(table).getAllByRole("row")[1]!;
    expect(within(row).getByText("Coastal Fence Co")).toBeVisible();
    expect(within(row).getByText("Sub")).toBeVisible();
    expect(within(row).getByText("555-0100")).toBeVisible();
    expect(within(row).getByText("office@coastalfence.test")).toBeVisible();
    expect(within(row).getByText("Central Florida")).toBeVisible();
  });

  it("creates a company through the client and lands on its detail", async () => {
    const user = userEvent.setup();
    const created = makeCompany({ id: "co-new", name: "Gulf Gates LLC", kind: "vendor" });
    const client = stubClient({
      createCompany: vi.fn().mockResolvedValue(created),
      getCompany: vi.fn().mockResolvedValue(created),
    });

    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "Companies" }));
    await user.click(await screen.findByRole("button", { name: "New company" }));

    await user.type(screen.getByLabelText("Name"), "Gulf Gates LLC");
    await user.selectOptions(screen.getByLabelText("Kind"), "vendor");
    await user.type(screen.getByLabelText("Service area"), "Tampa Bay");
    await user.click(screen.getByRole("button", { name: "Create company" }));

    expect(client.createCompany).toHaveBeenCalledWith(
      expect.objectContaining({ name: "Gulf Gates LLC", kind: "vendor", serviceArea: "Tampa Bay" }),
    );
    expect(await screen.findByRole("heading", { name: /Gulf Gates LLC/ })).toBeVisible();
  });

  it("shows the archive validation message when a company still has active contacts", async () => {
    const user = userEvent.setup();
    const company = makeCompany({ id: "co1", name: "Coastal Fence Co", version: 1 });
    const client = stubClient({
      listCompanies: vi.fn().mockResolvedValue([company]),
      getCompany: vi.fn().mockResolvedValue(company),
      archiveCompany: vi.fn().mockRejectedValue({
        kind: "validation_failed",
        message:
          'cannot archive company "Coastal Fence Co": it still has 2 active contact(s); archive or reassign them first',
        code: "company_has_active_contacts",
        field: "companyId",
      }),
    });

    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "Companies" }));
    await user.click(await screen.findByText("Coastal Fence Co"));
    await user.click(await screen.findByRole("button", { name: "Archive" }));

    expect(await screen.findByText(/still has 2 active contact/)).toBeVisible();
  });
});
