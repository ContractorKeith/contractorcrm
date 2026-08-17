import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { stubClient } from "../test/stub-client";
import { MetadataDefinitionsDialog } from "./MetadataDefinitionsDialog";

const tag = { id: "tag-1", label: "Priority", colorRole: null, archivedAt: null, createdAt: "", updatedAt: "", version: 3 } as const;
const selectDefinition = {
  id: "field-1", entityType: "contact" as const, label: "Project class", fieldType: "select" as const,
  sortKey: 0, archivedAt: null, createdAt: "", updatedAt: "", version: 4,
  options: [{ id: "option-1", definitionId: "field-1", label: "A", sortKey: 0 }],
};

describe("MetadataDefinitionsDialog", () => {
  it("creates a tag and closes with Escape", async () => {
    const user = userEvent.setup();
    const client = stubClient({ createTag: vi.fn().mockResolvedValue(tag), listTags: vi.fn().mockResolvedValue([]) });
    const close = vi.fn();
    render(<MetadataDefinitionsDialog client={client} entityType="contact" onClose={close} />);
    const input = await screen.findByRole("textbox", { name: "Tag label" });
    expect(input).toHaveFocus();
    await user.type(input, "Priority");
    await user.click(screen.getByRole("button", { name: "Create tag" }));
    expect(client.createTag).toHaveBeenCalledWith({ label: "Priority", colorRole: null });
    expect(screen.getByRole("status")).toHaveTextContent("Tag created.");
    await user.keyboard("{Escape}");
    expect(close).toHaveBeenCalled();
  });

  it("updates a tag and archives only after confirmation", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listTags: vi.fn().mockResolvedValue([tag]),
      updateTag: vi.fn().mockResolvedValue({ ...tag, label: "Urgent", version: 4 }),
      archiveTag: vi.fn().mockResolvedValue({ ...tag, archivedAt: "2026-08-17", version: 4 }),
    });
    render(<MetadataDefinitionsDialog client={client} entityType="contact" onClose={vi.fn()} />);
    await user.click(await screen.findByRole("button", { name: "Edit Priority" }));
    const label = screen.getByRole("textbox", { name: "Tag label" });
    await user.clear(label);
    await user.type(label, "Urgent");
    await user.selectOptions(screen.getByRole("combobox", { name: "Color role" }), "attention");
    await user.click(screen.getByRole("button", { name: "Update tag" }));
    expect(client.updateTag).toHaveBeenCalledWith(expect.objectContaining({ tagId: "tag-1", expectedVersion: 3, label: "Urgent", colorRole: "attention" }));

    await user.click(screen.getByRole("button", { name: "Archive Priority" }));
    expect(screen.getByRole("alertdialog", { name: "Archive Priority?" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    expect(client.archiveTag).not.toHaveBeenCalled();
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(screen.getByRole("button", { name: "Archive" })).toHaveFocus();
    await user.keyboard("{Tab}");
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: "Archive Priority" })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Archive Priority" }));
    await user.click(screen.getByRole("button", { name: "Archive" }));
    expect(client.archiveTag).toHaveBeenCalledWith({ tagId: "tag-1", expectedVersion: 3 });
  });

  it("edits select options without changing the field type", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listCustomFieldDefs: vi.fn().mockResolvedValue([selectDefinition]),
      updateCustomFieldDef: vi.fn().mockResolvedValue(selectDefinition),
    });
    render(<MetadataDefinitionsDialog client={client} entityType="contact" onClose={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Custom fields" }));
    await user.click(await screen.findByRole("button", { name: "Edit Project class" }));
    expect(screen.getByRole("combobox", { name: "Field type" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Add option" }));
    await user.type(screen.getByRole("textbox", { name: "Option 2" }), "B");
    await user.click(screen.getByRole("button", { name: "Update field" }));
    expect(client.updateCustomFieldDef).toHaveBeenCalledWith(expect.objectContaining({
      definitionId: "field-1",
      expectedVersion: 4,
      options: [{ id: "option-1", label: "A" }, { label: "B" }],
    }));
  });
});
