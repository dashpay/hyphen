<script lang="ts">
  /** Row for transaction inputs and outputs. */
  interface Props {
    readonly label: string;
    readonly address: string;
    readonly addressHref?: string | undefined;
    readonly amount: string;
    readonly meta?: string | undefined;
  }

  const { label, address, addressHref, amount, meta }: Props = $props();
</script>

<div class="io-row">
  <div class="copy">
    <span class="label">{label}</span>
    {#if addressHref == null}
      <span class="address">{address}</span>
    {:else}
      <a class="address address-link" href={addressHref}>{address}</a>
    {/if}
    {#if meta}
      <span class="meta">{meta}</span>
    {/if}
  </div>
  <span class="amount">{amount}</span>
</div>

<style>
  .io-row {
    align-items: center;
    display: flex;
    gap: var(--space-4);
    justify-content: space-between;
    min-height: 58px;
    padding: 0 16px;
    transition: background var(--duration-fast) var(--ease-default);
  }

  .io-row:hover {
    background: var(--surface-raised);
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
  }

  .label {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .address {
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 300;
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .address-link {
    text-decoration: none;
  }

  .address-link:hover,
  .address-link:focus-visible {
    text-decoration: underline;
  }

  .meta {
    color: var(--text-tertiary);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 400;
  }

  .amount {
    color: var(--color-fg);
    flex-shrink: 0;
    font-family: var(--font-primary);
    font-size: var(--text-md);
    font-weight: 600;
  }
</style>
