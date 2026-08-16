import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SavedView, SavedViewDefinition } from "../api/types";
import { stubClient } from "../test/stub-client";
import { SavedViews } from "./SavedViews";

const definition: SavedViewDefinition = {
  schemaVersion: 1,
  filter: { includeArchived: true },
  sort: { field: "name", direction: "descending" },
};

const savedView = (overrides: Partial<SavedView> = {}): SavedView => ({
  id: "view-1",
  name: "All companies",
  entityType: "company",
  definition,
  sortKey: 0,
  createdAt: "2026-08-16T20:00:00Z",
  updatedAt: "2026-08-16T20:00:00Z",
  version: 1,
  ...overrides,
});

describe("SavedViews", () => {
  it("applies a stored definition and exposes selected-state feedback", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    const client = stubClient({ listSavedViews: vi.fn().mockResolvedValue([savedView()]) });

    render(
      <SavedViews
        client={client}
        entityType="company"
        definition={definition}
        onApply={onApply}
      />,
    );

    const picker = await screen.findByRole("combobox", { name: "Saved company view" });
    await user.selectOptions(picker, "view-1");
    expect(onApply).toHaveBeenCalledWith(definition);
    expect(picker).toHaveValue("view-1");
    expect(screen.getByRole("status")).toHaveTextContent("All companies applied.");
  });

  it("marks an applied view as modified when current state diverges", async () => {
    const user = userEvent.setup();
    const client = stubClient({ listSavedViews: vi.fn().mockResolvedValue([savedView()]) });
    const { rerender } = render(
      <SavedViews client={client} entityType="company" definition={definition} onApply={vi.fn()} />,
    );
    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Saved company view" }),
      "view-1",
    );
    rerender(
      <SavedViews
        client={client}
        entityType="company"
        definition={{
          ...definition,
          filter: { includeArchived: false },
        }}
        onApply={vi.fn()}
      />,
    );
    expect(screen.getByText("Modified")).toBeVisible();
    expect(screen.getByRole("combobox", { name: "Saved company view" })).toHaveValue("view-1");
  });

  it("creates a named view from the exact current definition", async () => {
    const user = userEvent.setup();
    const created = savedView();
    const client = stubClient({
      listSavedViews: vi.fn().mockResolvedValueOnce([]).mockResolvedValue([created]),
      createSavedView: vi.fn().mockResolvedValue(created),
    });

    render(
      <SavedViews client={client} entityType="company" definition={definition} onApply={vi.fn()} />,
    );
    await user.click(await screen.findByRole("button", { name: "Save view" }));
    const input = screen.getByRole("textbox", { name: "View name" });
    expect(input).toHaveFocus();
    await user.type(input, "All companies");
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Save view" }));

    expect(client.createSavedView).toHaveBeenCalledWith({
      name: "All companies",
      entityType: "company",
      definition,
    });
    const picker = await screen.findByRole("combobox", { name: "Saved company view" });
    expect(picker).toHaveValue("view-1");
    expect(picker).toHaveFocus();
  });

  it("restores focus to the invoker when a dialog is cancelled with Escape", async () => {
    const user = userEvent.setup();
    render(
      <SavedViews
        client={stubClient()}
        entityType="company"
        definition={definition}
        onApply={vi.fn()}
      />,
    );
    const trigger = await screen.findByRole("button", { name: "Save view" });
    await user.click(trigger);
    expect(screen.getByRole("textbox", { name: "View name" })).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("updates, renames, and confirms deletion with optimistic versions", async () => {
    const user = userEvent.setup();
    const current = savedView();
    const renamed = savedView({ name: "Key companies", version: 2 });
    const listSavedViews = vi
      .fn()
      .mockResolvedValueOnce([current])
      .mockResolvedValueOnce([current])
      .mockResolvedValueOnce([renamed])
      .mockResolvedValueOnce([]);
    const client = stubClient({
      listSavedViews,
      updateSavedView: vi.fn().mockResolvedValueOnce(current).mockResolvedValueOnce(renamed),
      deleteSavedView: vi.fn().mockResolvedValue(undefined),
    });

    render(
      <SavedViews client={client} entityType="company" definition={definition} onApply={vi.fn()} />,
    );
    const picker = await screen.findByRole("combobox", { name: "Saved company view" });
    await user.selectOptions(picker, "view-1");
    await user.click(screen.getByRole("button", { name: "Update" }));
    expect(client.updateSavedView).toHaveBeenNthCalledWith(1, {
      savedViewId: "view-1",
      expectedVersion: 1,
      name: "All companies",
      definition,
    });

    await user.click(screen.getByRole("button", { name: "Rename" }));
    const input = screen.getByRole("textbox", { name: "View name" });
    await user.clear(input);
    await user.type(input, "Key companies");
    await user.click(screen.getByRole("button", { name: "Rename view" }));
    expect(client.updateSavedView).toHaveBeenNthCalledWith(2, {
      savedViewId: "view-1",
      expectedVersion: 1,
      name: "Key companies",
      definition,
    });

    await user.click(await screen.findByRole("button", { name: "Delete" }));
    expect(screen.getByRole("dialog", { name: "Delete Key companies?" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Delete view" }));
    expect(client.deleteSavedView).toHaveBeenCalledWith({
      savedViewId: "view-1",
      expectedVersion: 2,
    });
    expect(await screen.findByRole("combobox", { name: "Saved company view" })).toHaveValue("");
  });

  it("reports unsupported stored definitions without rewriting them", async () => {
    const client = stubClient({
      listSavedViews: vi.fn().mockRejectedValue({
        kind: "invalid_stored_data",
        message: "saved view schema version 99 is unsupported",
      }),
    });

    render(
      <SavedViews client={client} entityType="company" definition={definition} onApply={vi.fn()} />,
    );
    expect(
      await screen.findByText("A saved view has an unsupported or damaged definition. It was not changed."),
    ).toBeVisible();
    expect(client.updateSavedView).not.toHaveBeenCalled();
  });
});
