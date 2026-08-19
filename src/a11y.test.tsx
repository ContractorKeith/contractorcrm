import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";
import { ContextDisclosure } from "./components/ContextDisclosure";
import {
  makeActivity,
  makeAttentionExplanation,
  makeAttentionFlag,
  makeContact,
  makeContextPreview,
  makeTask,
  stubClient,
} from "./test/stub-client";

// Cross-surface accessibility guarantees: names, focus movement, and live
// regions that are easy to regress in ordinary feature work.
describe("accessibility", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("captions record tables with the row count and the keyboard model", async () => {
    render(<App client={stubClient({ listContacts: vi.fn().mockResolvedValue([makeContact()]) })} />);

    const table = await screen.findByRole("table", { name: "Contact list" });
    expect(within(table).getByText(/1 row\./)).toBeInTheDocument();
    expect(within(table).getByText(/arrow keys to move between rows and Enter to open/)).toBeInTheDocument();
  });

  it("gives the favorite column readable text instead of a bare star", async () => {
    render(
      <App
        client={stubClient({
          listContacts: vi
            .fn()
            .mockResolvedValue([
              makeContact({ id: "c1", displayName: "Dana Ruiz", favorite: true }),
              makeContact({ id: "c2", displayName: "Avery Cole", favorite: false }),
            ]),
        })}
      />,
    );

    const table = await screen.findByRole("table", { name: "Contact list" });
    const dana = within(table).getByText("Dana Ruiz").closest("tr")!;
    const avery = within(table).getByText("Avery Cole").closest("tr")!;
    expect(within(dana).getByText("Favorite")).toBeInTheDocument();
    expect(within(avery).getByText("Not a favorite")).toBeInTheDocument();
  });

  it("moves focus to the workspace when the view changes so it is never lost", async () => {
    const user = userEvent.setup();
    render(<App client={stubClient({ listContacts: vi.fn().mockResolvedValue([makeContact()]) })} />);

    await screen.findByRole("table", { name: "Contact list" });
    await user.click(screen.getByRole("button", { name: "Companies" }));

    await waitFor(() => expect(screen.getByRole("main")).toHaveFocus());
  });

  it("names timeline row controls after the entry they act on", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([makeContact()]),
      getContact: vi.fn().mockResolvedValue(makeContact()),
      getTimeline: vi
        .fn()
        .mockResolvedValue([makeActivity({ id: "a1", kind: "call", summary: "Left a voicemail" })]),
    });

    render(<App client={client} />);
    await user.click(await screen.findByText("Dana Ruiz"));

    const list = await screen.findByRole("list", { name: "Activity entries" });
    expect(within(list).getByRole("button", { name: "Edit call — Left a voicemail" })).toBeVisible();
    expect(
      within(list).getByRole("button", { name: "Delete call — Left a voicemail" }),
    ).toBeVisible();
  });

  it("hands focus back to the entry's Edit button when the inline form closes", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([makeContact()]),
      getContact: vi.fn().mockResolvedValue(makeContact()),
      getTimeline: vi
        .fn()
        .mockResolvedValue([makeActivity({ id: "a1", kind: "call", summary: "Left a voicemail" })]),
    });

    render(<App client={client} />);
    await user.click(await screen.findByText("Dana Ruiz"));

    const list = await screen.findByRole("list", { name: "Activity entries" });
    await user.click(within(list).getByRole("button", { name: "Edit call — Left a voicemail" }));
    await user.click(screen.getByRole("button", { name: "Cancel editing call — Left a voicemail" }));

    await waitFor(() =>
      expect(
        within(screen.getByRole("list", { name: "Activity entries" })).getByRole("button", {
          name: "Edit call — Left a voicemail",
        }),
      ).toHaveFocus(),
    );
  });

  it("sends focus into the task editor and returns it to New task on cancel", async () => {
    const user = userEvent.setup();
    render(<App client={stubClient({ listTasks: vi.fn().mockResolvedValue([makeTask()]) })} />);

    await user.click(await screen.findByRole("button", { name: "Tasks" }));
    const newTask = await screen.findByRole("button", { name: "New task" });
    await user.click(newTask);

    const form = await screen.findByRole("form", { name: "Task form" });
    await waitFor(() => expect(within(form).getByLabelText("Title")).toHaveFocus());

    await user.click(within(form).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "New task" })).toHaveFocus());
  });

  it("announces an attention explanation through a live region", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      getAiSettings: vi.fn().mockResolvedValue({
        version: 1,
        enabled: true,
        providerLabel: "Local model",
        baseUrl: "http://127.0.0.1:11434/v1",
        model: "llama3.1",
        hasApiKey: false,
      }),
      getAttentionFlags: vi
        .fn()
        .mockResolvedValue([makeAttentionFlag({ id: "flag-1", recordDisplayName: "Dana Ruiz" })]),
      explainAttentionFlag: vi.fn().mockResolvedValue(makeAttentionExplanation()),
    });

    render(<App client={client} />);
    await user.click(await screen.findByRole("button", { name: "Attention" }));

    const flag = await screen.findByRole("listitem");
    await user.click(within(flag).getByRole("button", { name: "Explain Dana Ruiz" }));

    const live = await waitFor(() => {
      const region = within(flag).getByRole("status");
      expect(region).toHaveTextContent(/./);
      return region;
    });
    expect(live).toHaveAttribute("aria-live", "polite");
  });

  it("distinguishes repeated context disclosures and makes the preview reachable", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      previewContext: vi.fn().mockResolvedValue(makeContextPreview()),
    });

    render(
      <ContextDisclosure
        client={client}
        request={{ tool: "summarize_history", parentType: "contact", parentId: "contact-1" }}
        about="Dana Ruiz"
      />,
    );

    const summary = screen.getByText("See what will be sent");
    expect(summary).toHaveAccessibleName("See what will be sent for Dana Ruiz");

    await user.click(summary);

    // The preview box clips its content, so it has to be focusable and named.
    const preview = await screen.findByRole("region", { name: "Context to be sent for Dana Ruiz" });
    expect(preview).toHaveAttribute("tabindex", "0");
  });
});
