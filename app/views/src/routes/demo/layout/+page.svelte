<script lang="ts">
  import {
    VwAppShell,
    VwButton,
    VwDivider,
    VwHeader,
    VwHStack,
    VwPage,
    VwPanel,
    VwSection,
    VwSidebar,
    VwText,
    VwVStack,
  } from "$lib";
</script>

<svelte:head>
  <title>hyphen-views | Layout</title>
</svelte:head>

<VwAppShell>
  {#snippet sidebar()}
    <VwSidebar>
      <VwVStack gap={12}>
        <VwText size="xl" weight="bold">Explorer</VwText>
        <VwText size="sm" color="muted">Overview</VwText>
        <VwText size="sm" color="muted">Blocks</VwText>
        <VwText size="sm" color="muted">Transactions</VwText>
      </VwVStack>
    </VwSidebar>
  {/snippet}

  <VwVStack gap={20}>
    <VwHeader>
      {#snippet left()}
        <VwText size="lg" weight="extrabold">hyphen views</VwText>
      {/snippet}
      {#snippet right()}
        <VwHStack gap={12}>
          <VwButton variant="ghost">Refresh</VwButton>
          <VwButton variant="primary">Connect</VwButton>
        </VwHStack>
      {/snippet}
    </VwHeader>

    <VwPage
      eyebrow="Demo"
      title="Layout Primitives"
      subtitle="The shell, page and section primitives compose the explorer’s larger page structure."
    >
      <VwSection title="Card Surfaces" subtitle="Neutral containers for explorer dashboards and detail pages.">
        <VwHStack gap={16} wrap>
          <VwPanel variant="card" padding={24}>
            <VwText size="lg" weight="semibold">Card surface</VwText>
          </VwPanel>
          <VwPanel variant="glass" border padding={24}>
            <VwText size="lg" weight="semibold">Glass surface</VwText>
          </VwPanel>
          <VwPanel variant="tinted" padding={24}>
            <VwText size="lg" weight="semibold">Tinted surface</VwText>
          </VwPanel>
        </VwHStack>
      </VwSection>

      <VwDivider />

      <VwSection title="Stack Composition" subtitle="Simple layout composition using the existing stack primitives.">
        <VwVStack gap={12}>
          <VwText size="md+">Primary detail column</VwText>
          <VwHStack gap={12} wrap>
            <VwPanel padding={20}><VwText>Left panel</VwText></VwPanel>
            <VwPanel padding={20}><VwText>Center panel</VwText></VwPanel>
            <VwPanel padding={20}><VwText>Right panel</VwText></VwPanel>
          </VwHStack>
        </VwVStack>
      </VwSection>
    </VwPage>
  </VwVStack>
</VwAppShell>
