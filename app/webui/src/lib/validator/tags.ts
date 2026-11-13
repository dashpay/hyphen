import { tr } from "@repo/hyphen-i18n";

export interface ValidatorTag {
  readonly label: string;
  readonly tone: "blue" | "green" | "mint" | "red" | "yellow";
}

/** Maps validator types to the shared tag label and tone. */
export function getValidatorTypeTag(type: string): ValidatorTag {
  if (type.toLowerCase() === "evonode") {
    return {
      label: tr("Evo"),
      tone: "blue",
    };
  }

  return {
    label: tr("Regular"),
    tone: "mint",
  };
}

function isBannedStatus(status: string | null | undefined): boolean {
  if (status == null) {
    return false;
  }

  switch (status.toLowerCase()) {
    case "banned":
    case "pose_banned":
    case "revoked":
      return true;
    default:
      return false;
  }
}

/** Maps validator statuses to the shared tag label and tone. */
export function getValidatorStatusTag(indexedStatus: string, liveStatus?: string | null): ValidatorTag {
  if (liveStatus?.toLowerCase() === "valid") {
    return {
      label: tr("Active"),
      tone: "green",
    };
  }

  if (isBannedStatus(liveStatus) || isBannedStatus(indexedStatus)) {
    return {
      label: tr("Banned"),
      tone: "red",
    };
  }

  switch ((liveStatus ?? indexedStatus).toLowerCase()) {
    case "registered":
      return {
        label: tr("Registered"),
        tone: "yellow",
      };
    default:
      return {
        label: liveStatus ?? indexedStatus,
        tone: "mint",
      };
  }
}
