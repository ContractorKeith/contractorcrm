import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import {
  makeContact,
  makeOpportunity,
  makeOpportunityDetail,
  stubClient,
} from "../test/stub-client";

// Open the Pipeline tab, then flip the toolbar switch to the board.
async function openBoard(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Pipeline" }));
  await user.click(await screen.findByRole("button", { name: "Board" }));
  return screen.findByRole("region", { name: "Pipeline board" });
}

// Board fixture: two in New lead, one Estimating, one Won, one Lost, one archived.
const boardOpportunities = () => [
  makeOpportunity({ id: "o1", name: "Backyard fence", value: { valueMinor: 100000, currencyCode: "USD" } }),
  makeOpportunity({
    id: "o2",
    name: "Pool enclosure",
    value: { valueMinor: 25050, currencyCode: "USD" },
    contactDisplayName: null,
    companyName: "Coastal Fence Co",
  }),
  makeOpportunity({
    id: "o3",
    name: "Gate repair",
    stageId: "stage-estimating",
    stageName: "Estimating",
    value: { valueMinor: 50000, currencyCode: "USD" },
  }),
  makeOpportunity({
    id: "o4",
    name: "Won job",
    stageId: "stage-won",
    stageName: "Won",
    value: { valueMinor: 700000, currencyCode: "USD" },
  }),
  makeOpportunity({
    id: "o5",
    name: "Lost job",
    stageId: "stage-lost",
    stageName: "Lost",
    value: { valueMinor: 30000, currencyCode: "USD" },
  }),
  makeOpportunity({
    id: "o6",
    name: "Archived deal",
    value: { valueMinor: 999900, currencyCode: "USD" },
    archivedAt: "2026-08-01T00:00:00Z",
  }),
];

describe("pipeline board", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("groups cards by stage with open columns in pipeline order, then Won and Lost", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue(boardOpportunities()),
    });

    render(<App client={client} />);
    const board = await openBoard(user);

    const columns = within(board).getAllByRole("region");
    expect(columns.map((column) => column.getAttribute("aria-label"))).toEqual([
      "New lead",
      "Estimating",
      "Quoted",
      "Won",
      "Lost",
    ]);

    const newLead = within(columns[0]!).getByRole("list", { name: "New lead opportunities" });
    const cards = within(newLead).getAllByRole("listitem");
    expect(cards).toHaveLength(2);
    expect(within(cards[0]!).getByText("Backyard fence")).toBeVisible();
    expect(within(cards[1]!).getByText("Pool enclosure")).toBeVisible();
    // Secondary line shows contact when present, company otherwise.
    expect(within(cards[0]!).getByText("Dana Ruiz")).toBeVisible();
    expect(within(cards[1]!).getByText("Coastal Fence Co")).toBeVisible();
    expect(
      within(columns[1]!).getByRole("list", { name: "Estimating opportunities" }),
    ).toBeInTheDocument();
  });

  it("shows per-column count and formatted total, and a quiet hint for empty stages", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue(boardOpportunities()),
    });

    render(<App client={client} />);
    const board = await openBoard(user);

    const newLead = within(board).getByRole("region", { name: "New lead" });
    expect(within(newLead).getByText("2")).toBeVisible();
    expect(within(newLead).getByText("$1,250.50")).toBeVisible(); // 100000 + 25050 minor units

    // Quoted has nothing in it — count 0, $0 total, empty hint instead of cards.
    const quoted = within(board).getByRole("region", { name: "Quoted" });
    expect(within(quoted).getByText("0")).toBeVisible();
    expect(within(quoted).getByText("$0.00")).toBeVisible();
    expect(within(quoted).getByText("Nothing in this stage.")).toBeVisible();
    expect(within(quoted).queryByRole("list")).not.toBeInTheDocument();
  });

  it("renders Won and Lost as card-free summaries with count and total", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue(boardOpportunities()),
    });

    render(<App client={client} />);
    const board = await openBoard(user);

    const won = within(board).getByRole("region", { name: "Won" });
    expect(within(won).getByText("1")).toBeVisible();
    expect(within(won).getByText("$7,000.00")).toBeVisible();
    expect(within(won).queryByRole("list")).not.toBeInTheDocument();
    expect(within(won).queryByRole("button")).not.toBeInTheDocument();

    const lost = within(board).getByRole("region", { name: "Lost" });
    expect(within(lost).getByText("$300.00")).toBeVisible();
    expect(within(lost).queryByRole("button")).not.toBeInTheDocument();
  });

  it("excludes archived opportunities from every column and total", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue(boardOpportunities()),
    });

    render(<App client={client} />);
    const board = await openBoard(user);

    expect(within(board).queryByText("Archived deal")).not.toBeInTheDocument();
    // New lead total stays the two live cards only.
    const newLead = within(board).getByRole("region", { name: "New lead" });
    expect(within(newLead).getByText("$1,250.50")).toBeVisible();
  });

  it("opens the opportunity detail when a card is clicked", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue(boardOpportunities()),
      getOpportunity: vi.fn().mockResolvedValue(makeOpportunityDetail()),
      getContact: vi.fn().mockResolvedValue(makeContact()),
    });

    render(<App client={client} />);
    await openBoard(user);

    await user.click(screen.getByRole("button", { name: /Backyard fence/ }));

    expect(await screen.findByRole("heading", { name: /Backyard fence/ })).toBeVisible();
    expect(client.getOpportunity).toHaveBeenCalledWith("o1");
  });

  it("toggles between the list table and the board", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listOpportunities: vi.fn().mockResolvedValue([makeOpportunity()]),
    });

    render(<App client={client} />);
    await user.click(await screen.findByRole("button", { name: "Pipeline" }));

    // Default is the table.
    expect(await screen.findByRole("table", { name: "Pipeline list" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Board" }));
    expect(await screen.findByRole("region", { name: "Pipeline board" })).toBeVisible();
    expect(screen.queryByRole("table", { name: "Pipeline list" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "List" }));
    expect(await screen.findByRole("table", { name: "Pipeline list" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Pipeline board" })).not.toBeInTheDocument();
  });

  it("draws the top of a very deep column and points at the list for the rest", async () => {
    const user = userEvent.setup();
    const deep = Array.from({ length: 130 }, (_, index) =>
      makeOpportunity({
        id: `deep-${index}`,
        name: `Deep job ${index}`,
        value: { valueMinor: 100, currencyCode: "USD" },
      }),
    );
    const client = stubClient({ listOpportunities: vi.fn().mockResolvedValue(deep) });

    render(<App client={client} />);
    const board = await openBoard(user);

    const newLead = within(board).getByRole("region", { name: "New lead" });
    // The count and total still cover every opportunity in the stage.
    expect(within(newLead).getByText("130")).toBeVisible();
    expect(within(newLead).getByText("$130.00")).toBeVisible();
    // Only the first 100 are drawn, and the column says so.
    expect(
      within(within(newLead).getByRole("list")).getAllByRole("listitem"),
    ).toHaveLength(100);
    expect(within(newLead).getByText(/Showing the first 100 of 130/)).toBeVisible();
    expect(within(newLead).getByText("Deep job 0")).toBeVisible();
    expect(within(newLead).queryByText("Deep job 100")).not.toBeInTheDocument();
  });
});
