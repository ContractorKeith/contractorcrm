import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SavedViewFilters } from "./SavedViewFilters";

describe("SavedViewFilters", () => it("emits finite v2 tag and text predicates", async () => {
  const user = userEvent.setup(); const onChange = vi.fn(); render(<SavedViewFilters entityType="contact" tags={[{ id: "tag", label: "Residential", colorRole: null, archivedAt: null, createdAt: "", updatedAt: "", version: 1 }]} definitions={[{ id: "field", label: "Area", fieldType: "text", entityType: "contact", sortKey: 0, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [] }]} definition={{ schemaVersion: 2, filter: { includeArchived: false, tagIdsAll: [], customFields: [] }, sort: { field: "displayName", direction: "ascending" } }} onChange={onChange} />);
  await user.selectOptions(screen.getByRole("listbox", { name: "Tags (all must match)" }), "tag"); fireEvent.change(screen.getByRole("textbox", { name: "Area filter" }), { target: { value: "residential" } }); expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ filter: expect.objectContaining({ customFields: [expect.objectContaining({ definitionId: "field", fieldType: "text", operator: "contains", value: "residential" })] }) }));
}));

describe("SavedViewFilters typed controls", () => it("renders applied predicates and emits finite select/date operators", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  const definition = {
    schemaVersion: 2 as const,
    filter: { includeArchived: false, tagIdsAll: [], customFields: [{ definitionId: "date", fieldType: "date" as const, operator: "after" as const, value: "2026-08-01" }] },
    sort: { field: "displayName" as const, direction: "ascending" as const },
  };
  render(<SavedViewFilters entityType="contact" tags={[]} definitions={[
    { id: "date", label: "Inspection", fieldType: "date", entityType: "contact", sortKey: 0, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [] },
    { id: "class", label: "Class", fieldType: "select", entityType: "contact", sortKey: 1, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [{ id: "a", definitionId: "class", label: "A", sortKey: 0 }] },
  ]} definition={definition} onChange={onChange} />);
  expect(screen.getByLabelText("Inspection filter")).toHaveValue("2026-08-01");
  expect(screen.getByLabelText("Inspection operator")).toHaveValue("after");
  await user.selectOptions(screen.getByRole("combobox", { name: "Class filter" }), "a");
  expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ filter: expect.objectContaining({ customFields: expect.arrayContaining([expect.objectContaining({ definitionId: "class", fieldType: "select", operator: "is", value: "a" })]) }) }));
}));
