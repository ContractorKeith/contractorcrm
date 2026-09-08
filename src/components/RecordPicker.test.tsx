import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { RecordPicker, type RecordOption } from "./RecordPicker";

const options: RecordOption[] = [
  { value: "one", label: "Contact — Alex Rivera" },
  { value: "two", label: "Company — Rivera Construction" },
  { value: "old", label: "Old client (archived)", archived: true },
];

function Form({ initial = "", onSave = vi.fn() }: { initial?: string; onSave?: (id: string) => void }) {
  const [value, setValue] = useState(initial);
  return <form onSubmit={(event) => { event.preventDefault(); onSave(value); }}>
    <RecordPicker label="Linked to" value={value} options={options} onChange={setValue} />
    <button type="submit">Save</button>
  </form>;
}

describe("record linking", () => {
  it("filters across words and selects by keyboard without submitting the form", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn();
    render(<Form onSave={onSave} />);
    const input = screen.getByRole("combobox", { name: "Linked to" });
    await user.type(input, "rivera");
    expect(screen.getAllByRole("option")).toHaveLength(2);
    await user.keyboard("{ArrowDown}{Enter}");
    expect(input).toHaveValue("Company — Rivera Construction");
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute("aria-expanded", "false");
    expect(onSave).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith("two");
  });

  it("preserves the current identity when a search is abandoned and clears explicitly", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn();
    render(<Form initial="one" onSave={onSave} />);
    const input = screen.getByRole("combobox");
    await user.click(input);
    await user.clear(input);
    await user.type(input, "missing");
    expect(screen.getByRole("status")).toHaveTextContent("No matching records");
    await user.keyboard("{Escape}");
    expect(input).toHaveValue("Contact — Alex Rivera");
    await user.click(screen.getByRole("button", { name: "Clear Linked to" }));
    expect(input).toHaveValue("");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(onSave).toHaveBeenCalledWith("");
  });

  it("supports pointer selection without reopening the popup", async () => {
    const user = userEvent.setup();
    render(<Form />);
    const input = screen.getByRole("combobox");
    await user.type(input, "alex");
    await user.click(screen.getByRole("option", { name: "Contact — Alex Rivera" }));
    expect(input).toHaveValue("Contact — Alex Rivera");
    expect(input).toHaveAttribute("aria-expanded", "false");
  });

  it("retains archived links but offers only active records for new links", async () => {
    const user = userEvent.setup();
    render(<Form initial="old" />);
    const input = screen.getByRole("combobox");
    expect(input).toHaveValue("Old client (archived)");
    await user.click(input);
    expect(screen.queryByRole("option", { name: /archived/ })).toBeNull();
    await user.tab();
    expect(input).toHaveValue("Old client (archived)");
  });

  it("bounds a large list while keeping later records findable", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<RecordPicker label="Contact" value="" onChange={onChange}
      options={Array.from({ length: 10000 }, (_, index) => ({ value: String(index), label: `Client ${index}` }))} />);
    const input = screen.getByRole("combobox");
    await user.click(input);
    expect(screen.getAllByRole("option")).toHaveLength(20);
    expect(screen.getByRole("status")).toHaveTextContent("Showing 20 of 10000");
    await user.type(input, "Client 9999");
    await user.keyboard("{Enter}");
    expect(onChange).toHaveBeenCalledWith("9999");
  });
});
