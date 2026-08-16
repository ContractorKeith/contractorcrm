import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import {
  makeContact,
  makeLostReason,
  makeOpportunity,
  makeOpportunityDetail,
  makeStageHistoryEntry,
  stubClient,
} from "../test/stub-client";
import { formatLocalDateTime } from "./date-format";

// Open the Pipeline tab from the app shell.
async function openPipeline(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Pipeline" }));
}

describe("pipeline table", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("renders opportunity rows with formatted money from integer minor units", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue([
        makeOpportunity({
          id: "o1",
          name: "Backyard fence",
          value: { valueMinor: 123456, currencyCode: "USD" },
          probabilityPercent: 50,
          expectedCloseDate: "2026-09-01",
        }),
      ]),
    });

    render(<App client={client} />);
    await openPipeline(user);

    const table = await screen.findByRole("table", { name: "Pipeline list" });
    const row = within(table).getAllByRole("row")[1]!;
    expect(within(row).getByText("Backyard fence")).toBeVisible();
    expect(within(row).getByText("New lead")).toBeVisible();
    expect(within(row).getByText("Dana Ruiz")).toBeVisible();
    expect(within(row).getByText("$1,234.56")).toBeVisible();
    expect(within(row).getByText("50%")).toBeVisible();
    expect(within(row).getByText("2026-09-01")).toBeVisible();
    expect(within(row).getByText("Referral")).toBeVisible();
  });

  it("renders the last-contacted and next-task projection columns", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue([
        makeOpportunity({
          id: "o1",
          lastContactedAt: "2026-08-12T15:00:00Z",
          nextOpenTaskDueAt: "2026-08-20T16:00:00Z",
        }),
      ]),
    });

    render(<App client={client} />);
    await openPipeline(user);

    const table = await screen.findByRole("table", { name: "Pipeline list" });
    expect(within(table).getByRole("columnheader", { name: "Last contacted" })).toBeVisible();
    expect(within(table).getByRole("columnheader", { name: "Next task" })).toBeVisible();
    const row = within(table).getAllByRole("row")[1]!;
    expect(within(row).getByText(formatLocalDateTime("2026-08-12T15:00:00Z"))).toBeVisible();
    expect(within(row).getByText("2026-08-20T16:00:00Z")).toBeVisible();
  });

  it("sorts by value on header click and reports aria-sort", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue([
        makeOpportunity({ id: "o1", name: "Big job", value: { valueMinor: 900000, currencyCode: "USD" } }),
        makeOpportunity({ id: "o2", name: "Small job", value: { valueMinor: 10000, currencyCode: "USD" } }),
      ]),
    });

    render(<App client={client} />);
    await openPipeline(user);

    const table = await screen.findByRole("table", { name: "Pipeline list" });
    const valueHeader = within(table).getByRole("columnheader", { name: "Value" });
    expect(valueHeader).toHaveAttribute("aria-sort", "none");

    await user.click(within(valueHeader).getByRole("button"));
    expect(valueHeader).toHaveAttribute("aria-sort", "ascending");
    let rows = within(table).getAllByRole("row");
    expect(within(rows[1]!).getByText("Small job")).toBeVisible();
    expect(within(rows[2]!).getByText("Big job")).toBeVisible();

    await user.click(within(valueHeader).getByRole("button"));
    expect(valueHeader).toHaveAttribute("aria-sort", "descending");
    rows = within(table).getAllByRole("row");
    expect(within(rows[1]!).getByText("Big job")).toBeVisible();
    expect(within(rows[2]!).getByText("Small job")).toBeVisible();
  });

  it("hides archived opportunities by default and shows them with the toggle", async () => {
    const user = userEvent.setup();
    const active = makeOpportunity({ id: "o1", name: "Live deal" });
    const archived = makeOpportunity({
      id: "o2",
      name: "Old deal",
      archivedAt: "2026-08-01T00:00:00Z",
    });
    const client = stubClient({
      listOpportunities: vi
        .fn()
        .mockImplementation((includeArchived: boolean) =>
          Promise.resolve(includeArchived ? [active, archived] : [active]),
        ),
    });

    render(<App client={client} />);
    await openPipeline(user);

    await screen.findByRole("table", { name: "Pipeline list" });
    expect(screen.queryByText("Old deal")).not.toBeInTheDocument();

    await user.click(screen.getByLabelText("Show archived"));
    expect(await screen.findByText("Old deal")).toBeVisible();
  });
});

