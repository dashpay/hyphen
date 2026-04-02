<script lang="ts">
  /** Divider used to separate adjacent layout regions. */
  interface Props {
    readonly direction?: "horizontal" | "vertical";
    readonly opacity?: number;
  }

  const { direction = "horizontal", opacity = 0.5 }: Props = $props();

  const computedStyle = $derived(
    direction === "horizontal"
      ? `height: 1px; width: 100%; opacity: ${String(opacity)}`
      : `width: 1px; height: 100%; opacity: ${String(opacity)}`
  );
</script>

<div class="divider" style={computedStyle} role="separator" aria-orientation={direction}></div>

<style>
  .divider {
    background: var(--color-fg);
    flex-shrink: 0;
  }
</style>
