import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { makeAttachment, stubClient } from "../test/stub-client";
import { RecordAttachments } from "./RecordAttachments";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn() }));

const openDialog = vi.mocked(open);
const openWithOs = vi.mocked(openPath);

beforeEach(() => {
  vi.clearAllMocks();
});

describe("RecordAttachments list", () => {
  it("renders managed files with a human-readable size", async () => {
    const client = stubClient({
      listAttachments: vi
        .fn()
        .mockResolvedValue([makeAttachment({ fileName: "site-plan.pdf", sizeBytes: 2048 })]),
    });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    expect(await screen.findByText("site-plan.pdf")).toBeInTheDocument();
    expect(screen.getByText("2.0 KB")).toBeInTheDocument();
    expect(client.listAttachments).toHaveBeenCalledWith("contact", "contact-1");
  });

  it("shows a quiet empty state when nothing is attached", async () => {
    const client = stubClient();
    render(<RecordAttachments client={client} parentType="opportunity" parentId="opp-1" />);

    expect(await screen.findByText("No attachments yet.")).toBeInTheDocument();
  });
});

describe("RecordAttachments add", () => {
  it("attaches every picked file and reloads the list", async () => {
    const user = userEvent.setup();
    openDialog.mockResolvedValue(["/tmp/plan.pdf", "/tmp/photo.jpg"]);
    const listAttachments = vi
      .fn()
      .mockResolvedValueOnce([])
      .mockResolvedValue([
        makeAttachment({ id: "a1", fileName: "plan.pdf" }),
        makeAttachment({ id: "a2", fileName: "photo.jpg" }),
      ]);
    const client = stubClient({ listAttachments, addAttachment: vi.fn().mockResolvedValue(makeAttachment()) });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    await user.click(await screen.findByRole("button", { name: "Add file…" }));

    await waitFor(() => expect(client.addAttachment).toHaveBeenCalledTimes(2));
    expect(client.addAttachment).toHaveBeenNthCalledWith(1, {
      parentType: "contact",
      parentId: "contact-1",
      sourcePath: "/tmp/plan.pdf",
    });
    expect(client.addAttachment).toHaveBeenNthCalledWith(2, {
      parentType: "contact",
      parentId: "contact-1",
      sourcePath: "/tmp/photo.jpg",
    });
    expect(await screen.findByText("photo.jpg")).toBeInTheDocument();
  });

  it("surfaces the core message when a file is over the size cap", async () => {
    const user = userEvent.setup();
    openDialog.mockResolvedValue(["/tmp/huge.zip"]);
    const client = stubClient({
      addAttachment: vi.fn().mockRejectedValue({
        kind: "validation_failed",
        code: "file_too_large",
        field: "sourcePath",
        message: "That file is larger than the 100 MB attachment limit.",
      }),
    });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    await user.click(await screen.findByRole("button", { name: "Add file…" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "That file is larger than the 100 MB attachment limit.",
    );
  });
});

describe("RecordAttachments open", () => {
  it("opens the managed copy at the path the core reports", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listAttachments: vi.fn().mockResolvedValue([makeAttachment({ fileName: "site-plan.pdf" })]),
      attachmentPath: vi
        .fn()
        .mockResolvedValue({ path: "/data/attachments/attachment-1/site-plan.pdf", exists: true }),
    });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    await user.click(await screen.findByRole("button", { name: "Open site-plan.pdf" }));

    await waitFor(() =>
      expect(openWithOs).toHaveBeenCalledWith("/data/attachments/attachment-1/site-plan.pdf"),
    );
  });

  it("reports a missing managed file instead of opening it", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listAttachments: vi.fn().mockResolvedValue([makeAttachment({ fileName: "site-plan.pdf" })]),
      attachmentPath: vi.fn().mockResolvedValue({ path: "/data/gone.pdf", exists: false }),
    });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    await user.click(await screen.findByRole("button", { name: "Open site-plan.pdf" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("site-plan.pdf is missing");
    expect(openWithOs).not.toHaveBeenCalled();
  });
});

describe("RecordAttachments remove", () => {
  it("confirms in place before removing and passes the expected version", async () => {
    const user = userEvent.setup();
    const listAttachments = vi
      .fn()
      .mockResolvedValueOnce([makeAttachment({ fileName: "site-plan.pdf", version: 3 })])
      .mockResolvedValue([]);
    const client = stubClient({ listAttachments });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    await user.click(await screen.findByRole("button", { name: "Remove site-plan.pdf" }));
    expect(client.removeAttachment).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Confirm removing site-plan.pdf" }));

    expect(client.removeAttachment).toHaveBeenCalledWith({
      attachmentId: "attachment-1",
      expectedVersion: 3,
    });
    expect(await screen.findByText("No attachments yet.")).toBeInTheDocument();
  });

  it("cancels the confirm without touching the core", async () => {
    const user = userEvent.setup();
    const client = stubClient({
      listAttachments: vi.fn().mockResolvedValue([makeAttachment({ fileName: "site-plan.pdf" })]),
    });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    await user.click(await screen.findByRole("button", { name: "Remove site-plan.pdf" }));
    await user.click(screen.getByRole("button", { name: "Keep site-plan.pdf" }));

    expect(client.removeAttachment).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Remove site-plan.pdf" })).toBeInTheDocument();
  });

  it("silently reloads when the attachment changed elsewhere", async () => {
    const user = userEvent.setup();
    const listAttachments = vi
      .fn()
      .mockResolvedValueOnce([makeAttachment({ fileName: "site-plan.pdf" })])
      .mockResolvedValue([makeAttachment({ id: "a2", fileName: "revised-plan.pdf" })]);
    const client = stubClient({
      listAttachments,
      removeAttachment: vi.fn().mockRejectedValue({
        kind: "version_conflict",
        message: "The attachment changed.",
        resource: "attachment",
        recordId: "attachment-1",
        expectedVersion: 1,
        currentVersion: 2,
      }),
    });
    render(<RecordAttachments client={client} parentType="contact" parentId="contact-1" />);

    await user.click(await screen.findByRole("button", { name: "Remove site-plan.pdf" }));
    await user.click(screen.getByRole("button", { name: "Confirm removing site-plan.pdf" }));

    expect(await screen.findByText("revised-plan.pdf")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
