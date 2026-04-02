<script lang="ts">
  /** Address summary panel for headline balance and stat fields. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwPanel } from "@repo/hyphen-views";

  import type { ApiAddressSummary } from "$lib";
  import { getWebUiConfig } from "$lib";
  import { HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import {
    HyAmountText,
    HyCopyButton,
    HyDetailSection,
    HyMetricCard,
    HyMetricGrid,
    HyPanelHeader,
    HyScrollBody,
  } from "$lib/views";

  interface Props {
    readonly address: string;
    readonly summary: ApiAddressSummary;
    readonly onback?: () => void;
  }

  const {
    address,
    summary,
    onback = (): void => {
      globalThis.history.back();
    },
  }: Props = $props();

  let copiedValue: string | null = $state(null);

  function copyToClipboard(value: string): void {
    void globalThis.navigator.clipboard.writeText(value).then((): void => {
      copiedValue = value;
      globalThis.setTimeout((): void => {
        copiedValue = null;
      }, 1_500);
    });
  }
</script>

<VwPanel border={true} padding={0} radius="sm" variant="glass">
  <HyPanelHeader {onback} title={tr("Address")} />

  <HyScrollBody container={true}>
    <HyDetailSection title={tr("Address")}>
      <div class="hash-row">
        <span class="mono hash-full">{address}</span>
        <HyCopyButton
          copied={copiedValue === address}
          inline={true}
          label={tr("Copy address")}
          onpress={(): void => {
            copyToClipboard(address);
          }}
          title={tr("Copy address")}
        />
      </div>
    </HyDetailSection>

    <HyDetailSection>
      <HyMetricCard tone="accent">
        <span class="balance-label">{tr("Balance")}</span>
        <span class="balance-value">
          <HyAmountText value={HyUtilFormatSelectedAmount(summary.balance, getWebUiConfig())} />
        </span>
      </HyMetricCard>
    </HyDetailSection>

    <HyDetailSection>
      <HyMetricGrid>
        <HyMetricCard>
          <span class="stat-label">{tr("Total Sent")}</span>
          <span class="stat-value">
            <HyAmountText value={HyUtilFormatSelectedAmount(summary.totalSent, getWebUiConfig())} />
          </span>
        </HyMetricCard>
        <HyMetricCard>
          <span class="stat-label">{tr("Total Received")}</span>
          <span class="stat-value">
            <HyAmountText value={HyUtilFormatSelectedAmount(summary.totalReceived, getWebUiConfig())} />
          </span>
        </HyMetricCard>
      </HyMetricGrid>
    </HyDetailSection>
  </HyScrollBody>
</VwPanel>

<style>
  .hash-row {
    align-items: center;
    display: flex;
    gap: var(--hy-detail-copy-gap);
  }

  .hash-full {
    color: var(--color-fg);
    font-size: var(--text-sm);
    line-height: 1.4;
    word-break: break-all;
  }

  .balance-label {
    color: var(--accent-blue);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .balance-value {
    color: var(--color-fg);
    display: inline-flex;
    font-family: var(--font-mono);
    font-size: var(--text-md);
    font-weight: 800;
    max-width: 100%;
    min-width: 0;
  }

  .stat-label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .stat-value {
    color: var(--color-fg);
    display: inline-flex;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 700;
    max-width: 100%;
    min-width: 0;
  }

  .mono {
    font-family: var(--font-mono);
  }
</style>