describe("opportunity form", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("converts the dollars input to integer minor units on create", async () => {
    const user = userEvent.setup();
    const detail = makeOpportunityDetail();
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([makeContact()]),
      createOpportunity: vi.fn().mockResolvedValue(makeOpportunity()),
      getOpportunity: vi.fn().mockResolvedValue(detail),
      getContact: vi.fn().mockResolvedValue(makeContact()),
    });

    render(<App client={client} />);
    await openPipeline(user);
    await user.click(await screen.findByRole("button", { name: "New opportunity" }));

    await user.type(screen.getByLabelText("Name"), "Backyard fence");
    await user.selectOptions(screen.getByLabelText("Contact"), "contact-1");
    await user.type(screen.getByLabelText("Value ($)"), "$1,234.56");

    await user.click(screen.getByRole("button", { name: "Create opportunity" }));

    expect(client.createOpportunity).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Backyard fence",
        contactId: "contact-1",
        companyId: null,
        stageId: "stage-new", // defaults to the first open stage
        valueMinor: 123456,
        currencyCode: "USD",
      }),
    );
  });

  it("rejects an unparseable dollars input without calling the core", async () => {
    const user = userEvent.setup();
    const client = stubClient();

    render(<App client={client} />);
    await openPipeline(user);
    await user.click(await screen.findByRole("button", { name: "New opportunity" }));

    await user.type(screen.getByLabelText("Name"), "Backyard fence");
    await user.type(screen.getByLabelText("Value ($)"), "about 5k");
    await user.click(screen.getByRole("button", { name: "Create opportunity" }));

    expect(await screen.findByText(/Enter a dollar amount/)).toBeVisible();
    expect(client.createOpportunity).not.toHaveBeenCalled();
  });

  it("surfaces the server's link-required validation error inline", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      createOpportunity: vi.fn().mockRejectedValue({
        kind: "validation_failed",
        message: "contactId: link a contact or a company",
        code: "opportunity_link_required",
        field: "contactId",
      }),
    });

    render(<App client={client} />);
    await openPipeline(user);
    await user.click(await screen.findByRole("button", { name: "New opportunity" }));

    await user.type(screen.getByLabelText("Name"), "Backyard fence");
    const contactField = screen.getByLabelText("Contact").closest("label")!;
    await user.click(screen.getByRole("button", { name: "Create opportunity" }));

    expect(await within(contactField).findByText("link a contact or a company")).toBeVisible();
  });
});

