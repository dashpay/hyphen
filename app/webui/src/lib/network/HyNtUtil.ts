import { tr } from "@repo/hyphen-i18n";
import type { ApiNetworkPeerInfo } from "$lib/api";

import { formatLocaleNumber } from "$lib/data/format";

export interface HyNtPeerRow {
  readonly id: string;
  readonly addr: string;
  readonly connectionType: string;
  readonly network: string;
  readonly ping: string;
  readonly relay: string;
  readonly subversion: string;
}

function readString(record: Readonly<Record<string, unknown>>, key: string, fallback = "—"): string {
  const value = record[key];
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

function readNumber(record: Readonly<Record<string, unknown>>, key: string): number | null {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readBoolean(record: Readonly<Record<string, unknown>>, key: string): boolean | null {
  const value = record[key];
  return typeof value === "boolean" ? value : null;
}

/** Formats one peer count for panel subtitles. */
export function HyNtFormatPeerCount(count: number): string {
  return tr("{count, plural, one {# peer} other {# peers}}", { count });
}

/** Reads one network snapshot field as compact display text. */
export function HyNtReadSnapshotValue(
  snapshot: Readonly<Record<string, unknown>>,
  key: string,
  fallback = "—"
): string {
  const value = snapshot[key];

  if (typeof value === "number" && Number.isFinite(value)) {
    return formatLocaleNumber(value);
  }

  if (typeof value === "boolean") {
    return value ? tr("Yes") : tr("No");
  }

  if (typeof value === "string" && value.length > 0) {
    return value;
  }

  return fallback;
}

/** Normalizes raw peer info objects into table rows. */
export function HyNtGetPeerRows(peers: readonly ApiNetworkPeerInfo[]): readonly HyNtPeerRow[] {
  return peers.map((peer, index): HyNtPeerRow => {
    const ping = readNumber(peer, "pingtime");
    const relay = readBoolean(peer, "relaytxes");

    return {
      id: readString(peer, "session_id", String(index)),
      addr: readString(peer, "addr"),
      connectionType: readString(peer, "connection_type"),
      network: readString(peer, "network"),
      ping: ping == null ? "—" : `${ping.toFixed(3)} s`,
      relay: relay == null ? "—" : relay ? tr("On") : tr("Off"),
      subversion: readString(peer, "subver"),
    };
  });
}
