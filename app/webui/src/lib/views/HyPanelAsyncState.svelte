<script lang="ts">
  /** Shared panel-local async state wrapper for loading, error, empty, and content states. */
  import type { Snippet } from "svelte";
  import { tr } from "@repo/hyphen-i18n";
  import { VwEmptyState, VwErrorState, VwThrobber } from "@repo/hyphen-views";

  import HyPanelState from "./HyPanelState.svelte";
  import HyShellError from "./HyShellError.svelte";

  interface Props {
    readonly children?: Snippet;
    readonly empty?: boolean;
    readonly emptyMessage?: string;
    readonly emptyTitle?: string;
    readonly errorMessage?: string | null;
    readonly errorStatus?: number | null;
    readonly errorTitle?: string;
    readonly loading?: boolean;
  }

  const {
    children,
    empty = false,
    emptyMessage = tr("No data."),
    emptyTitle = tr("Nothing to show yet"),
    errorMessage = null,
    errorStatus = null,
    errorTitle = tr("Request failed"),
    loading = false,
  }: Props = $props();
</script>

{#if loading}
  <HyPanelState>
    <VwThrobber />
  </HyPanelState>
{:else if errorMessage != null}
  <HyPanelState>
    {#if errorStatus != null}
      <HyShellError message={errorMessage} title={`HTTP ${String(errorStatus)}`} />
    {:else}
      <VwErrorState message={errorMessage} title={errorTitle} />
    {/if}
  </HyPanelState>
{:else if empty}
  <HyPanelState>
    <VwEmptyState message={emptyMessage} title={emptyTitle} />
  </HyPanelState>
{:else}
  {@render children?.()}
{/if}
