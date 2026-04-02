<script lang="ts">
  /** Fixed spacer for token-sized gaps in compositions. */
  interface Props {
    readonly size?: number | "flex";
  }

  const { size = "flex" }: Props = $props();

  const computedStyle = $derived(size === "flex" ? "flex: 1" : `flex: 0 0 ${String(size)}px`);
</script>

<div class="spacer" style={computedStyle} aria-hidden="true"></div>

<style>
  .spacer {
    pointer-events: none;
  }
</style>
