<script lang="ts">
  /** Shared full-page detail route state wrapper for loading, error, missing, and content states. */
  import type { Snippet } from "svelte";
  import { VwErrorState, VwThrobber } from "@repo/hyphen-views";

  import HyCenteredState from "./HyCenteredState.svelte";
  import HyHttpError from "./HyHttpError.svelte";

  interface Props {
    readonly children?: Snippet;
    readonly errorMessage?: string | null;
    readonly errorStatus?: number | null;
    readonly loading: boolean;
    readonly missing: boolean;
    readonly missingLabel: string;
    readonly missingMessage?: string;
  }

  const {
    children,
    errorMessage = null,
    errorStatus = null,
    loading,
    missing,
    missingLabel,
  }: Props = $props();
</script>

{#if loading}
  <HyCenteredState><VwThrobber /></HyCenteredState>
{:else if errorMessage != null}
  {#if errorStatus != null}
    <HyHttpError message={errorMessage} status={errorStatus} />
  {:else}
    <HyCenteredState><VwErrorState message={errorMessage} /></HyCenteredState>
  {/if}
{:else if missing}
  <HyHttpError message={missingLabel} status={404} />
{:else}
  {@render children?.()}
{/if}
