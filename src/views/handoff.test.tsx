import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import {
  makeContact,
  makeHandoffRef,
  makeOpportunity,
  makeOpportunityDetail,
  stubClient,
} from "../test/stub-client";

// Open the seeded opportunity's detail view from the pipeline table.
async function openDetail(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Pipeline" }));
  await user.click(await screen.findByText("Backyard fence"));
  await screen.findByRole("heading", { name: /Backyard fence/ });
}

// Detail stub at version 4; overrides tune the hand-off scenario per test.
function detailClient(overrides: Parameters<typeof stubClient>[0] = {}) {
  return stubClient({
    listOpportunities: vi.fn().mockResolvedValue([makeOpportunity()]),
    getOpportunity: vi.fn().mockResolvedValue(makeOpportunityDetail({ version: 4 })),
    getContact: vi.fn().mockResolvedValue(makeContact()),
    ...overrides,
  });
}

// Version 4 on the won stage — the job link form should render.
const wonDetail = () => makeOpportunityDetail({ version: 4, stageId: "stage-won" });

describe("hand-off panel", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("renders not-linked placeholders for both references", async () => {
    const user = userEvent.setup();
    const client = detailClient();

    render(<App client={client} />);
    await openDetail(user);

    expect(screen.getByRole("heading", { name: "Hand-off" })).toBeVisible();
    expect(screen.getAllByText("Not linked")).toHaveLength(2);
  });

  it("links a quote with the drafted reference and the loaded version", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      linkQuote: vi.fn().mockResolvedValue(makeOpportunity({ version: 5 })),
    });

    render(<App client={client} />);
    await openDetail(user);

    await user.type(screen.getByLabelText("Quote tool"), "quoter");
    await user.type(screen.getByLabelText("Quote id"), "Q-123");
    await user.type(screen.getByLabelText("Quote label"), "Backyard quote");
    await user.click(screen.getByRole("button", { name: "Link quote" }));

    expect(client.linkQuote).toHaveBeenCalledWith({
      opportunityId: "opp-1",
      expectedVersion: 4,
      quoteRef: { tool: "quoter", externalId: "Q-123", label: "Backyard quote" },
    });
    // The record reloads after a successful link.
    expect(client.getOpportunity).toHaveBeenCalledTimes(2);
  });

  it("shows the linked quote row with tool, id, label, and link time", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      getOpportunity: vi.fn().mockResolvedValue(
        makeOpportunityDetail({
          version: 4,
          quoteRef: makeHandoffRef({ label: "Q-123 rev A" }),
        }),
      ),
    });

    render(<App client={client} />);
    await openDetail(user);

    expect(
      screen.getByText("quoter · Q-123 · Q-123 rev A · linked 2026-08-14T17:55:00Z"),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Unlink quote" })).toBeVisible();
  });

  it("hides the job form on an open stage and shows the won hint", async () => {
    const user = userEvent.setup();
    const client = detailClient();

    render(<App client={client} />);
    await openDetail(user);

    expect(screen.queryByLabelText("Job tool")).not.toBeInTheDocument();
    expect(
      screen.getByText("Job hand-off is available once this deal is won."),
    ).toBeVisible();
  });

  it("shows the job form on the won stage and links with the loaded version", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      getOpportunity: vi.fn().mockResolvedValue(wonDetail()),
      linkJob: vi.fn().mockResolvedValue(makeOpportunity({ version: 5 })),
    });

    render(<App client={client} />);
    await openDetail(user);

    await user.type(screen.getByLabelText("Job tool"), "contractorproject");
    await user.type(screen.getByLabelText("Job id"), "job-77");
    await user.click(screen.getByRole("button", { name: "Link job" }));

    expect(client.linkJob).toHaveBeenCalledWith({
      opportunityId: "opp-1",
      expectedVersion: 4,
      jobRef: { tool: "contractorproject", externalId: "job-77", label: null },
    });
  });

  it("unlinks the job only after the user confirms", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      getOpportunity: vi.fn().mockResolvedValue(
        makeOpportunityDetail({
          version: 4,
          stageId: "stage-won",
          jobRef: makeHandoffRef({ tool: "contractorproject", externalId: "job-77" }),
        }),
      ),
      unlinkJob: vi.fn().mockResolvedValue(makeOpportunity({ version: 5 })),
    });
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);

    render(<App client={client} />);
    await openDetail(user);

    await user.click(screen.getByRole("button", { name: "Unlink job" }));
    expect(client.unlinkJob).not.toHaveBeenCalled();

    confirmSpy.mockReturnValue(true);
    await user.click(screen.getByRole("button", { name: "Unlink job" }));
    expect(client.unlinkJob).toHaveBeenCalledWith({
      opportunityId: "opp-1",
      expectedVersion: 4,
    });
  });

  it("surfaces the core's opportunity_not_won rejection plainly", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      // Won stage in the UI, but the core saw a stage change and refuses.
      getOpportunity: vi.fn().mockResolvedValue(wonDetail()),
      linkJob: vi.fn().mockRejectedValue({
        kind: "validation_failed",
        message: 'cannot link a job to opportunity "Backyard fence": it is in stage "Quoted", and job hand-offs require the won stage',
        code: "opportunity_not_won",
        field: "opportunityId",
      }),
    });

    render(<App client={client} />);
    await openDetail(user);

    await user.type(screen.getByLabelText("Job tool"), "contractorproject");
    await user.type(screen.getByLabelText("Job id"), "job-77");
    await user.click(screen.getByRole("button", { name: "Link job" }));

    expect(await screen.findByText(/job hand-offs require the won stage/)).toBeVisible();
  });

  it("exports the envelope and shows the written path", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      exportHandoffEnvelope: vi.fn().mockResolvedValue({
        destinationPath: "/tmp/backyard-fence.json",
        schemaVersion: 1,
      }),
    });

    render(<App client={client} />);
    await openDetail(user);

    await user.type(screen.getByLabelText("Export destination"), "/tmp/backyard-fence.json");
    await user.click(screen.getByRole("button", { name: "Export envelope" }));

    expect(client.exportHandoffEnvelope).toHaveBeenCalledWith(
      "opp-1",
      "/tmp/backyard-fence.json",
      false,
    );
    expect(await screen.findByText("Exported to /tmp/backyard-fence.json")).toBeVisible();
  });

  it("offers an overwrite confirm on destination_exists and retries with overwrite", async () => {
    const user = userEvent.setup();
    const exportHandoffEnvelope = vi
      .fn()
      .mockRejectedValueOnce({
        kind: "validation_failed",
        message: "/tmp/backyard-fence.json already exists; enable overwrite to replace it",
        code: "destination_exists",
        field: "destinationPath",
      })
      .mockResolvedValueOnce({
        destinationPath: "/tmp/backyard-fence.json",
        schemaVersion: 1,
      });
    const client = detailClient({ exportHandoffEnvelope });

    render(<App client={client} />);
    await openDetail(user);

    await user.type(screen.getByLabelText("Export destination"), "/tmp/backyard-fence.json");
    await user.click(screen.getByRole("button", { name: "Export envelope" }));

    expect(await screen.findByText("File exists — overwrite?")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Overwrite" }));

    expect(exportHandoffEnvelope).toHaveBeenLastCalledWith(
      "opp-1",
      "/tmp/backyard-fence.json",
      true,
    );
    expect(await screen.findByText("Exported to /tmp/backyard-fence.json")).toBeVisible();
  });

  it("shows the conflict banner when a link hits a version conflict", async () => {
    const user = userEvent.setup();
    const client = detailClient({
      linkQuote: vi.fn().mockRejectedValue({
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

    await user.type(screen.getByLabelText("Quote tool"), "quoter");
    await user.type(screen.getByLabelText("Quote id"), "Q-123");
    await user.click(screen.getByRole("button", { name: "Link quote" }));

    expect(await screen.findByText(/changed outside this form/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Reload latest" })).toBeVisible();
  });
});
