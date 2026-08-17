import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { stubClient } from "../test/stub-client";
import { RecordMetadata } from "./RecordMetadata";

describe("RecordMetadata", () => it("edits every typed value, removes tags, and saves the replacement", async () => {
  const user = userEvent.setup(); const client = stubClient({ listTags: vi.fn().mockResolvedValue([{ id: "t", label: "Priority", colorRole: null, archivedAt: null, createdAt: "", updatedAt: "", version: 1 }]), listCustomFieldDefs: vi.fn().mockResolvedValue([
    { id: "text", label: "Notes", fieldType: "text", entityType: "contact", sortKey: 0, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [] }, { id: "num", label: "Budget", fieldType: "number", entityType: "contact", sortKey: 1, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [] }, { id: "date", label: "Due", fieldType: "date", entityType: "contact", sortKey: 2, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [] }, { id: "sel", label: "Class", fieldType: "select", entityType: "contact", sortKey: 3, archivedAt: null, createdAt: "", updatedAt: "", version: 1, options: [{ id: "o", definitionId: "sel", label: "A", sortKey: 0 }] },
  ]), getRecordMetadata: vi.fn().mockResolvedValue({ tagIds: ["t"], values: [] }), setRecordMetadata: vi.fn().mockResolvedValue({ tagIds: [], values: [] }) });
  render(<form data-testid="outer-form"><RecordMetadata client={client} entityType="contact" recordId="c" expectedVersion={1} /></form>);
  await screen.findByRole("button", { name: "Remove Priority tag" }); await user.click(screen.getByRole("button", { name: "Remove Priority tag" }));
  await user.type(screen.getByRole("textbox", { name: "Notes" }), "hello"); await user.type(screen.getByRole("spinbutton", { name: "Budget" }), "12"); await user.type(screen.getByLabelText("Due"), "2026-09-01"); await user.selectOptions(screen.getByRole("combobox", { name: "Class" }), "o"); await user.click(screen.getByRole("button", { name: "Save metadata" }));
  expect(client.setRecordMetadata).toHaveBeenCalledWith(expect.objectContaining({ tagIds: [], values: expect.arrayContaining([expect.objectContaining({ definitionId: "text", textValue: "hello" }), expect.objectContaining({ definitionId: "num", numberValue: 12 }), expect.objectContaining({ definitionId: "date", dateValue: "2026-09-01" }), expect.objectContaining({ definitionId: "sel", optionId: "o" })]) })); expect(screen.getByRole("status")).toHaveTextContent("Metadata saved.");
}));

describe("RecordMetadata definition management", () => it("opens management and restores focus after Close", async () => {
  const user = userEvent.setup();
  const client = stubClient({ getRecordMetadata: vi.fn().mockResolvedValue({ tagIds: [], values: [] }) });
  render(<form data-testid="outer-form"><RecordMetadata client={client} entityType="contact" recordId="c" expectedVersion={1} /></form>);
  const manage = await screen.findByRole("button", { name: "Manage tags and fields" });
  await user.click(manage);
  expect(screen.getByRole("dialog", { name: "Manage tags and custom fields" })).toBeInTheDocument();
  expect(screen.getByTestId("outer-form").querySelector("form")).toBeNull();
  await user.click(screen.getByRole("button", { name: "Close" }));
  expect(manage).toHaveFocus();
}));

describe("RecordMetadata archived lifecycle", () => it("retains an archived value until the user explicitly clears it", async () => {
  const user = userEvent.setup();
  const archived = { id: "old", label: "Legacy note", fieldType: "text" as const, entityType: "contact" as const, sortKey: 0, archivedAt: "2026-08-17", createdAt: "", updatedAt: "", version: 2, options: [] };
  const value = { id: "value", definitionId: "old", entityType: "contact" as const, recordId: "c", textValue: "Keep me", numberValue: null, dateValue: null, optionId: null, createdAt: "", updatedAt: "" };
  const client = stubClient({ listCustomFieldDefs: vi.fn().mockResolvedValue([archived]), getRecordMetadata: vi.fn().mockResolvedValue({ tagIds: [], values: [value] }), setRecordMetadata: vi.fn().mockResolvedValue({ tagIds: [], values: [] }) });
  render(<RecordMetadata client={client} entityType="contact" recordId="c" expectedVersion={2} />);
  expect(await screen.findByRole("textbox", { name: "Legacy note" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Clear archived Legacy note" }));
  await user.click(screen.getByRole("button", { name: "Save metadata" }));
  expect(client.setRecordMetadata).toHaveBeenCalledWith(expect.objectContaining({ values: [] }));
}));
