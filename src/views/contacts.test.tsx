import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { makeCompany, makeContact, stubClient } from "../test/stub-client";
import { formatLocalDateTime } from "./date-format";

describe("contact list and detail", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("renders contact rows with company, kind, role, channel, and favorite", async () => {
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([
        makeContact({
          id: "c1",
          displayName: "Dana Ruiz",
          companyId: "company-1",
          kind: "client",
          role: "owner",
          favorite: true,
          channels: [
            {
              id: "ch1",
              contactId: "c1",
              kind: "phone",
              label: "Mobile",
              value: "555-0134",
              preferred: true,
              sortKey: 0,
            },
          ],
        }),
        makeContact({ id: "c2", displayName: "Marco Bell", role: "estimator", kind: "sub" }),
      ]),
      listCompanies: vi.fn().mockResolvedValue([makeCompany({ id: "company-1" })]),
    });

    render(<App client={client} />);

    const table = await screen.findByRole("table", { name: "Contact list" });
    const firstRow = within(table).getAllByRole("row")[1]!;
    expect(within(firstRow).getByText("Dana Ruiz")).toBeVisible();
    expect(within(firstRow).getByText("Coastal Fence Co")).toBeVisible();
    expect(within(firstRow).getByText("Client")).toBeVisible();
    expect(within(firstRow).getByText("Owner")).toBeVisible();
    expect(within(firstRow).getByText("555-0134")).toBeVisible();
    expect(within(firstRow).getByText("★")).toBeVisible();
    expect(within(table).getByText("Marco Bell")).toBeVisible();
  });

  it("renders the last-contacted and next-task projection columns", async () => {
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([
        {
          ...makeContact({ id: "c1", displayName: "Dana Ruiz" }),
          lastContactedAt: "2026-08-12T15:00:00Z",
          nextOpenTaskDueAt: "2026-08-20T16:00:00Z",
        },
      ]),
    });

    render(<App client={client} />);

    const table = await screen.findByRole("table", { name: "Contact list" });
    expect(within(table).getByRole("columnheader", { name: "Last contacted" })).toBeVisible();
    expect(within(table).getByRole("columnheader", { name: "Next task" })).toBeVisible();
    const row = within(table).getAllByRole("row")[1]!;
    expect(within(row).getByText(formatLocalDateTime("2026-08-12T15:00:00Z"))).toBeVisible();
    expect(within(row).getByText("2026-08-20T16:00:00Z")).toBeVisible();
  });

  it("moves selection with arrow keys and opens the record with Enter", async () => {
    const user = userEvent.setup();
    const contacts = [
      makeContact({ id: "c1", displayName: "Dana Ruiz" }),
      makeContact({ id: "c2", displayName: "Marco Bell" }),
    ];
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue(contacts),
      getContact: vi.fn().mockResolvedValue(contacts[1]),
    });

    render(<App client={client} />);

    const table = await screen.findByRole("table", { name: "Contact list" });
    const rows = within(table).getAllByRole("row");
    rows[1]!.focus();

    await user.keyboard("{ArrowDown}");
    expect(rows[2]).toHaveFocus();

    await user.keyboard("{Enter}");
    expect(client.getContact).toHaveBeenCalledWith("c2");
    expect(await screen.findByRole("heading", { name: /Marco Bell/ })).toBeVisible();
    expect(await screen.findByRole("heading", { name: "Tags and custom fields" })).toBeVisible();
    expect(client.getRecordMetadata).toHaveBeenCalledWith("contact", "c2");
  });

  it("hides archived contacts by default and shows them with the toggle", async () => {
    const user = userEvent.setup();
    const active = makeContact({ id: "c1", displayName: "Dana Ruiz" });
    const archived = makeContact({
      id: "c2",
      displayName: "Old Sub",
      archivedAt: "2026-08-01T00:00:00Z",
    });
    const client = stubClient({
      listContacts: vi
        .fn()
        .mockImplementation((includeArchived: boolean) =>
          Promise.resolve(includeArchived ? [active, archived] : [active]),
        ),
    });

    render(<App client={client} />);

    await screen.findByRole("table", { name: "Contact list" });
    expect(screen.queryByText("Old Sub")).not.toBeInTheDocument();

    await user.click(screen.getByLabelText("Show archived"));
    expect(await screen.findByText("Old Sub")).toBeVisible();
  });

  it("applies a saved archive filter and deterministic contact sort", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listSavedViews: vi.fn().mockResolvedValue([
        {
          id: "view-contacts",
          name: "All contacts Z-A",
          entityType: "contact",
          definition: {
            schemaVersion: 1,
            filter: { includeArchived: true },
            sort: { field: "displayName", direction: "descending" },
          },
          sortKey: 0,
          createdAt: "2026-08-16T20:00:00Z",
          updatedAt: "2026-08-16T20:00:00Z",
          version: 1,
        },
      ]),
      listContacts: vi.fn().mockResolvedValue([
        makeContact({ id: "a", displayName: "Alpha" }),
        makeContact({ id: "z", displayName: "Zulu", archivedAt: "2026-08-01T00:00:00Z" }),
      ]),
    });

    render(<App client={client} />);
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Saved contact view" }),
      "view-contacts",
    );
    expect(client.listContacts).toHaveBeenLastCalledWith(true);
    const table = await screen.findByRole("table", { name: "Contact list" });
    expect(within(table).getByRole("columnheader", { name: "Name" })).toHaveAttribute(
      "aria-sort",
      "descending",
    );
    const rows = within(table).getAllByRole("row");
    expect(within(rows[1]!).getByText("Zulu")).toBeVisible();
    expect(within(rows[2]!).getByText("Alpha")).toBeVisible();
  });

  it("uses the core matcher for an active tag filter", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listTags: vi.fn().mockResolvedValue([{ id: "tag-client", label: "Client", colorRole: null, archivedAt: null, createdAt: "", updatedAt: "", version: 1 }]),
      listContacts: vi.fn().mockResolvedValue([makeContact({ id: "kept" }), makeContact({ id: "hidden", displayName: "Hidden" })]),
      matchSavedView: vi.fn().mockResolvedValue(["kept"]),
    });
    render(<App client={client} />);
    await user.selectOptions(await screen.findByLabelText("Tags (all must match)"), "tag-client");
    expect(await screen.findByText("Dana Ruiz")).toBeVisible();
    expect(screen.queryByText("Hidden")).not.toBeInTheDocument();
    expect(client.matchSavedView).toHaveBeenCalledWith("contact", expect.objectContaining({ schemaVersion: 2, filter: expect.objectContaining({ tagIdsAll: ["tag-client"] }) }));
  });

  it("submits the create payload through the client", async () => {
    const user = userEvent.setup();
    const created = makeContact({ id: "new-1", displayName: "Sam Ortega" });
    const client = stubClient({
      createContact: vi.fn().mockResolvedValue(created),
      getContact: vi.fn().mockResolvedValue(created),
    });

    render(<App client={client} />);
    await user.click(await screen.findByRole("button", { name: "New contact" }));

    await user.type(screen.getByLabelText("First name"), "Sam");
    await user.type(screen.getByLabelText("Last name"), "Ortega");
    await user.selectOptions(screen.getByLabelText("Kind"), "client");
    await user.selectOptions(screen.getByLabelText("Role"), "site_contact");

    await user.click(screen.getByRole("button", { name: "Add phone or email" }));
    await user.type(screen.getByLabelText("Value"), "555-0188");
    await user.click(screen.getByLabelText("Preferred"));

    await user.click(screen.getByRole("button", { name: "Create contact" }));

    expect(client.createContact).toHaveBeenCalledWith(
      expect.objectContaining({
        firstName: "Sam",
        lastName: "Ortega",
        displayName: null,
        kind: "client",
        role: "site_contact",
        companyId: null,
        favorite: false,
        channels: [{ kind: "phone", label: null, value: "555-0188", preferred: true }],
      }),
    );
    expect(await screen.findByRole("heading", { name: /Sam Ortega/ })).toBeVisible();
  });

  it("shows a validation error from the core inline near the field", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      createContact: vi.fn().mockRejectedValue({
        kind: "invalid_input",
        message: "displayName: is required when first and last name are both empty",
        field: "displayName",
      }),
    });

    render(<App client={client} />);
    await user.click(await screen.findByRole("button", { name: "New contact" }));
    const displayName = screen.getByLabelText("Display name").closest("label")!;
    await user.click(screen.getByRole("button", { name: "Create contact" }));

    expect(
      await within(displayName).findByText("is required when first and last name are both empty"),
    ).toBeVisible();
  });

  it("offers a reload when a save hits a version conflict", async () => {
    const user = userEvent.setup();
    const contact = makeContact({ id: "c1", displayName: "Dana Ruiz", version: 1 });
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([contact]),
      getContact: vi.fn().mockResolvedValue(contact),
      updateContact: vi.fn().mockRejectedValue({
        kind: "version_conflict",
        message: "contact c1 changed: expected version 1, current version 3",
        resource: "contact",
        recordId: "c1",
        expectedVersion: 1,
        currentVersion: 3,
      }),
    });

    render(<App client={client} />);
    await user.click(await screen.findByText("Dana Ruiz"));
    await user.click(await screen.findByRole("button", { name: "Edit" }));
    await user.click(await screen.findByRole("button", { name: "Save contact" }));

    expect(await screen.findByText(/changed outside this form/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Reload latest" })).toBeVisible();
  });

  it("archives from the detail view with the current version", async () => {
    const user = userEvent.setup();
    const contact = makeContact({ id: "c1", displayName: "Dana Ruiz", version: 2 });
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([contact]),
      getContact: vi.fn().mockResolvedValue(contact),
      archiveContact: vi
        .fn()
        .mockResolvedValue({ ...contact, archivedAt: "2026-08-14T13:00:00Z", version: 3 }),
    });

    render(<App client={client} />);
    await user.click(await screen.findByText("Dana Ruiz"));
    await user.click(await screen.findByRole("button", { name: "Archive" }));

    expect(client.archiveContact).toHaveBeenCalledWith({ id: "c1", expectedVersion: 2 });
    expect(await screen.findByRole("button", { name: "Unarchive" })).toBeVisible();
  });
});
