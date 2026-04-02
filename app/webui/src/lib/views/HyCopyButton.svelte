<script lang="ts">
  /** Copy button with the shared Hyphen inline control styling. */
  import IconCopy from "virtual:icons/mdi/content-copy";

  interface Props {
    readonly copied?: boolean;
    readonly inline?: boolean;
    readonly label: string;
    readonly onpress: (event: MouseEvent) => void;
    readonly title?: string;
    readonly copiedTitle?: string;
  }

  const { copied = false, inline = false, label, onpress, title = "Copy", copiedTitle = "Copied!" }: Props = $props();
</script>

<button
  aria-label={label}
  class="copy-button"
  class:inline
  onclick={onpress}
  title={copied ? copiedTitle : title}
  type="button"
>
  <IconCopy height="13" width="13" />
</button>

<style>
  .copy-button {
    align-items: center;
    background: none;
    border: none;
    border-radius: var(--hy-icon-button-radius);
    color: var(--text-muted);
    cursor: pointer;
    display: inline-flex;
    flex-shrink: 0;
    height: var(--hy-icon-button-size);
    justify-content: center;
    opacity: 0.5;
    padding: 0;
    transition:
      color var(--duration-fast) var(--ease-default),
      opacity var(--duration-fast) var(--ease-default);
    width: var(--hy-icon-button-size);
  }

  .copy-button:hover {
    color: var(--color-fg);
    opacity: 1;
  }

  .copy-button.inline {
    color: var(--accent-blue);
    opacity: 1;
    width: auto;
  }
</style>
