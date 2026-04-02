<script lang="ts">
  import type { Snippet } from "svelte";
  import { createRawSnippet } from "svelte";
  import {
    VwAppShell,
    VwBarChart,
    VwButton,
    VwDisclosurePanel,
    VwDivider,
    VwDonutChart,
    VwExplorerSummaryList,
    VwGraphCard,
    VwHashPill,
    VwHeader,
    VwHStack,
    VwKeyValueList,
    VwPage,
    VwPanel,
    VwSection,
    VwSidebar,
    VwStatCard,
    VwTable,
    VwText,
    VwTransactionIoRow,
    VwVStack,
  } from "$lib";

  interface BlockRow {
    readonly height: number;
    readonly hash: string;
    readonly transactions: number;
    readonly value: string;
  }

  function textSnippet(value: string): Snippet {
    return createRawSnippet(() => ({
      render: (): string => value,
    }));
  }

  const blocks: BlockRow[] = [
    { height: 2241101, hash: "00000000f2c4c67195f1d90a4fe15d0ab93b4f4a", transactions: 14, value: "842.13 CUR" },
    { height: 2241100, hash: "00000000e1f4d8278ae1453c12cb88d16757d011", transactions: 21, value: "901.77 CUR" },
    { height: 2241099, hash: "00000000bc5a1e20aa449f2e91da20c43ab8f999", transactions: 17, value: "799.01 CUR" },
  ];

  const blockColumns = [
    {
      key: "height",
      label: "Height",
      render: (row: BlockRow): Snippet => textSnippet(String(row.height)),
    },
    {
      key: "hash",
      label: "Hash",
      // Demo data is static here; escape values before reusing this snippet with dynamic content.
      render: (row: BlockRow): Snippet =>
        createRawSnippet(() => ({
          render: (): string =>
            `<code class="vw-demo-table-hash">${row.hash.slice(0, 12)}…${row.hash.slice(-8)}</code>`,
        })),
    },
    {
      key: "transactions",
      label: "Transactions",
      align: "right" as const,
      render: (row: BlockRow): Snippet => textSnippet(String(row.transactions)),
    },
    {
      key: "value",
      label: "Value",
      align: "right" as const,
      render: (row: BlockRow): Snippet => textSnippet(row.value),
    },
  ] as const;

  const recentTransactions = [
    {
      id: "tx-1",
      title: "Coinbase Reward",
      subtitle: "4f1f7b0a6c9d7ea3b1452c9118a9f2db",
      meta: "Block 2241101",
      trailing: "2.88 CUR",
    },
    {
      id: "tx-2",
      title: "Asset Lock",
      subtitle: "bc12f13ad9d02e774cfab2d5f0ffcc01",
      meta: "Bridge settlement",
      trailing: "125.00 CUR",
    },
    {
      id: "tx-3",
      title: "Validator Update",
      subtitle: "ff102aa9c2d2e13d6c9b44ae00d145fa",
      meta: "ProTx rotation",
      trailing: "0.01 CUR",
    },
  ] as const;

  const balances = [
    { label: "Top 10 addresses", value: 42, displayValue: "42%" },
    { label: "Top 100 addresses", value: 67, displayValue: "67%" },
    { label: "Top 1,000 addresses", value: 84, displayValue: "84%" },
  ] as const;
</script>

<svelte:head>
  <title>hyphen-views | Mock Explorer</title>
</svelte:head>

