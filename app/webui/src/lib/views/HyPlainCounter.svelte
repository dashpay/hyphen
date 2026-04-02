<script lang="ts">
  /** Metric counter card without embedded history graphics. */
  import type { Component } from "svelte";

  import { VwPanel, VwText, VwVStack } from "@repo/hyphen-views";

  import { HyAnHoverGloss } from "../anim";
  import HyAmountText from "./HyAmountText.svelte";

  interface Props {
    readonly label: string;
    readonly value: string;
    readonly icon: Component;
  }

  const { label, value, icon: Icon }: Props = $props();
</script>

<div class="counter-wrap" use:HyAnHoverGloss>
  <VwPanel border={true} padding={16} radius="sm" variant="glass">
    <VwVStack gap={8} justify="center">
      <VwText color="muted" size="sm" weight="medium">{label}</VwText>
      <VwText size="xl" weight="extrabold">
        <HyAmountText {value} />
      </VwText>
    </VwVStack>
    <div class="bg-icon">
      <Icon color="var(--accent-blue)" height="120" width="120" />
    </div>
  </VwPanel>
</div>

<style>
  .counter-wrap {
    height: 100%;
    min-width: 0;
  }

  .counter-wrap :global(.text) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .counter-wrap :global(.panel) {
    height: 100%;
    overflow: hidden;
    position: relative;
  }

  :global(.counter-wrap.glass-tap) {
    animation: glass-tap 0.4s ease;
  }

  @keyframes glass-tap {
    0% {
      scale: 1;
    }
    30% {
      scale: 0.97;
    }
    60% {
      scale: 1.02;
    }
    100% {
      scale: 1;
    }
  }

  .bg-icon {
    -webkit-mask-image: linear-gradient(to right, transparent, black 80%);
    mask-image: linear-gradient(to right, transparent, black 80%);
    opacity: 0.2;
    pointer-events: none;
    position: absolute;
    right: var(--hy-plain-counter-icon-offset-x);
    top: 50%;
    transform: translateY(-50%);
  }

  .bg-icon > :global(svg) {
    paint-order: stroke;
    stroke: var(--color-card-surface);
    stroke-width: var(--hy-graph-counter-icon-stroke-width);
  }
</style>
