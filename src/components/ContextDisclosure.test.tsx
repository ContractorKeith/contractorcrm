import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { makeContextPreview, stubClient } from "../test/stub-client";
import { ContextDisclosure } from "./ContextDisclosure";

describe("ContextDisclosure", () => {
  it("fetches nothing until it is opened, then shows the exact projection", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      previewContext: vi.fn().mockResolvedValue(makeContextPreview()),
    });

    render(
      <ContextDisclosure
        client={client}
        request={{ tool: "summarize_history", parentType: "contact", parentId: "contact-1" }}
      />,
    );

    expect(client.previewContext).not.toHaveBeenCalled();
    await user.click(screen.getByText("See what will be sent"));

    expect(client.previewContext).toHaveBeenCalledWith({
      tool: "summarize_history",
      parentType: "contact",
      parentId: "contact-1",
    });
    expect(
      await screen.findByText(/Walked the back fence line/, { exact: false }),
    ).toBeVisible();
    // The disclosure list names every record whose data would be sent.
    expect(screen.getByText("Dana Ruiz")).toBeVisible();
  });

  it("reports a failed preview instead of pretending nothing would be sent", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      previewContext: vi.fn().mockRejectedValue({
        kind: "not_found",
        message: "contact contact-9 was not found",
        resource: "contact",
        recordId: "contact-9",
      }),
    });

    render(
      <ContextDisclosure
        client={client}
        request={{ tool: "summarize_history", parentType: "contact", parentId: "contact-9" }}
      />,
    );
    await user.click(screen.getByText("See what will be sent"));

    expect(await screen.findByRole("alert")).toHaveTextContent("contact contact-9 was not found");
  });
});
