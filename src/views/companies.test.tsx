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

  it("applies a saved company sort and archived filter", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listSavedViews: vi.fn().mockImplementation((entityType) =>
        Promise.resolve(
          entityType === "company"
            ? [{
                id: "view-companies",
                name: "All companies Z-A",
                entityType: "company",
                definition: { schemaVersion: 1, filter: { includeArchived: true }, sort: { field: "name", direction: "descending" } },
                sortKey: 0,
                createdAt: "2026-08-16T20:00:00Z",
                updatedAt: "2026-08-16T20:00:00Z",
                version: 1,
              }]
            : [],
        ),
      ),
      listCompanies: vi.fn().mockResolvedValue([
        makeCompany({ id: "a", name: "Alpha" }),
        makeCompany({ id: "z", name: "Zulu", archivedAt: "2026-08-01T00:00:00Z" }),
      ]),
    });

    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "Companies" }));
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Saved company view" }),
      "view-companies",
    );
    expect(client.listCompanies).toHaveBeenLastCalledWith(true);
    const rows = within(await screen.findByRole("table", { name: "Company list" })).getAllByRole("row");
    expect(within(rows[1]!).getByText("Zulu")).toBeVisible();
    expect(within(rows[2]!).getByText("Alpha")).toBeVisible();
  });

  it("routes typed filters through the company saved-view matcher", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listCustomFieldDefs: vi.fn().mockResolvedValue([{ id: "field-date", entityType: "company", label: "Renewal", fieldType: "date", sortKey: 0, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [] }]),
      listCompanies: vi.fn().mockResolvedValue([makeCompany({ id: "kept" }), makeCompany({ id: "hidden", name: "Hidden Co" })]),
      matchSavedView: vi.fn().mockResolvedValue(["kept"]),
    });
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "Companies" }));
    await user.type(await screen.findByLabelText("Renewal filter"), "2026-10-01");
    expect(await screen.findByText("Coastal Fence Co")).toBeVisible();
    expect(screen.queryByText("Hidden Co")).not.toBeInTheDocument();
    expect(client.matchSavedView).toHaveBeenCalledWith("company", expect.objectContaining({ filter: expect.objectContaining({ customFields: [expect.objectContaining({ definitionId: "field-date", fieldType: "date" })] }) }));
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
    expect(await screen.findByRole("heading", { name: "Tags and custom fields" })).toBeVisible();
    expect(client.getRecordMetadata).toHaveBeenCalledWith("company", "co-new");
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
