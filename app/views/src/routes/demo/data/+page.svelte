<script lang="ts">
  import type { Snippet } from "svelte";
  import { createRawSnippet } from "svelte";
  import {
    VwBarChart,
    VwDonutChart,
    VwGraphCard,
    VwHStack,
    VwPanel,
    VwStatCard,
    VwTable,
    VwText,
    VwVStack,
  } from "$lib";

  interface BlockRow {
    readonly height: number;
    readonly hash: string;
    readonly txs: number;
  }

  const blockRows: BlockRow[] = [
    { height: 2241101, hash: "00000000f2c4c671...fe15d0ab", txs: 14 },
    { height: 2241100, hash: "00000000e1f4d827...2cb88d16", txs: 21 },
    { height: 2241099, hash: "00000000bc5a1e20...91da20c4", txs: 17 },
  ];

  function escapeHtml(value: string): string {
    return value
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  function textSnippet(value: string): Snippet {
    return createRawSnippet(() => ({
      render: (): string => escapeHtml(value),
    }));
  }

  const columns = [
    {
      key: "height",
      label: "Height",
      render: (row: BlockRow): Snippet => textSnippet(String(row.height)),
    },
    {
      key: "hash",
      label: "Block Hash",
      render: (row: BlockRow): Snippet => textSnippet(row.hash),
    },
    {
      key: "txs",
      label: "Transactions",
      align: "right" as const,
      render: (row: BlockRow): Snippet => textSnippet(String(row.txs)),
    },
  ] as const;
</script>

<svelte:head>
  <title>hyphen-views | Data</title>
</svelte:head>

<VwVStack gap={24}>
  <VwText size="3xl" weight="extrabold">Tables and Charts</VwText>

  <VwHStack gap={16} wrap>
    <VwStatCard label="Supply" value="11.9M CUR" meta="Tracked output set" />
    <VwGraphCard label="Transactions" value="1,248" meta="Last 24 hours" points={[14, 18, 21, 17, 24, 29, 27]} />
  </VwHStack>

  <VwPanel padding={24}>
    <VwVStack gap={20}>
      <VwText size="xl" weight="bold">Typed VwTable</VwText>
      <VwTable {columns} getRowKey={(row: BlockRow): string => String(row.height)} rows={blockRows} />
    </VwVStack>
  </VwPanel>

  <VwHStack gap={16} wrap>
    <VwPanel padding={24}>
      <VwVStack gap={20}>
        <VwText size="xl" weight="bold">Bar Chart</VwText>
        <VwBarChart
          items={[
            { label: "Top 10", value: 42, displayValue: "42%" },
            { label: "Top 50", value: 67, displayValue: "67%" },
            { label: "Top 100", value: 79, displayValue: "79%" },
          ]}
        />
      </VwVStack>
    </VwPanel>

    <VwPanel padding={24}>
      <VwVStack gap={20}>
        <VwText size="xl" weight="bold">Donut Chart</VwText>
        <VwDonutChart
          centerLabel="Distribution"
          centerSubvalue="Top holders"
          centerValue="67%"
          segments={[
            { label: "Top 10", value: 42 },
            { label: "Top 50", value: 25 },
            { label: "Rest", value: 33 },
          ]}
        />
      </VwVStack>
    </VwPanel>
  </VwHStack>
</VwVStack>
