<script lang="ts">
  import { VwExplorerSummaryList, VwPage, VwPanel, VwSection, VwText, VwVStack } from "$lib";

  const demos = [
    {
      id: "typography",
      title: "Typography and Tokens",
      subtitle: "/demo/typography",
      meta: "Baseline text roles and color surfaces",
    },
    {
      id: "layout",
      title: "Layout Primitives",
      subtitle: "/demo/layout",
      meta: "Shell, header, page and section composition",
    },
    {
      id: "data",
      title: "Tables and Charts",
      subtitle: "/demo/data",
      meta: "Typed tables, stat cards and distribution charts",
    },
    {
      id: "feedback",
      title: "Feedback States",
      subtitle: "/demo/feedback",
      meta: "Loading, empty, error and not-found surfaces",
    },
    {
      id: "explorer",
      title: "Mock Explorer",
      subtitle: "/demo/explorer",
      meta: "Full lorem-ipsum explorer composition",
    },
  ] as const;
</script>

<svelte:head>
  <title>hyphen-views</title>
</svelte:head>

<VwPage
  eyebrow="Views"
  title="Declarative Explorer Primitives"
  subtitle="This project is the neutral component layer for the explorer rewrite. Each demo route exercises one slice of the final UI."
>
  <VwSection
    title="Demo Routes"
    subtitle="Use these routes to inspect the primitives in isolation before the web UI starts consuming them."
  >
    <VwPanel padding={24}>
      <VwVStack gap={16}>
        <VwText size="lg" weight="semibold">Available Showcases</VwText>
        <VwExplorerSummaryList items={demos} />
      </VwVStack>
    </VwPanel>
  </VwSection>
</VwPage>