describe("opportunity detail and stage moves", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  // Open the seeded opportunity's detail view from the pipeline table.
  async function openDetail(user: ReturnType<typeof userEvent.setup>) {
    await openPipeline(user);
    await user.click(await screen.findByText("Backyard fence"));
    await screen.findByRole("heading", { name: /Backyard fence/ });
  }

  function detailClient(overrides: Parameters<typeof stubClient>[0] = {}) {
    return stubClient({
      listOpportunities: vi.fn().mockResolvedValue([makeOpportunity()]),
      getOpportunity: vi.fn().mockResolvedValue(makeOpportunityDetail({ version: 4 })),
      getContact: vi.fn().mockResolvedValue(makeContact()),
      listLostReasons: vi.fn().mockResolvedValue([makeLostReason()]),
      ...overrides,
    });
  }

  it("moves stage through the command with the expected version", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      moveOpportunityStage: vi.fn().mockResolvedValue(makeOpportunity({ stageId: "stage-estimating" })),
    });

    render(<App client={client} />);
    await openDetail(user);

    await user.selectOptions(screen.getByLabelText("Move to stage"), "stage-estimating");
    await user.click(screen.getByRole("button", { name: "Move" }));

    expect(client.moveOpportunityStage).toHaveBeenCalledWith({
      opportunityId: "opp-1",
      toStageId: "stage-estimating",
      lostReasonId: null,
      expectedVersion: 4,
    });
  });

  it("blocks a lost move without a reason before it reaches the core", async () => {
    const user = userEvent.setup();
    const client = detailClient();

    render(<App client={client} />);
    await openDetail(user);

    await user.selectOptions(screen.getByLabelText("Move to stage"), "stage-lost");
    await user.click(screen.getByRole("button", { name: "Move" }));

    expect(await screen.findByText(/Select a lost reason/)).toBeVisible();
    expect(client.moveOpportunityStage).not.toHaveBeenCalled();
  });

  it("renders a missing_lost_reason error from the core next to the reason select", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      moveOpportunityStage: vi.fn().mockRejectedValue({
        kind: "missing_lost_reason",
        message: "moving opportunity opp-1 to the lost stage requires a lost reason",
        resource: "opportunity",
        recordId: "opp-1",
      }),
    });

    render(<App client={client} />);
    await openDetail(user);

    await user.selectOptions(screen.getByLabelText("Move to stage"), "stage-lost");
    await user.selectOptions(screen.getByLabelText("Lost reason"), "reason-1");
    await user.click(screen.getByRole("button", { name: "Move" }));

    expect(
      await screen.findByText(/requires a lost reason/),
    ).toBeVisible();
  });

  it("shows the conflict banner when a move hits a version conflict", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      moveOpportunityStage: vi.fn().mockRejectedValue({
        kind: "version_conflict",
        message: "opportunity opp-1 changed: expected version 4, current version 6",
        resource: "opportunity",
        recordId: "opp-1",
        expectedVersion: 4,
        currentVersion: 6,
      }),
    });

    render(<App client={client} />);
    await openDetail(user);

    await user.selectOptions(screen.getByLabelText("Move to stage"), "stage-won");
    await user.click(screen.getByRole("button", { name: "Move" }));

    expect(await screen.findByText(/changed outside this form/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Reload latest" })).toBeVisible();
  });

  it("shows an em dash for the source when the kind is unset, even with a label", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      getOpportunity: vi.fn().mockResolvedValue(
        makeOpportunityDetail({ source: null, sourceLabel: "Angie's List" }),
      ),
    });

    render(<App client={client} />);
    await openDetail(user);

    // The free-text label never stands in for a missing source kind.
    expect(screen.getByText("Source").nextElementSibling).toHaveTextContent("—");
    expect(screen.queryByText(/Angie's List/)).not.toBeInTheDocument();
  });

  it("hides and clears source detail in the editor when source kind is unset", async () => {
    const user = userEvent.setup();
    const updateOpportunity = vi.fn().mockResolvedValue(makeOpportunity());
    const client = detailClient({
      getOpportunity: vi.fn().mockResolvedValue(
        makeOpportunityDetail({ source: null, sourceLabel: "orphaned detail" }),
      ),
      updateOpportunity,
    });

    render(<App client={client} />);
    await openDetail(user);
    await user.click(screen.getByRole("button", { name: "Edit" }));

    expect(await screen.findByRole("heading", { name: "Edit opportunity" })).toBeVisible();
    expect(screen.queryByLabelText("Source detail")).not.toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("Source"), "referral");
    await user.type(screen.getByLabelText("Source detail"), "Dana");
    await user.selectOptions(screen.getByLabelText("Source"), "");
    expect(screen.queryByLabelText("Source detail")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save opportunity" }));

    expect(updateOpportunity).toHaveBeenCalledWith(
      expect.objectContaining({
        patch: expect.objectContaining({ source: null, sourceLabel: null }),
      }),
    );
  });

  it("appends the free-text label to a set source kind", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      getOpportunity: vi.fn().mockResolvedValue(
        makeOpportunityDetail({ source: "referral", sourceLabel: "referred by Dana" }),
      ),
    });

    render(<App client={client} />);
    await openDetail(user);

    expect(screen.getByText("Referral · referred by Dana")).toBeVisible();
  });

  it("renders the stage history newest-first with actor and lost reason", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      getOpportunity: vi.fn().mockResolvedValue(
        makeOpportunityDetail({
          stageId: "stage-lost",
          lostReasonId: "reason-1",
          stageHistory: [
            makeStageHistoryEntry({ id: "h1", createdAt: "2026-08-10T12:00:00Z" }),
            makeStageHistoryEntry({
              id: "h2",
              fromStageId: "stage-new",
              toStageId: "stage-lost",
              actor: "agent",
              lostReasonId: "reason-1",
              createdAt: "2026-08-12T12:00:00Z",
            }),
          ],
        }),
      ),
    });

    render(<App client={client} />);
    await openDetail(user);

    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(within(items[0]!).getByText("New lead → Lost")).toBeVisible();
    expect(within(items[0]!).getByText(/agent · 2026-08-12T12:00:00Z · Price too high/)).toBeVisible();
    expect(within(items[1]!).getByText("— → New lead")).toBeVisible();
  });
});
