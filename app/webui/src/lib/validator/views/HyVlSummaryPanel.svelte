<script lang="ts">
  /** Validator summary panel for headline metadata and tags. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import { VwPanel } from "@repo/hyphen-views";

  import type { ApiValidatorDetail } from "$lib";
  import { getValidatorStatusTag, getValidatorTypeTag } from "$lib";
  import { HyUtilFormatNumber, HyUtilFormatRelativeDate } from "$lib/HyUtil";
  import { HyVlFormatEndpoint } from "$lib/validator/HyVlUtil";
  import { HyCopyButton, HyPanelHeader, HyScrollBody } from "$lib/views";

  interface Props {
    readonly validator: ApiValidatorDetail;
    readonly onback?: () => void;
  }

  const {
    validator,
    onback = (): void => {
      globalThis.history.back();
    },
  }: Props = $props();

  let copiedHash = $state<string | null>(null);

  const typeTag = $derived.by(() => getValidatorTypeTag(validator.type));
  const statusTag = $derived.by(() => getValidatorStatusTag(validator.status, validator.live.status));
  const poseScoreLabel = $derived.by((): string => {
    return validator.live.pose?.banScore == null ? "—" : HyUtilFormatNumber(validator.live.pose.banScore);
  });
  const networkColumns = $derived.by((): readonly { label: string; value: string }[] => {
    const columns: { label: string; value: string }[] = [];
    const core = HyVlFormatEndpoint(validator.services.coreP2p[0]);
    const platformHttps = HyVlFormatEndpoint(validator.services.platformHttps[0]);
    const platformP2p = HyVlFormatEndpoint(validator.services.platformP2p[0]);

    columns.push({ label: tr("Core P2P"), value: core });

    if (platformHttps !== tr("No entries")) {
      columns.push({ label: tr("Platform HTTPS"), value: platformHttps });
    }

    if (platformP2p !== tr("No entries")) {
      columns.push({ label: tr("Platform P2P"), value: platformP2p });
    }

    return columns;
  });
  const addressRows = $derived.by((): readonly { label: string; value: string | null; clickable: boolean }[] => {
    return [
      { label: tr("Owner"), value: validator.addresses.owner ?? null, clickable: true },
      { label: tr("Payout"), value: validator.addresses.payout ?? null, clickable: true },
      { label: tr("Payee"), value: validator.addresses.payee ?? null, clickable: true },
      { label: tr("Voting"), value: validator.addresses.voting ?? null, clickable: true },
      { label: tr("Operator"), value: validator.operatorPubKey ?? null, clickable: false },
    ];
  });
  const registeredValue = $derived.by((): string => {
    return fmt("#{}", HyUtilFormatNumber(validator.registeredHeight));
  });

  const lastPaidValue = $derived.by((): string | null => {
    if (validator.live.lastPaidHeight != null && validator.live.lastPaidAt != null) {
      return fmt(
        "#{} ({})",
        HyUtilFormatNumber(validator.live.lastPaidHeight),
        HyUtilFormatRelativeDate(validator.live.lastPaidAt)
      );
    }

    if (validator.live.lastPaidHeight != null) {
      return fmt("#{}", HyUtilFormatNumber(validator.live.lastPaidHeight));
    }

    if (validator.live.lastPaidAt != null) {
      return HyUtilFormatRelativeDate(validator.live.lastPaidAt);
    }

    return null;
  });

  function copyToClipboard(value: string, event?: MouseEvent): void {
    event?.stopPropagation();
    void globalThis.navigator.clipboard.writeText(value).then((): void => {
      copiedHash = value;
      globalThis.setTimeout((): void => {
        copiedHash = null;
      }, 1_500);
    });
  }
</script>

<div class="summary-panel">
  <VwPanel border={true} padding={0} radius="sm" variant="glass">
    <HyPanelHeader {onback} title={tr("Validator")} />

    <HyScrollBody container={true}>
      <div class="summary-stack">
      <div>
        <div class="section-title">ProTxHash</div>
        <div class="hy-reference-hash-row">
          <span class="mono hash-full hy-wrap-anywhere">{validator.proTxHash}</span>
          <HyCopyButton
            copied={copiedHash === validator.proTxHash}
            inline={true}
            label={tr("Copy proTxHash")}
            onpress={(event: MouseEvent): void => {
              copyToClipboard(validator.proTxHash, event);
            }}
            title={tr("Copy hash")}
          />
        </div>
      </div>

      <div class="validator-stat-grid">
        <div class="hy-reference-stat-card">
          <span class="hy-reference-stat-label">{tr("Status")}</span>
          <span class="info-value">
            <span
              class:active={statusTag.tone === "green"}
              class:banned={statusTag.tone === "red"}
              class:registered={statusTag.tone === "yellow"}
              class="status-dot"
            ></span>
            <span
              class:active-text={statusTag.tone === "green"}
              class:banned-text={statusTag.tone === "red"}
              class:registered-text={statusTag.tone === "yellow"}
            >
              {statusTag.label}
            </span>
          </span>
        </div>
        <div class="hy-reference-stat-card">
          <span class="hy-reference-stat-label">{tr("Type")}</span>
          <span class="info-value">
            <span class:mint-type={typeTag.tone === "mint"} class="type-badge">{typeTag.label}</span>
          </span>
        </div>
        <div class="hy-reference-stat-card">
          <span class="hy-reference-stat-label">{tr("PoSe Score")}</span>
          <span class="hy-reference-stat-value mono">{poseScoreLabel}</span>
        </div>
      </div>

      <div>
        <div class="section-title">{tr("Network")}</div>
        <div class:three-cols={networkColumns.length === 3} class="network-grid">
          {#each networkColumns as column (column.label)}
            <div class="network-cell">
              <span class="kv-label">{column.label}</span>
              <span class="kv-value mono">{column.value}</span>
            </div>
          {/each}
        </div>
      </div>

      <div>
        <div class="section-title">{tr("Addresses")}</div>
        {#each addressRows as row (row.label)}
          <div class="kv-row">
            <span class="kv-label">{row.label}</span>
            {#if row.value != null}
              <div class="kv-value-hash">
                {#if row.clickable}
                  <a class="address-link mono hy-wrap-anywhere" href={"/address?id=" + encodeURIComponent(row.value)}>
                    {row.value}
                  </a>
                {:else}
                  <span class="mono hy-wrap-anywhere">{row.value}</span>
                {/if}
                {#if row.clickable}
                  <HyCopyButton
                    copied={copiedHash === row.value}
                    inline={true}
                    label={tr("Copy value")}
                    onpress={(event: MouseEvent): void => {
                      copyToClipboard(row.value ?? "", event);
                    }}
                    title={tr("Copy")}
                  />
                {/if}
              </div>
            {:else}
              <span class="kv-value mono">—</span>
            {/if}
          </div>
        {/each}
      </div>

      <div class="validator-stat-grid validator-stat-grid--two">
        <div class="hy-reference-stat-card">
          <span class="hy-reference-stat-label">{tr("Registered")}</span>
          <span class="hy-reference-stat-value mono">{registeredValue}</span>
        </div>
        <div class="hy-reference-stat-card">
          <span class="hy-reference-stat-label">{tr("Last Paid")}</span>
          <span class="hy-reference-stat-value mono">{lastPaidValue ?? "—"}</span>
        </div>
      </div>
      </div>
    </HyScrollBody>
  </VwPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .summary-stack {
    display: grid;
    gap: var(--hy-detail-flow-gap);
    padding: var(--hy-detail-section-padding);
  }

  .validator-stat-grid {
    display: grid;
    gap: var(--hy-detail-grid-gap);
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .summary-panel {
    height: 100%;
    min-width: 0;
  }

  .summary-panel :global(.panel) {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: 100%;
    min-width: 0;
    overflow: hidden;
  }

  .validator-stat-grid--two {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .network-grid {
    display: grid;
    gap: var(--hy-detail-grid-gap);
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .network-grid.three-cols {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .network-cell {
    align-items: start;
    border: 1px solid rgba(255, 255, 255, 0.05);
    border-radius: var(--hy-detail-tag-radius);
    display: grid;
    gap: 8px;
    min-width: 0;
    padding: 12px 14px;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .hash-full {
    font-size: var(--text-sm);
  }

  .section-title {
    color: var(--accent-blue);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.04em;
    margin-bottom: var(--hy-detail-tag-gap);
    text-transform: uppercase;
  }

  .kv-label {
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .info-value,
  .kv-value {
    align-items: center;
    color: var(--color-fg);
    display: flex;
    gap: var(--hy-detail-tag-gap);
    min-width: 0;
    font-size: var(--text-sm);
  }

  .kv-value-hash {
    align-items: start;
    column-gap: var(--hy-detail-hash-gap);
    display: grid;
    font-size: var(--text-sm);
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .status-dot {
    background: var(--status-negative);
    border-radius: 50%;
    flex-shrink: 0;
    height: var(--hy-status-dot-size);
    width: var(--hy-status-dot-size);
  }

  .status-dot.active {
    background: var(--status-positive);
  }

  .status-dot.registered {
    background: var(--status-warning);
  }

  .active-text {
    color: var(--status-positive);
  }

  .banned-text {
    color: var(--status-negative);
  }

  .registered-text {
    color: var(--status-warning);
  }

  .type-badge {
    background: var(--surface-accent-strong);
    border-radius: var(--hy-detail-tag-radius);
    color: var(--accent-blue);
    font-size: var(--text-xs);
    font-weight: 700;
    padding: var(--hy-badge-inline-padding);
    white-space: nowrap;
  }

  .type-badge.mint-type {
    background: var(--surface-mint-subtle);
    color: var(--accent-mint);
  }

  .address-link {
    color: var(--accent-blue);
    text-decoration: none;
  }

  .address-link:hover {
    color: var(--accent-blue-bright);
    text-decoration: underline;
  }

  .kv-row {
    align-items: start;
    display: grid;
    gap: var(--hy-detail-flow-gap);
    grid-template-columns: var(--hy-detail-kv-label-width-validator) minmax(0, 1fr);
    padding: var(--hy-detail-inline-padding);
  }

  @include view.media-down(tablet) {
    .summary-panel {
      height: auto;
    }

    .summary-panel :global(.panel) {
      height: auto;
    }

    .network-grid,
    .network-grid.three-cols {
      grid-template-columns: 1fr;
    }

    .kv-row {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