<VwAppShell>
  {#snippet sidebar()}
    <VwSidebar>
      <VwVStack gap={16}>
        <VwText size="xl" weight="extrabold">hyphen</VwText>
        <VwText size="sm" color="muted">Overview</VwText>
        <VwText size="sm" color="muted">Blocks</VwText>
        <VwText size="sm" color="muted">Transactions</VwText>
        <VwText size="sm" color="muted">Addresses</VwText>
        <VwText size="sm" color="muted">Validators</VwText>
      </VwVStack>
    </VwSidebar>
  {/snippet}

  <VwVStack gap={20}>
    <VwHeader>
      {#snippet left()}
        <VwVStack gap={4}>
          <VwText size="lg" weight="extrabold">Currency Explorer</VwText>
          <VwText size="sm" color="muted">Lorem ipsum network summary with filler metadata.</VwText>
        </VwVStack>
      {/snippet}
      {#snippet right()}
        <VwHStack gap={12}>
          <VwButton variant="ghost">7d</VwButton>
          <VwButton variant="ghost">30d</VwButton>
          <VwButton variant="primary">Refresh</VwButton>
        </VwHStack>
      {/snippet}
    </VwHeader>

    <VwPage
      eyebrow="Mock Explorer"
      title="Overview"
      subtitle="Lorem ipsum dolor sit amet, consectetur adipiscing elit. The layout mirrors the final explorer without binding to live API data yet."
    >
      <VwSection title="Headline Metrics" subtitle="Static values standing in for the live network summary.">
        <VwHStack gap={16} wrap>
          <VwStatCard label="Latest Height" value="2,241,101" meta="3 seconds ago" />
          <VwGraphCard
            label="Transactions"
            value="1,248"
            meta="24h rolling count"
            points={[14, 18, 21, 19, 28, 32, 29]}
          />
          <VwGraphCard
            label="Value Moved"
            value="12.7K CUR"
            meta="24h rolling value"
            points={[420, 500, 460, 700, 810, 920, 860]}
          />
        </VwHStack>
      </VwSection>

      <VwSection title="Block Feed" subtitle="Recent blocks with placeholder values and lorem-style identifiers.">
        <VwTable columns={blockColumns} getRowKey={(row: BlockRow): string => String(row.height)} rows={blocks} />
      </VwSection>

      <VwHStack gap={16} wrap>
        <VwPanel padding={24}>
          <VwVStack gap={20}>
            <VwText size="xl" weight="bold">Latest Transactions</VwText>
            <VwExplorerSummaryList items={recentTransactions} />
          </VwVStack>
        </VwPanel>

        <VwPanel padding={24}>
          <VwVStack gap={20}>
            <VwText size="xl" weight="bold">Holder Concentration</VwText>
            <VwDonutChart
              centerLabel="Tracked"
              centerSubvalue="Top holders"
              centerValue="67%"
              segments={[
                { label: "Top 10", value: 42 },
                { label: "Top 100", value: 25 },
                { label: "Rest", value: 33 },
              ]}
            />
            <VwBarChart items={balances} />
          </VwVStack>
        </VwPanel>
      </VwHStack>

      <VwSection
        title="Transaction Detail Preview"
        subtitle="Explorer-specific row and metadata primitives working together."
      >
        <VwPanel padding={24}>
          <VwVStack gap={20}>
            <VwHStack gap={12} wrap>
              <VwText size="lg" weight="bold">Transaction</VwText>
              <VwHashPill value="4f1f7b0a6c9d7ea3b1452c9118a9f2dbcf0e1284b2960e8f44d1ff99b5ce1120" />
            </VwHStack>
            <VwDivider />
            <VwKeyValueList
              items={[
                { key: "fee", label: "Fee", value: "0.0001 CUR" },
                { key: "size", label: "Size", value: "612 bytes" },
                { key: "type", label: "Type", value: "Asset Lock" },
                { key: "block", label: "Block", value: "2241101" },
              ]}
            />
            <VwDivider />
            <VwTransactionIoRow
              address="Xv8PvY2cJ0uL5oQ4VnP7RtM2Bq9DxT7YzA"
              amount="125.00 CUR"
              label="VwInput 0"
              meta="Spend from wallet cluster"
            />
            <VwTransactionIoRow
              address="Xw1MnB5dJ7pQ2rS4tV8yUaC6Lm0NzP3QeF"
              amount="124.99 CUR"
              label="Output 0"
              meta="Bridge escrow destination"
            />
          </VwVStack>
        </VwPanel>
      </VwSection>

      <VwSection
        title="Expandable Analysis"
        subtitle="Disclosure panels preview the explorer’s long-form auxiliary details."
      >
        <VwDisclosurePanel summary="Lorem ipsum payload summary" title="Decoded payload preview">
          <VwText size="base">
            Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et
            dolore magna aliqua.
          </VwText>
        </VwDisclosurePanel>
      </VwSection>
    </VwPage>
  </VwVStack>
</VwAppShell>
