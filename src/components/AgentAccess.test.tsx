import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { stubClient } from "../test/stub-client";
import { AgentAccess } from "./AgentAccess";

describe("personal agent setup", () => {
  it("copies exact paths as JSON and makes write access an explicit choice", async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    const helper = 'C:\\Program Files\\ContractorCRM\\contractorcrm-mcp.exe';
    const database = 'C:\\Users\\Pat O\'Brien\\CRM "work"\\contractorcrm.sqlite3';
    render(<AgentAccess client={stubClient({
      getAgentHelperPath: vi.fn().mockResolvedValue(helper),
      getDatabaseInfo: vi.fn().mockResolvedValue({ databasePath: database }),
    })} />);

    const copy = screen.getByRole("button", { name: "Copy configuration" });
    await waitFor(() => expect(copy).toBeEnabled());
    expect(screen.getByRole("combobox", { name: "Agent access" })).toHaveValue("read");
    await user.click(copy);
    expect(JSON.parse(writeText.mock.calls[0]![0])).toEqual({
      mcpServers: { contractorcrm: { command: helper, args: ["--database", database] } },
    });
    expect(screen.getByRole("status")).toHaveTextContent("Configuration copied.");

    await user.selectOptions(screen.getByRole("combobox"), "write");
    await user.click(copy);
    expect(JSON.parse(writeText.mock.calls[1]![0]).mcpServers.contractorcrm.args)
      .toEqual(["--database", database, "--read-write"]);
    expect(screen.getByText(/only applied AI proposals have undo/)).toBeVisible();
    await user.click(screen.getByText("Connection details"));
    expect(screen.getByLabelText("PowerShell command"))
      .toHaveValue(`& '${helper}' --database '${database.replaceAll("'", "''")}' --read-write`);
  });

  it("keeps copy disabled until both real paths resolve", async () => {
    let resolveHelper!: (value: string) => void;
    render(<AgentAccess client={stubClient({
      getAgentHelperPath: vi.fn(() => new Promise<string>((resolve) => { resolveHelper = resolve; })),
    })} />);
    expect(screen.getByRole("button", { name: "Copy configuration" })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "MCP configuration (JSON)" })).toHaveValue("");
    resolveHelper("/Applications/ContractorCRM.app/Contents/MacOS/contractorcrm-mcp");
    await waitFor(() => expect(screen.getByRole("button", { name: "Copy configuration" })).toBeEnabled());
  });

  it("reports missing paths and retries without copying placeholder commands", async () => {
    const user = userEvent.setup();
    const getAgentHelperPath = vi.fn().mockRejectedValueOnce(new Error("Missing helper"))
      .mockResolvedValue("/Applications/CRM/contractorcrm-mcp");
    render(<AgentAccess client={stubClient({ getAgentHelperPath })} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not find the agent helper");
    expect(screen.getByRole("button", { name: "Copy configuration" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Copy configuration" })).toBeEnabled());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("selects the config for keyboard copying if clipboard access fails", async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(new Error("Unavailable"));
    render(<AgentAccess client={stubClient()} />);
    const copy = screen.getByRole("button", { name: "Copy configuration" });
    await waitFor(() => expect(copy).toBeEnabled());
    await user.click(copy);
    const config = screen.getByRole("textbox", { name: "MCP configuration (JSON)" }) as HTMLTextAreaElement;
    expect(config).toHaveFocus();
    expect(config.selectionStart).toBe(0);
    expect(config.selectionEnd).toBe(config.value.length);
    expect(screen.getByRole("status")).toHaveTextContent("copy it with your keyboard");
  });
});
