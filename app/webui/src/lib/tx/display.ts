import { tr } from "@repo/hyphen-i18n";

import { assetLockRenderer } from "./assetlock";
import { assetUnlockRenderer } from "./assetunlock";
import { coinbaseRenderer } from "./coinbase";
import { dataRenderer } from "./data";
import { mnhfRenderer } from "./mnhf";
import { paymentRenderer } from "./payment";
import { proRegRenderer } from "./proreg";
import { proUpRegRenderer } from "./proupreg";
import { proUpRevRenderer } from "./prouprev";
import { proUpServRenderer } from "./proupserv";
import { quorumRenderer } from "./quorum";

export type TransactionTone =
  | "assetlock"
  | "assetunlock"
  | "coinbase"
  | "data"
  | "default"
  | "mnhf"
  | "proreg"
  | "proupreg"
  | "prouprev"
  | "proupserv"
  | "quorum";

export interface TransactionDisplay {
  readonly label: string;
  readonly tone: TransactionTone;
}

interface TransactionDisplaySource {
  readonly label: string;
  readonly tone: TransactionTone;
}

const DISPLAY_MAP: Readonly<Record<string, TransactionDisplaySource>> = {
  asset_lock: { label: assetLockRenderer.label, tone: assetLockRenderer.tone },
  asset_unlock: { label: assetUnlockRenderer.label, tone: assetUnlockRenderer.tone },
  coinbase: { label: coinbaseRenderer.label, tone: coinbaseRenderer.tone },
  data: { label: dataRenderer.label, tone: dataRenderer.tone },
  mnhf: { label: mnhfRenderer.label, tone: mnhfRenderer.tone },
  mn_hf_signal: { label: mnhfRenderer.label, tone: mnhfRenderer.tone },
  normal: { label: paymentRenderer.label, tone: paymentRenderer.tone },
  payment: { label: paymentRenderer.label, tone: paymentRenderer.tone },
  provider_register: { label: proRegRenderer.label, tone: proRegRenderer.tone },
  provider_update_registrar: { label: proUpRegRenderer.label, tone: proUpRegRenderer.tone },
  provider_update_revoke: { label: proUpRevRenderer.label, tone: proUpRevRenderer.tone },
  provider_update_service: { label: proUpServRenderer.label, tone: proUpServRenderer.tone },
  proreg: { label: "Register", tone: "proreg" },
  proupreg: { label: "Update", tone: "proupreg" },
  prouprev: { label: "Revoke", tone: "prouprev" },
  proupserv: { label: "Update", tone: "proupserv" },
  quorum_commitment: { label: quorumRenderer.label, tone: quorumRenderer.tone },
  standard: { label: paymentRenderer.label, tone: paymentRenderer.tone },
};

function normalizeTransactionType(txType: string, blockPosition?: number | null): string {
  if (blockPosition === 0) {
    return "coinbase";
  }

  return txType;
}

function translateDisplayLabel(label: string): string {
  switch (label) {
    case "Payment":
      return tr("Payment");
    case "Reward":
      return tr("Reward");
    case "Data":
      return tr("Data");
    case "Lock":
      return tr("Lock");
    case "Unlock":
      return tr("Unlock");
    case "Register":
      return tr("Register");
    case "Update":
      return tr("Update");
    case "Revoke":
      return tr("Revoke");
    case "Commit":
      return tr("Commit");
    case "ForkSig":
      return tr("ForkSig");
    default:
      return label;
  }
}

/** Maps API tx types to the shared label and badge treatment. */
export function getTransactionDisplay(txType: string, blockPosition?: number | null): TransactionDisplay {
  const normalizedType = normalizeTransactionType(txType, blockPosition);
  const display = DISPLAY_MAP[normalizedType];

  if (display != null) {
    return {
      label: translateDisplayLabel(display.label),
      tone: display.tone,
    };
  }

  return {
    label: normalizedType
      .split(/[_\s-]+/u)
      .filter((part): boolean => part.length > 0)
      .map((part): string => part.slice(0, 1).toUpperCase() + part.slice(1).toLowerCase())
      .join(" "),
    tone: "default",
  };
}
