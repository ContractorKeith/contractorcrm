import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { makeContact, makeTask, stubClient } from "../test/stub-client";

// Open the Tasks tab from the app shell.
async function openTasks(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Tasks" }));
}

describe("tasks view", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders task rows and flags overdue ones with a label, not hue alone", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([makeContact()]),
      listTasks: vi.fn().mockResolvedValue([
        makeTask({ id: "t1", title: "Call inspector", dueAt: "2020-01-01T00:00:00Z" }),
        makeTask({ id: "t2", title: "Order pickets", dueAt: "2099-01-01T00:00:00Z", priority: "high" }),
      ]),
    });

    render(<App client={client} />);
    await openTasks(user);

    const table = await screen.findByRole("table", { name: "Task list" });
    const rows = within(table).getAllByRole("row");
    expect(within(rows[1]!).getByText("Call inspector")).toBeVisible();
    expect(within(rows[1]!).getByText("Contact — Dana Ruiz")).toBeVisible();
    expect(within(rows[1]!).getByText("Overdue")).toBeVisible();
    expect(within(rows[2]!).getByText("Order pickets")).toBeVisible();
    expect(within(rows[2]!).getByText("High")).toBeVisible();
    expect(within(rows[2]!).queryByText("Overdue")).not.toBeInTheDocument();
  });

  it("defaults to open tasks and switches the filter request", async () => {
    const user = userEvent.setup();
    const client = stubClient();

    render(<App client={client} />);
    await openTasks(user);

    expect(client.listTasks).toHaveBeenCalledWith({ status: "open" });

    await user.click(screen.getByRole("button", { name: "Overdue" }));
    expect(client.listTasks).toHaveBeenCalledWith({ overdueOnly: true });

    await user.click(screen.getByRole("button", { name: "All" }));
    expect(client.listTasks).toHaveBeenCalledWith({});
  });

  it("creates a task linked to a contact through the parent picker", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([makeContact()]),
      createTask: vi.fn().mockResolvedValue(makeTask()),
    });

    render(<App client={client} />);
    await openTasks(user);
    await user.click(screen.getByRole("button", { name: "New task" }));

    await user.type(screen.getByLabelText("Title"), "Call inspector");
    await user.selectOptions(await screen.findByLabelText("Linked to"), "contact:contact-1");
    await user.selectOptions(screen.getByLabelText("Priority"), "high");
    await user.click(screen.getByRole("button", { name: "Create task" }));

    expect(client.createTask).toHaveBeenCalledWith({
      title: "Call inspector",
      body: null,
      parentType: "contact",
      parentId: "contact-1",
      dueAt: null,
      remindAt: null,
      priority: "high",
    });
  });

  it("completes a task with logActivity when the log checkbox is on", async () => {
    const user = userEvent.setup();
    const task = makeTask({ id: "t1", version: 5 });
    const client = stubClient({
      listContacts: vi.fn().mockResolvedValue([makeContact()]),
      listTasks: vi.fn().mockResolvedValue([task]),
      completeTask: vi.fn().mockResolvedValue({ ...task, status: "done", version: 6 }),
    });

    render(<App client={client} />);
    await openTasks(user);
    await user.click(await screen.findByText("Follow up with Dana"));

    await user.click(screen.getByLabelText("Log to timeline"));
    await user.click(screen.getByRole("button", { name: "Complete" }));

    expect(client.completeTask).toHaveBeenCalledWith({
      taskId: "t1",
      expectedVersion: 5,
      logActivity: true,
    });
  });

  it("disables the log checkbox for a task with no parent", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listTasks: vi
        .fn()
        .mockResolvedValue([makeTask({ id: "t1", parentType: null, parentId: null })]),
    });

    render(<App client={client} />);
    await openTasks(user);
    await user.click(await screen.findByText("Follow up with Dana"));

    expect(screen.getByLabelText("Log to timeline")).toBeDisabled();
  });

  it("reopens a done task and deletes only after a confirm", async () => {
    const user = userEvent.setup();
    const task = makeTask({ id: "t1", status: "done", version: 2 });
    const client = stubClient({
      listTasks: vi.fn().mockResolvedValue([task]),
      reopenTask: vi.fn().mockResolvedValue({ ...task, status: "open", version: 3 }),
      deleteTask: vi.fn().mockResolvedValue(undefined),
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App client={client} />);
    await openTasks(user);
    await user.click(await screen.findByText("Follow up with Dana"));

    await user.click(screen.getByRole("button", { name: "Reopen" }));
    expect(client.reopenTask).toHaveBeenCalledWith({ taskId: "t1", expectedVersion: 2 });

    await user.click(await screen.findByText("Follow up with Dana"));
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(client.deleteTask).toHaveBeenCalledWith({ taskId: "t1", expectedVersion: 2 });
  });
});
