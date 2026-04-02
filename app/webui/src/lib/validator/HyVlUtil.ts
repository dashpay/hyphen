import { tr } from "@repo/hyphen-i18n";

/** Formats a validator history count for panel metadata. */
export function HyVlFormatHistoryCount(count: number): string {
  return tr("{count, plural, one {# entry} other {# entries}}", { count });
}

/** Formats one validator endpoint entry for table and detail display. */
export function HyVlFormatEndpoint(entry: unknown): string {
  if (typeof entry === "string") {
    return entry.length > 0 ? entry : tr("No entries");
  }

  if (Array.isArray(entry)) {
    const values = entry
      .map((value): string | null => (typeof value === "string" && value.length > 0 ? value : null))
      .filter((value): value is string => value != null);
    return values[0] ?? tr("No entries");
  }

  if (entry != null && typeof entry === "object") {
    const values = Object.values(entry)
      .map((value): string | null => {
        if (typeof value === "string" && value.length > 0) {
          return value;
        }

        if (typeof value === "number" || typeof value === "bigint") {
          return String(value);
        }

        return null;
      })
      .filter((value): value is string => value != null);

    return values.length > 0 ? values.join(":") : tr("No entries");
  }

  return tr("No entries");
}
