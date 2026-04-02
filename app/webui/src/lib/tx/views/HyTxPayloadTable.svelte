<script lang="ts">
  /** Payload section for transaction-style detail views. */
  import type { TransactionPayloadField, TransactionRenderer } from "$lib";

  interface Props {
    readonly payloadFields: readonly TransactionPayloadField[];
    readonly transactionRenderer: TransactionRenderer;
  }

  const { payloadFields, transactionRenderer }: Props = $props();
</script>

{#if transactionRenderer.renderMode !== "payment" && payloadFields.length > 0}
  <div class="hy-detail-payload-table">
    {#each payloadFields as field (field.label)}
      <span class="payload-key">{field.label}</span>
      <span class="payload-val" class:mono={field.mono ?? field.value.length > 20} class:muted={field.muted ?? false}>
        {field.value}
      </span>
    {/each}
  </div>
{/if}

<style>
  .payload-key {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .payload-val {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  .muted {
    color: var(--text-muted);
  }

  .mono {
    font-family: var(--font-mono);
  }
</style>
