import { tr } from "@repo/hyphen-i18n";
import type { ApiProposalDetail } from "$lib/api";

/** Formats one proposal count for list subtitles. */
export function HyPrFormatProposalCount(count: number): string {
  return tr("{count, plural, one {# proposal} other {# proposals}}", { count });
}

/** Formats proposal names for display labels. */
export function HyPrFormatName(name: string): string {
  return name.replaceAll("_", " ");
}

/** Returns the net yes-minus-no vote count for one proposal. */
export function HyPrGetNetVotes(proposal: ApiProposalDetail): number {
  return proposal.votes.yes - proposal.votes.no;
}

/** Returns the funded proposals from one proposal list. */
export function HyPrGetFundedProposals(proposals: readonly ApiProposalDetail[]): readonly ApiProposalDetail[] {
  return proposals.filter((proposal): boolean => proposal.funded);
}

/** Returns the summed yes/no/abstain votes across one proposal list. */
export function HyPrGetTotalVotes(proposals: readonly ApiProposalDetail[]): number {
  return proposals.reduce((sum: number, proposal: ApiProposalDetail): number => {
    return sum + proposal.votes.yes + proposal.votes.no + proposal.votes.abstain;
  }, 0);
}

/** Returns the next governance superblock height from one current chain height. */
export function HyPrGetNextSuperblock(
  blockHeight: number | null,
  governanceSuperblockInterval: number | null
): number | null {
  if (
    blockHeight == null ||
    !Number.isFinite(blockHeight) ||
    blockHeight < 0 ||
    governanceSuperblockInterval == null ||
    !Number.isFinite(governanceSuperblockInterval) ||
    governanceSuperblockInterval <= 0
  ) {
    return null;
  }

  return Math.ceil((blockHeight + 1) / governanceSuperblockInterval) * governanceSuperblockInterval;
}
