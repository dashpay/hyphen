<script lang="ts">
  /** Shared full-page async state wrapper for loading, error, empty, and content states. */
  import type { Snippet } from "svelte";
  import { tr } from "@repo/hyphen-i18n";

  import { VwEmptyState, VwErrorState, VwThrobber } from "@repo/hyphen-views";

  import HyCenteredState from "./HyCenteredState.svelte";
  import HyHttpError from "./HyHttpError.svelte";

  interface Props {
    readonly children?: Snippet;
    readonly empty: boolean;
    readonly emptyMessage: string;
    readonly emptyTitle: string;
    readonly errorMessage: string | null;
    readonly errorStatus?: number | null;
    readonly errorTitle?: string;
    readonly loading: boolean;
  }

  const {
    children,
    empty,
    emptyMessage,
    emptyTitle,
    errorMessage,
    errorStatus = null,
    errorTitle = tr("Request failed"),
    loading,
  }: Props = $props();
</script>

{#if loading}
  <HyCenteredState><VwThrobber /></HyCenteredState>
{:else if errorMessage != null}
  {#if errorStatus != null}
    <HyHttpError message={errorMessage} status={errorStatus} />
  {:else}
    <HyCenteredState><VwErrorState message={errorMessage} title={errorTitle} /></HyCenteredState>
  {/if}
{:else if empty}
  <HyCenteredState><VwEmptyState message={emptyMessage} title={emptyTitle} /></HyCenteredState>
{:else}
  {@render children?.()}
{/if}
