<script lang="ts">
  /** Pill for monospace hashes and compact identifiers. */
  interface Props {
    readonly value: string;
    readonly truncate?: boolean;
  }

  const { value, truncate = true }: Props = $props();

  const displayValue = $derived.by((): string => {
    if (!truncate || value.length <= 18) {
      return value;
    }
    return `${value.slice(0, 10)}…${value.slice(-8)}`;
  });
</script>

<code class="hash-pill" title={value}>{displayValue}</code>

<style>
  .hash-pill {
    background: var(--surface-raised);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-pill);
    color: var(--color-fg);
    display: inline-flex;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 500;
    max-width: 100%;
    overflow: hidden;
    padding: 8px 14px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
