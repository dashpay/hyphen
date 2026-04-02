<script lang="ts">
  /** Table for live governance proposal summaries. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import IconChevronDown from "virtual:icons/mdi/chevron-down";
  import IconChevronRight from "virtual:icons/mdi/chevron-right";
  import type { ApiProposalDetail } from "$lib";
  import { getWebUiConfig } from "$lib";
  import { HyUtilFormatDate, HyUtilFormatNumber, HyUtilFormatFullDate, HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import { HyPrFormatName, HyPrGetNetVotes } from "$lib/proposal/HyPrUtil";
  import { HyAmountText, HyPanelAsyncState, HyScrollBody, HyTableHeaderRow } from "$lib/views";

  interface Props {
    readonly proposals: readonly ApiProposalDetail[];
    readonly expandedHash?: string | null;
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly onproposaltoggle?: (hash: string) => void;
    readonly scrollMode?: "page" | "self";
  }

  const {
    proposals,
    expandedHash = null,
    hasMore = false,
    loadingMore = false,
    onloadmore,
    onproposaltoggle,
    scrollMode = "self",
  }: Props = $props();
  let hoveredProposal = $state<string | null>(null);

  const DONUT_COLORS = [
    "#4C7EFF",
    "#60F6D2",
    "#FF6B8A",
    "#FFB74D",
    "#AB7AFF",
    "#4DD0E1",
    "#FF8A65",
    "#81C784",
    "#F06292",
    "#90A4AE",
    "#E6EE9C",
    "#CE93D8",
    "#80DEEA",
    "#FFAB91",
    "#A5D6A7",
    "#EF9A9A",
    "#B39DDB",
    "#FFF176",
    "#80CBC4",
    "#BCAAA4",
  ] as const;

  function absoluteYes(proposal: ApiProposalDetail): number {
    return proposal.yesCount - proposal.noCount;
  }

  function approvalRatio(proposal: ApiProposalDetail): number {
    const totalVotes = Math.max(proposal.yesCount + proposal.noCount + proposal.abstainCount, 1);
    return Math.max(-1, Math.min(absoluteYes(proposal) / totalVotes, 1));
  }

  function barLeft(proposal: ApiProposalDetail): string {
    const ratio = approvalRatio(proposal);
    if (ratio >= 0) {
      return "50%";
    }

    return String(50 + ratio * 50) + "%";
  }

  function barWidth(proposal: ApiProposalDetail): string {
    const ratio = Math.abs(approvalRatio(proposal));
    return String(Math.max(ratio * 50, 1)) + "%";
  }

  function formatDate(timestamp: string): string {
    return HyUtilFormatDate(timestamp);
  }

  function getRowHoverTint(index: number): string {
    return (DONUT_COLORS[index % DONUT_COLORS.length] ?? "#4C7EFF") + "14";
  }

  function getRowExpandedTint(index: number): string {
    return (DONUT_COLORS[index % DONUT_COLORS.length] ?? "#4C7EFF") + "1f";
  }
</script>

<div class="proposal-table">
  <HyScrollBody {hasMore} {loadingMore} {onloadmore} {scrollMode}>
    {#if proposals.length === 0}
      <HyPanelAsyncState empty emptyMessage={tr("No data.")} emptyTitle={tr("No entries")} />
    {:else}
      <HyTableHeaderRow>
        <span class="col col-name">{tr("Name")}</span>
        <span class="col col-amount">{tr("Amount")}</span>
        <span class="col col-votes">{fmt("{} / {}", tr("Yes"), tr("No"))}</span>
        <span class="col col-bar">{tr("Approval")}</span>
        <span class="col col-status">{tr("Status")}</span>
        <span class="col col-end">{tr("Ends")}</span>
      </HyTableHeaderRow>

      {#each proposals as proposal, index (proposal.hash)}
        <button
          class="p-row"
          data-proposal-hash={proposal.hash}
          class:p-row-hovered={hoveredProposal === proposal.hash}
          class:p-row-expanded={expandedHash === proposal.hash}
          style:--hy-proposal-row-hover={getRowHoverTint(index)}
          style:--hy-proposal-row-expanded={getRowExpandedTint(index)}
          onmouseenter={(): void => {
            hoveredProposal = proposal.hash;
          }}
          onmouseleave={(): void => {
            hoveredProposal = null;
          }}
          onclick={(): void => {
            onproposaltoggle?.(proposal.hash);
          }}
          type="button"
        >
          <span class="col col-name">
            <span class="expand-icon">
              {#if expandedHash === proposal.hash}
                <IconChevronDown aria-hidden="true" />
              {:else}
                <IconChevronRight aria-hidden="true" />
              {/if}
            </span>
            <span class="proposal-dot" style:background={DONUT_COLORS[index % DONUT_COLORS.length] ?? "#4C7EFF"}></span>
            <span class="proposal-name">{HyPrFormatName(proposal.name)}</span>
          </span>
          <span class="col col-amount mono">
            <HyAmountText value={HyUtilFormatSelectedAmount(proposal.paymentAmount, getWebUiConfig(), 2)} />
          </span>
          <span class="col col-votes">
            <span class="yes">{HyUtilFormatNumber(proposal.yesCount)}</span>
            <span class="sep">/</span>
            <span class="no">{HyUtilFormatNumber(proposal.noCount)}</span>
          </span>
          <span class="col col-bar">
            <span class="bar-track">
              <span class="bar-midline"></span>
              <span
                class="bar-fill"
                class:bar-positive={absoluteYes(proposal) >= 0}
                class:bar-negative={absoluteYes(proposal) < 0}
                style:left={barLeft(proposal)}
                style:width={barWidth(proposal)}
              ></span>
            </span>
            <span class="bar-label">{(approvalRatio(proposal) * 100).toFixed(0)}%</span>
          </span>
          <span class="col col-status">
            <span class="status-badge" class:funded={proposal.funded} class:unfunded={!proposal.funded}>
              {proposal.funded ? tr("Funded") : tr("Failing")}
            </span>
          </span>
          <span class="col col-end muted">{proposal.endTime == null ? "—" : formatDate(proposal.endTime)}</span>
        </button>

        {#if expandedHash === proposal.hash}
          <div class="p-detail">
            <div class="detail-grid">
              <span class="detail-label">{tr("Proposal Hash")}</span>
              <span class="detail-value mono">{proposal.hash}</span>

              {#if proposal.paymentAddress != null}
                <span class="detail-label">{tr("Payment Address")}</span>
                <span class="detail-value mono">{proposal.paymentAddress}</span>
              {/if}

              {#if proposal.url != null}
                <span class="detail-label">{tr("URL")}</span>
                <a class="detail-link detail-value" href={proposal.url} rel="noreferrer" target="_blank">
                  {proposal.url}
                </a>
              {/if}

              <span class="detail-label">{tr("Amount")}</span>
              <span class="detail-value">
                <HyAmountText value={HyUtilFormatSelectedAmount(proposal.paymentAmount, getWebUiConfig(), 2)} />
              </span>

              <span class="detail-label">{tr("Net Votes")}</span>
              <span class="detail-value">
                {HyUtilFormatNumber(HyPrGetNetVotes(proposal))} ({HyUtilFormatNumber(proposal.yesCount)} yes /
                {HyUtilFormatNumber(proposal.noCount)} no / {HyUtilFormatNumber(proposal.abstainCount)} abstain)
              </span>

              {#if proposal.creationTime != null}
                <span class="detail-label">{tr("Created")}</span>
                <span class="detail-value">{HyUtilFormatFullDate(proposal.creationTime)}</span>
              {/if}

              {#if proposal.startTime != null && proposal.endTime != null}
                <span class="detail-label">{tr("Voting Period")}</span>
                <span class="detail-value">
                  {fmt("{} — {}", formatDate(proposal.startTime), formatDate(proposal.endTime))}
                </span>
              {/if}
            </div>
          </div>
        {/if}
      {/each}
    {/if}
  </HyScrollBody>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .proposal-table {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .proposal-table :global(.scroll-body) {
    align-content: start;
    column-gap: 12px;
    display: grid;
    flex: 1;
    grid-template-columns: minmax(0, 1fr) max-content max-content max-content max-content;
    height: 100%;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
  }

  .proposal-table :global(.scroll-body::-webkit-scrollbar) {
    width: 8px;
  }

  .proposal-table :global(.scroll-body::-webkit-scrollbar-track) {
    background: transparent;
  }

  .proposal-table :global(.scroll-body::-webkit-scrollbar-thumb) {
    background: rgba(76, 126, 255, 0.3);
    border-radius: 10px;
  }

  .proposal-table :global(.scroll-body::-webkit-scrollbar-thumb:hover) {
    background: rgba(76, 126, 255, 0.5);
  }

  .p-row {
    align-items: center;
    background: none;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
    border-left: none;
    border-right: none;
    border-top: none;
    color: inherit;
    cursor: pointer;
    display: grid;
    font-family: var(--font-primary);
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    padding: 12px 20px;
    text-align: left;
    text-decoration: none;
    transition: background var(--duration-fast) var(--ease-default);
    width: 100%;
  }

  .p-row:hover,
  .p-row-hovered {
    background: var(--hy-proposal-row-hover, rgba(255, 255, 255, 0.04));
  }

  .p-row-expanded {
    background: var(--hy-proposal-row-expanded, rgba(76, 126, 255, 0.06));
    border-bottom-color: transparent;
  }

  .p-detail {
    background: var(--hy-proposal-row-expanded, rgba(76, 126, 255, 0.06));
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
    grid-column: 1 / -1;
    padding: 6px 20px 16px 36px;
  }

  .detail-grid {
    align-items: baseline;
    display: grid;
    gap: 6px 16px;
    grid-template-columns: max-content 1fr;
  }

  .detail-label {
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .detail-value {
    color: var(--color-fg);
    font-size: var(--text-sm);
    word-break: break-all;
  }

  .detail-link {
    color: var(--accent-blue);
    text-decoration: none;
    word-break: break-all;
  }

  .detail-link:hover {
    text-decoration: underline;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .col {
    align-items: center;
    display: flex;
    font-size: var(--text-sm);
    min-width: 0;
  }

  .expand-icon {
    align-items: center;
    color: var(--accent-blue);
    display: inline-flex;
    flex: 0 0 auto;
    height: 16px;
    justify-content: center;
    width: 16px;
  }

  .col-name {
    gap: 8px;
    min-width: 0;
    overflow: hidden;
  }

  .proposal-dot {
    border-radius: 50%;
    flex-shrink: 0;
    height: 8px;
    width: 8px;
  }

  .proposal-name {
    color: var(--color-fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .col-amount {
    justify-content: flex-end;
    white-space: nowrap;
  }

  .col-votes {
    gap: 4px;
    white-space: nowrap;
  }

  .yes {
    color: #81c784;
    font-weight: 600;
  }

  .no {
    color: #e57373;
    font-weight: 600;
  }

  .sep {
    color: var(--text-muted);
  }

  .col-bar {
    gap: 8px;
  }

  .bar-track {
    background: rgba(255, 255, 255, 0.08);
    border-radius: 3px;
    flex: 1;
    height: 6px;
    position: relative;
  }

  .bar-midline {
    background: rgba(255, 255, 255, 0.2);
    bottom: 0;
    left: 50%;
    position: absolute;
    top: 0;
    width: 1px;
  }

  .bar-fill {
    border-radius: 3px;
    height: 100%;
    position: absolute;
    top: 0;
    transition:
      left 0.3s ease,
      width 0.3s ease;
  }

  .bar-fill.bar-positive {
    background: #81c784;
  }

  .bar-fill.bar-negative {
    background: #e57373;
  }

  .bar-label {
    color: var(--text-muted);
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 700;
    min-width: 32px;
    text-align: right;
  }

  .col-status {
    white-space: nowrap;
  }

  .status-badge {
    border-radius: 6px;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.03em;
    padding: 3px 8px;
    text-transform: uppercase;
  }

  .status-badge.funded {
    background: rgba(129, 199, 132, 0.12);
    color: #81c784;
  }

  .status-badge.unfunded {
    background: rgba(229, 115, 115, 0.12);
    color: #e57373;
  }

  .col-end {
    white-space: nowrap;
  }

  .muted {
    color: var(--text-muted);
  }

  /* --hy-breakpoint-desktop */
  @include view.media-up(desktop) {
    .proposal-table :global(.scroll-body) {
      grid-template-columns: minmax(0, 1fr) max-content max-content minmax(120px, auto) max-content max-content;
    }

    .col-bar {
      display: flex;
    }
  }

  /* --hy-breakpoint-desktop */
  @include view.media-below(desktop) {
    .col-bar {
      display: none;
    }
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .proposal-table :global(.scroll-body) {
      overflow-y: visible;
      scrollbar-width: none;
    }

    .proposal-table :global(.scroll-body::-webkit-scrollbar) {
      height: 0;
      width: 0;
    }

    .col-end {
      display: none;
    }
  }

  /* --hy-breakpoint-phone */
  @include view.media-down(phone) {
    .proposal-table :global(.scroll-body) {
      grid-template-columns: minmax(0, 1fr) max-content max-content;
    }

    .col-votes {
      display: none;
    }
  }
</style>
