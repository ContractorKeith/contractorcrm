import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SearchResult } from "../api/types";
import { stubClient } from "../test/stub-client";
import { GlobalSearch } from "./GlobalSearch";

const result = (overrides: Partial<SearchResult> = {}): SearchResult => ({
  entityType: "contact",
  entityId: "contact-1",
  title: "Dana Ruiz",
  parentType: null,
  parentId: null,
  ...overrides,
});

describe("GlobalSearch", () => {
  beforeEach(() => {
    Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" });
  });

  it("opens with Command-K, loads recents then favorites, and restores focus", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listRecentRecords: vi.fn().mockResolvedValue([result()]),
      listFavoriteContacts: vi
        .fn()
        .mockResolvedValue([result({ entityId: "contact-2", title: "Avery Cole" })]),
    });
    render(<GlobalSearch client={client} onOpenResult={vi.fn()} />);
    const trigger = screen.getByRole("button", { name: /Search/ });
    trigger.focus();

    await user.keyboard("{Meta>}k{/Meta}");

    const dialog = await screen.findByRole("dialog", { name: "Search ContractorCRM" });
    expect(screen.getByRole("combobox", { name: "Search records" })).toHaveFocus();
    expect(within(dialog).getByRole("group", { name: "Recent records" })).toHaveTextContent(
      "Dana Ruiz",
    );
    expect(within(dialog).getByRole("group", { name: "Favorite contacts" })).toHaveTextContent(
      "Avery Cole",
    );
    expect(screen.getByRole("status")).toHaveTextContent("2 suggestions");

    await user.keyboard("{Escape}");
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it("uses Control-K off macOS and does not hijack unrelated editable controls", async () => {
    Object.defineProperty(navigator, "platform", { configurable: true, value: "Win32" });
    const user = userEvent.setup();
    render(
      <>
        <input aria-label="Unrelated field" />
        <GlobalSearch client={stubClient()} onOpenResult={vi.fn()} />
      </>,
    );

    const unrelated = screen.getByRole("textbox", { name: "Unrelated field" });
    await user.click(unrelated);
    await user.keyboard("{Control>}k{/Control}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(unrelated).toHaveFocus();

    await user.click(screen.getByRole("button", { name: /Search/ }));
    await user.keyboard("{Escape}");
    await user.keyboard("{Control>}k{/Control}");
    expect(await screen.findByRole("dialog")).toBeVisible();
  });

  it("does not hijack a nested contenteditable target and traps focus inside the modal", async () => {
    const user = userEvent.setup();
    render(
      <>
        <div contentEditable aria-label="Editable notes">
          <span data-testid="editable-child">Notes</span>
        </div>
        <GlobalSearch client={stubClient()} onOpenResult={vi.fn()} />
      </>,
    );

    screen.getByTestId("editable-child").focus();
    fireEvent.keyDown(screen.getByTestId("editable-child"), { key: "k", metaKey: true });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Search/ }));
    const input = screen.getByRole("combobox", { name: "Search records" });
    const close = screen.getByRole("button", { name: "Close" });
    input.focus();
    await user.keyboard("{Tab}");
    expect(close).toHaveFocus();
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(input).toHaveFocus();
  });

  it("replaces suggestions with typed FTS results and ignores stale responses", async () => {
    const user = userEvent.setup();
    let resolveFirst: ((value: SearchResult[]) => void) | undefined;
    const client = stubClient({
      searchRecords: vi
        .fn()
        .mockImplementationOnce(() => new Promise((resolve) => (resolveFirst = resolve)))
        .mockResolvedValueOnce([
          result({ entityType: "company", entityId: "c-2", title: "Beacon Builders" }),
        ]),
    });
    render(<GlobalSearch client={client} onOpenResult={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: /Search/ }));
    const input = screen.getByRole("combobox", { name: "Search records" });

    await user.type(input, "b");
    await user.type(input, "e");
    resolveFirst?.([result({ title: "Bad stale result" })]);

    expect(await screen.findByRole("option", { name: /Beacon Builders/ })).toBeVisible();
    expect(screen.queryByText("Bad stale result")).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("1 result");
  });

  it("supports arrows, Home, End, Enter, pointer selection, and stable ARIA state", async () => {
    const user = userEvent.setup();
    const onOpenResult = vi.fn().mockResolvedValue(true);
    const results = [
      result(),
      result({ entityType: "company", entityId: "company-1", title: "Beacon Builders" }),
      result({ entityType: "opportunity", entityId: "opp-1", title: "Kitchen remodel" }),
    ];
    render(
      <GlobalSearch
        client={stubClient({ searchRecords: vi.fn().mockResolvedValue(results) })}
        onOpenResult={onOpenResult}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Search/ }));
    const input = screen.getByRole("combobox", { name: "Search records" });
    await user.type(input, "build");
    await screen.findAllByRole("option");

    const firstActiveId = input.getAttribute("aria-activedescendant");
    expect(firstActiveId).toContain("contact-1");
    await user.keyboard("{End}");
    expect(input.getAttribute("aria-activedescendant")).toContain("opp-1");
    await user.keyboard("{Home}{ArrowDown}{ArrowUp}{ArrowDown}{Enter}");
    expect(onOpenResult).toHaveBeenLastCalledWith(results[1]);

    await user.click(screen.getByRole("button", { name: /Search/ }));
    await user.type(screen.getByRole("combobox"), "build");
    const opportunity = await screen.findByRole("option", { name: /Kitchen remodel/ });
    await user.click(opportunity);
    expect(onOpenResult).toHaveBeenLastCalledWith(results[2]);
  });

  it("renders explicit empty, no-result, and unavailable states", async () => {
    const user = userEvent.setup();
    const searchRecords = vi
      .fn()
      .mockResolvedValueOnce([])
      .mockRejectedValueOnce(new Error("offline"));
    render(<GlobalSearch client={stubClient({ searchRecords })} onOpenResult={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: /Search/ }));
    expect(await screen.findByText("No recent records or favorite contacts.")).toBeVisible();

    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "missing" } });
    expect(await screen.findByText("No matching records.")).toBeVisible();
    fireEvent.change(input, { target: { value: "broken" } });
    expect(await screen.findByText("Search is unavailable. Try again.")).toBeVisible();
  });
});
