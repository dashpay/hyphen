import type { ApiBlockDetail } from "../api";
import type { WebUiBlockTagRule, WebUiConfig } from "../config/schema";

export interface BlockTag {
  readonly label: string;
  readonly tone: string;
}

function matchesBlockTagRule(block: ApiBlockDetail, rule: WebUiBlockTagRule): boolean {
  if (rule.block_hash != null && rule.block_hash !== block.blockHash) {
    return false;
  }

  if (rule.height != null && rule.height !== block.blockHeight) {
    return false;
  }

  if (rule.has_coinbase != null && rule.has_coinbase !== (block.cbTx != null)) {
    return false;
  }

  if (rule.best_cl_height_present != null) {
    const hasBestClHeight = block.cbTx?.bestClHeight != null;

    if (rule.best_cl_height_present !== hasBestClHeight) {
      return false;
    }
  }

  return true;
}

/** Returns config-driven tags that match one block detail payload. */
export function getBlockTags(block: ApiBlockDetail, config: WebUiConfig): readonly BlockTag[] {
  return config.blockTags
    .filter((rule: WebUiBlockTagRule): boolean => matchesBlockTagRule(block, rule))
    .map(
      (rule: WebUiBlockTagRule): BlockTag => ({
        label: rule.label,
        tone: rule.tone,
      })
    );
}
