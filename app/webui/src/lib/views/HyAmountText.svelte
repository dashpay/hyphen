<script lang="ts">
  /** Amount text that truncates the numeric part before the unit. */
  import { splitDenominatedAmount } from "$lib/config/format";

  interface Props {
    readonly value: string;
    readonly title?: string;
  }

  const { value, title }: Props = $props();

  const parts = $derived(splitDenominatedAmount(value));
</script>

<span class="hy-amount-text" title={title ?? value}>
  <span class="hy-amount-number">{parts.amount}</span>
  {#if parts.unit != null}
    <span class="hy-amount-unit">{parts.unit}</span>
  {/if}
</span>

<style>
  .hy-amount-text {
    align-items: baseline;
    display: inline-flex;
    gap: 0.35em;
    max-width: 100%;
    min-width: 0;
  }

  .hy-amount-number {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hy-amount-unit {
    flex: 0 0 auto;
    white-space: nowrap;
  }
</style>
