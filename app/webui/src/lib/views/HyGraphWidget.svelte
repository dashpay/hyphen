<script lang="ts">
  /** Shared graph card and overlay controller for metric history views. */
  import { onMount } from "svelte";
  import type { Component } from "svelte";

  import type { ChartPoint } from "$lib/data/chart";
  import type { TimeRange } from "$lib/data/hashrate";

  import HyChartPanel from "./HyChartPanel.svelte";
  import HyGraphCounter from "./HyGraphCounter.svelte";
  import HyGraphOverlay from "./HyGraphOverlay.svelte";
  import { HyGraphGetPresentationMode } from "./HyGraphUtil";

  interface HyGraphRangeData {
    readonly bucketBlocks: number;
    readonly points: readonly ChartPoint[];
  }

  interface Props {
    readonly label: string;
    readonly value: string;
    readonly icon: Component;
    readonly data: readonly ChartPoint[];
    readonly blockTimeSeconds: number;
    readonly loadRange: (range: TimeRange) => Promise<HyGraphRangeData>;
    readonly valueFormatter?: ((value: number) => string) | undefined;
  }

  const { label, value, icon, data, blockTimeSeconds, loadRange, valueFormatter }: Props = $props();

  let isOpen = $state(false);
  let overlayData = $state<readonly ChartPoint[]>([]);
  let overlayBucketBlocks = $state(1);
  let overlayRange = $state<TimeRange>("1h");
  let viewportWidth = $state(1024);

  const presentationMode = $derived(HyGraphGetPresentationMode(viewportWidth));

  function syncViewportWidth(): void {
    viewportWidth = globalThis.innerWidth;
  }

  onMount(() => {
    syncViewportWidth();
  });

  async function openGraph(): Promise<void> {
    overlayRange = "1h";
    overlayData = [];
    overlayBucketBlocks = 1;

    const next = await loadRange("1h");
    overlayData = next.points;
    overlayBucketBlocks = next.bucketBlocks;
    isOpen = true;
  }

  function closeGraph(): void {
    isOpen = false;
  }

  async function handleRangeChange(range: TimeRange): Promise<void> {
    overlayRange = range;
    const next = await loadRange(range);
    overlayData = next.points;
    overlayBucketBlocks = next.bucketBlocks;
  }
</script>

<svelte:window
  onkeydown={(event: KeyboardEvent): void => {
    if (isOpen && event.key === "Escape") {
      closeGraph();
    }
  }}
  onresize={syncViewportWidth}
/>

<HyGraphCounter
  {data}
  {icon}
  {label}
  onExpand={(): void => {
    void openGraph();
  }}
  {value}
/>

{#if isOpen}
  <HyGraphOverlay fullscreen={presentationMode === "fullscreen"} onclick={closeGraph}>
    <HyChartPanel
      {blockTimeSeconds}
      bucketBlocks={overlayBucketBlocks}
      data={overlayData}
      {label}
      onclose={closeGraph}
      onrangechange={(range: TimeRange): void => {
        void handleRangeChange(range);
      }}
      selectedRange={overlayRange}
      {value}
      {valueFormatter}
    />
  </HyGraphOverlay>
{/if}
