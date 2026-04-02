<script lang="ts">
  /** Shared list-browser page shell for history routes with pager navigation. */
  import type { Snippet } from "svelte";
  import { VwErrorState, VwThrobber } from "@repo/hyphen-views";

  import HyCenteredState from "./HyCenteredState.svelte";
  import HyHttpError from "./HyHttpError.svelte";
  import HyListPanel from "./HyListPanel.svelte";
  import HyPageFrame from "./HyPageFrame.svelte";

  interface Props {
    readonly children?: Snippet;
    readonly errorMessage: string | null;
    readonly errorStatus?: number | null;
    readonly fullHeight?: boolean;
    readonly loading: boolean;
    readonly subtitle: string;
    readonly title: string;
  }

  const { children, errorMessage, errorStatus = null, fullHeight = true, loading, subtitle, title }: Props = $props();
</script>

<HyPageFrame>
  <HyListPanel {fullHeight} {subtitle} {title}>
    {#if loading}
      <HyCenteredState><VwThrobber /></HyCenteredState>
    {:else if errorMessage != null}
      {#if errorStatus != null}
        <HyHttpError message={errorMessage} status={errorStatus} />
      {:else}
        <HyCenteredState><VwErrorState message={errorMessage} /></HyCenteredState>
      {/if}
    {:else}
      {@render children?.()}
    {/if}
  </HyListPanel>
</HyPageFrame>
