<script lang="ts">
  /** Governance explorer page surface with budget donut and proposal table. */
  import { onMount } from "svelte";
  import { tr } from "@repo/hyphen-i18n";
  import { VwPanel, VwText } from "@repo/hyphen-views";

  import type { ApiNetworkResponse, ApiProposalDetail } from "$lib";
  import { HyUtilGetBreakpointWidth } from "$lib/HyUtil";
  import { HyPrFormatProposalCount, HyPrGetFundedProposals } from "$lib/proposal/HyPrUtil";
  import HyPrDetailsPanel from "./HyPrDetailsPanel.svelte";
  import { HyPrTable } from "$lib/proposal/views";

  interface Props {
    readonly proposals: readonly ApiProposalDetail[];
    readonly network: ApiNetworkResponse | null;
    readonly totalCount?: number | null;
    readonly blockHeight: number | null;
    readonly expandedHash?: string | null;
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly onProposalToggle?: (hash: string) => void;
  }

  const {
    proposals,
    network,
    totalCount = null,
    blockHeight,
    expandedHash = null,
    hasMore = false,
    loadingMore = false,
    onloadmore,
    onProposalToggle,
  }: Props = $props();
  let stackedForDock = $state(false);

  const fundedProposals = $derived(HyPrGetFundedProposals(proposals));
  const focusProposal = $derived.by((): ApiProposalDetail | null => {
    if (expandedHash != null) {
      return (
        proposals.find((proposal: ApiProposalDetail): boolean => proposal.hash === expandedHash) ?? proposals[0] ?? null
      );
    }

    return proposals[0] ?? null;
  });

  onMount((): (() => void) => {
    const query = globalThis.matchMedia(`(max-width: ${String(HyUtilGetBreakpointWidth("tablet"))}px)`);
    const sync = (): void => {
      stackedForDock = query.matches;
    };

    sync();
    query.addEventListener("change", sync);

    return (): void => {
      query.removeEventListener("change", sync);
    };
  });
</script>

<div class:dock-mode={stackedForDock} class="proposals-page">
  <div class="proposals-summary">
    <HyPrDetailsPanel {blockHeight} {focusProposal} {fundedProposals} {network} />
  </div>

  <div class="proposals-list">
    <VwPanel border={true} padding={0} radius="sm" variant="glass">
      <div class="panel-header">
        <div class="panel-header-row">
          <VwText as="h2" size="md+" weight="extrabold">{tr("Proposals")}</VwText>
          <VwText color="muted" size="sm" weight="medium">
            {HyPrFormatProposalCount(totalCount ?? proposals.length)} ({tr("{count} funded", {
              count: fundedProposals.length,
            })})
          </VwText>
        </div>
      </div>
      {#if onProposalToggle != null}
        <HyPrTable
          {expandedHash}
          {hasMore}
          {loadingMore}
          {onloadmore}
          {proposals}
          onproposaltoggle={onProposalToggle}
          scrollMode={stackedForDock ? "page" : "self"}
        />
      {:else}
        <HyPrTable
          {expandedHash}
          {hasMore}
          {loadingMore}
          {onloadmore}
          {proposals}
          scrollMode={stackedForDock ? "page" : "self"}
        />
      {/if}
    </VwPanel>
  </div>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .proposals-page {
    display: grid;
    gap: var(--hy-browser-gap);
    grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    height: 100%;
    min-width: 0;
  }

  .proposals-summary,
  .proposals-list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .proposals-list :global(.panel) {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .proposals-summary :global(.panel) {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  .panel-header {
    border-bottom: 1px solid var(--glass-border);
    flex-shrink: 0;
    padding: 16px 20px;
  }

  .panel-header-row {
    align-items: center;
    display: flex;
    gap: 8px;
    justify-content: space-between;
  }

  .proposals-page.dock-mode {
    display: flex;
    flex-direction: column;
    height: auto;
    overflow: visible;
  }

  .proposals-page.dock-mode .proposals-summary,
  .proposals-page.dock-mode .proposals-list {
    display: block;
    min-height: auto;
    overflow: visible;
  }

  .proposals-page.dock-mode .proposals-summary {
    flex: 0 0 auto;
  }

  .proposals-page.dock-mode .proposals-list {
    flex: 0 0 auto;
  }
</style>
