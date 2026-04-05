<script lang="ts">
  /** Combined governance summary card with budget donut and proposal stats. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwDonutChart, VwPanel, VwText } from "@repo/hyphen-views";

  import { getWebUiConfig } from "$lib";
  import { HyUtilFormatNumber, HyUtilFormatRelativeDate, HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import type { ApiNetworkResponse, ApiProposalDetail } from "$lib/api";
  import { HyPrGetNextSuperblock } from "$lib/proposal/HyPrUtil";

  interface DonutSegment {
    readonly color: string;
    readonly label: string;
    readonly value: number;
  }

  interface Props {
    readonly fundedProposals: readonly ApiProposalDetail[];
    readonly focusProposal: ApiProposalDetail | null;
    readonly blockHeight: number | null;
    readonly network: ApiNetworkResponse | null;
  }

  const { fundedProposals, focusProposal, blockHeight, network }: Props = $props();

  const DONUT_COLORS = [
    "#4C7EFF",
    "#60F6D2",
    "#FF6B8A",
    "#FFB74D",
    "#AB7AFF",
    "#4DD0E1",
    "#FF8A65",
    "#81C784",
  ] as const;

  let panelWidth = $state(0);
  let visualWidth = $state(0);
  let visualHeight = $state(0);
  let hoveredProposal = $state<number | null>(null);

  const donutSize = $derived.by((): number => {
    if (visualWidth <= 0 || visualHeight <= 0) {
      return 180;
    }

    const inset = panelWidth <= 640 ? 12 : 20;
    const minSize = panelWidth <= 640 ? 156 : 180;
    const maxSize = panelWidth <= 640 ? 260 : 320;
    const availableWidth = Math.max(visualWidth - inset * 2, 0);
    const availableHeight = Math.max(visualHeight - inset * 2, 0);
    const available = Math.min(availableWidth, availableHeight);

    return Math.max(minSize, Math.min(available, maxSize));
  });

  const donutStroke = $derived.by((): number => {
    return Math.max(22, Math.min(32, Math.round(donutSize * 0.15)));
  });

  const fundedTotal = $derived.by((): number => {
    return fundedProposals.reduce((sum: number, proposal: ApiProposalDetail): number => {
      return sum + proposal.paymentAmount;
    }, 0);
  });

  const donutSegments = $derived.by((): readonly DonutSegment[] => {
    return fundedProposals.map((proposal: ApiProposalDetail, index: number): DonutSegment => {
      return {
        color: DONUT_COLORS[index % DONUT_COLORS.length] ?? "#4C7EFF",
        label: proposal.name.replaceAll("_", " "),
        value: proposal.paymentAmount,
      };
    });
  });

  const centerSubvalue = $derived.by((): string => {
    const proposal = hoveredProposal == null ? null : fundedProposals[hoveredProposal];
    if (proposal != null) {
      return HyUtilFormatSelectedAmount(proposal.paymentAmount, getWebUiConfig(), 2);
    }

    return HyUtilFormatSelectedAmount(fundedTotal, getWebUiConfig(), 2);
  });

  const nextSuperblock = $derived.by((): number | null => {
    return HyPrGetNextSuperblock(blockHeight, getWebUiConfig().features.governance.superblockInterval);
  });

  const nextSuperblockEstimate = $derived.by((): string | null => {
    if (nextSuperblock == null || blockHeight == null) {
      return null;
    }

    const blocksRemaining = Math.max(nextSuperblock - blockHeight, 0);
    const estimate = new Date(Date.now() + blocksRemaining * 150_000);
    return HyUtilFormatRelativeDate(estimate.toISOString());
  });

  const registeredNodesLabel = $derived.by((): string => {
    if (network == null) {
      return "—";
    }

    const totalNodes = network.validators.regular.total + network.validators.evonode.total;
    const enabledNodes =
      (network.validators.regular.status["valid"] ?? 0) + (network.validators.evonode.status["valid"] ?? 0);

    return tr(
      "{} ({} enabled)",
      HyUtilFormatNumber(totalNodes),
      HyUtilFormatNumber(enabledNodes)
    );
  });

  const totalVotesLabel = $derived.by((): string => {
    if (focusProposal == null) {
      return "—";
    }

    return HyUtilFormatNumber(focusProposal.votes.yes + focusProposal.votes.no + focusProposal.votes.abstain);
  });

  const fundingThresholdLabel = $derived.by((): string => {
    return focusProposal == null ? "—" : HyUtilFormatNumber(focusProposal.fundingThreshold);
  });

  const budgetLabel = $derived.by((): string => {
    return HyUtilFormatSelectedAmount(fundedTotal, getWebUiConfig(), 2);
  });

  const nextSuperblockLabel = $derived.by((): string => {
    if (nextSuperblock == null) {
      return "—";
    }

    if (nextSuperblockEstimate == null) {
      return "#" + HyUtilFormatNumber(nextSuperblock);
    }

    return tr("#{} ({})", HyUtilFormatNumber(nextSuperblock), nextSuperblockEstimate);
  });
</script>

<VwPanel border={true} padding={0} radius="sm" variant="glass">
  <div class="widget-header">
    <VwText as="h2" size="md+" weight="extrabold">{tr("Details")}</VwText>
  </div>

  <div bind:clientWidth={panelWidth} class="summary-body">
    <div bind:clientHeight={visualHeight} bind:clientWidth={visualWidth} class="summary-visual">
      {#if fundedProposals.length > 0}
        <div class="donut-shell" style:width={String(donutSize) + "px"} style:height={String(donutSize) + "px"}>
          <VwDonutChart
            centerLabel={tr("Budget")}
            {centerSubvalue}
            centerValue="100%"
            fill={true}
            hoveredIndex={hoveredProposal}
            onhover={(index: number | null): void => {
              hoveredProposal = index;
            }}
            segments={donutSegments}
            size={donutSize}
            strokeWidth={donutStroke}
          />
        </div>
      {:else}
        <div class="empty-msg">
          <VwText color="muted" size="sm">{tr("No funded proposals")}</VwText>
        </div>
      {/if}
    </div>

    <div class="summary-info">
      <div class="info-row">
        <span class="detail-label">{tr("Registered Nodes")}</span>
        <span class="detail-value">{registeredNodesLabel}</span>
      </div>

      <div class="info-row">
        <span class="detail-label">{tr("Total Votes")}</span>
        <span class="detail-value">{totalVotesLabel}</span>
      </div>

      <div class="info-row">
        <span class="detail-label">{tr("Funding Threshold")}</span>
        <span class="detail-value">{fundingThresholdLabel}</span>
      </div>

      <div class="info-row">
        <span class="detail-label">{tr("Budget")}</span>
        <span class="detail-value">{budgetLabel}</span>
      </div>

      <div class="info-row">
        <span class="detail-label">{tr("Next Superblock")}</span>
        <span class="detail-value">{nextSuperblockLabel}</span>
      </div>
    </div>
  </div>
</VwPanel>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .widget-header {
    border-bottom: 1px solid var(--glass-border);
    flex-shrink: 0;
    padding: 16px 20px;
  }

  .summary-body {
    display: grid;
    flex: 1;
    grid-template-rows: minmax(0, 3fr) auto;
    height: 100%;
    min-height: 0;
  }

  .summary-visual {
    align-items: center;
    border-bottom: 1px solid var(--glass-border);
    display: flex;
    justify-content: center;
    min-height: 0;
    overflow: hidden;
    padding: 20px;
  }

  .donut-shell {
    flex: 0 0 auto;
    max-height: 100%;
    max-width: 100%;
  }

  .empty-msg {
    align-items: center;
    display: flex;
    justify-content: center;
    min-height: 180px;
  }

  .summary-info {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 12px 20px 18px;
  }

  .info-row {
    align-items: baseline;
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    column-gap: 16px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) max-content;
    padding: 10px 0;
  }

  .info-row:first-child {
    border-top: none;
    padding-top: 0;
  }

  .detail-label {
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 500;
    min-width: 0;
  }

  .detail-value {
    color: var(--color-fg);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 600;
    min-width: 0;
    text-align: right;
    white-space: nowrap;
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .summary-body {
      align-items: stretch;
      display: grid;
      gap: 0;
      grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
      grid-template-rows: none;
      height: auto;
    }

    .summary-visual {
      border-bottom: none;
      border-right: 1px solid var(--glass-border);
      padding: 16px 12px;
    }

    .summary-info {
      padding: 10px 16px 12px;
    }

    .info-row {
      align-items: start;
      grid-template-columns: minmax(0, 1fr);
      padding: 8px 0;
      row-gap: 4px;
    }

    .detail-value {
      text-align: left;
      white-space: normal;
    }
  }
</style>
