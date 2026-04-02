<script lang="ts">
  import { VwDivider, VwHStack, VwPanel, VwText, VwVStack } from "$lib";

  const swatches = [
    ["Background", "var(--color-bg)"],
    ["Foreground", "var(--color-fg)"],
    ["Surface", "var(--surface-base)"],
    ["Raised", "var(--surface-raised)"],
    ["Accent Blue", "var(--accent-blue)"],
    ["Accent Mint", "var(--accent-mint)"],
  ] as const;
</script>

<svelte:head>
  <title>hyphen-views | Typography</title>
</svelte:head>

<VwVStack gap={24}>
  <VwText size="3xl" weight="extrabold">Typography and Tokens</VwText>
  <VwText size="md+" color="muted">
    This route shows the baseline text scale and the core surface tokens that the rest of the explorer primitives build
    on.
  </VwText>

  <VwPanel variant="glass" border padding={24}>
    <VwVStack gap={16}>
      <VwText size="2xl" weight="bold">VwText Scale</VwText>
      <VwDivider />
      <VwVStack gap={12}>
        <VwText size="xs">Extra small metadata copy</VwText>
        <VwText size="sm">Small support text for labels</VwText>
        <VwText size="base">Base body copy for dense explorer rows</VwText>
        <VwText size="md+">Medium emphasis for cards and summaries</VwText>
        <VwText size="xl" weight="semibold">Large section headings</VwText>
        <VwText size="3xl" weight="extrabold">Hero scale for critical metrics</VwText>
      </VwVStack>
    </VwVStack>
  </VwPanel>

  <VwPanel variant="card" padding={24}>
    <VwVStack gap={16}>
      <VwText size="2xl" weight="bold">Surface Tokens</VwText>
      <VwDivider />
      <VwHStack gap={16} wrap>
        {#each swatches as [label, color]}
          <div class="vw-demo-swatch">
            <div class="vw-demo-chip" style={`background: ${color};`}></div>
            <VwText size="sm" weight="semibold">{label}</VwText>
            <VwText size="xs" color="muted">{color}</VwText>
          </div>
        {/each}
      </VwHStack>
    </VwVStack>
  </VwPanel>
</VwVStack>
