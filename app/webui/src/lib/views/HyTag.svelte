<script lang="ts">
  /** Shared Hyphen pill tag for compact labeled states and categories. */
  interface Props {
    readonly background: string;
    readonly foreground: string;
    readonly label: string;
  }

  const { background, foreground, label }: Props = $props();
</script>

<span class="hy-tag" style:--tag-bg={background} style:--tag-fg={foreground}>
  {label}
</span>

<style>
  .hy-tag {
    align-self: flex-start;
    align-items: center;
    background: var(--tag-bg);
    border-radius: 999px;
    color: var(--tag-fg);
    display: inline-flex;
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 700;
    justify-content: center;
    letter-spacing: 0.04em;
    line-height: 1;
    max-width: max-content;
    padding: var(--hy-detail-pill-padding);
    text-transform: uppercase;
    white-space: nowrap;
  }
</style>
