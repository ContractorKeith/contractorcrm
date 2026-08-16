// Stored timestamps are UTC instants. Render them in the user's browser locale
// and local time zone so a projection reads like a date, not a wire value.
// Optional arguments keep the formatter deterministic in tests.
export function formatLocalDateTime(iso: string, locale?: string, timeZone?: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";

  return new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
    ...(timeZone ? { timeZone } : {}),
  }).format(date);
}
