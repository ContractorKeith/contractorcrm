import { describe, expect, it } from "vitest";

import { formatLocalDateTime } from "./date-format";

describe("formatLocalDateTime", () => {
  it("formats a UTC instant as a friendly date and time in the requested locale and zone", () => {
    expect(formatLocalDateTime("2026-08-12T15:00:00Z", "en-US", "UTC")).toBe(
      "Aug 12, 2026, 3:00 PM",
    );
  });

  it("keeps invalid projection values from leaking as Invalid Date", () => {
    expect(formatLocalDateTime("not-a-date", "en-US", "UTC")).toBe("—");
  });
});
