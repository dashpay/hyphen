<script lang="ts">
  /** Table for current proposal vote rows. */
  import { tr } from "@repo/hyphen-i18n";
  import type { ApiProposalVote } from "$lib";
  import { HyUtilFormatFullDate } from "$lib/HyUtil";
  import {
    HyScrollBody,
    HyTableGetTemplate,
    HyTableGetVisibleColumns,
    HyTableHasColumn,
    HyTableHeaderRow,
    HyTableObserveWidth,
    HyTag,
    type HyTableColumnSpec,
  } from "$lib/views";

  interface Props {
    readonly votes: readonly ApiProposalVote[];
    readonly onvalidatorclick?: (proTxHash: string) => void;
  }

  type HyPrVoteColumnId = "protx" | "time" | "vote";

  const { votes, onvalidatorclick }: Props = $props();

  let containerWidth = $state(Number.POSITIVE_INFINITY);
  let tableWrap = $state<HTMLDivElement | null>(null);

  const baseColumns = $derived.by((): readonly HyTableColumnSpec<HyPrVoteColumnId>[] => [
    { id: "protx", label: "ProTxHash", flexible: true, minWidth: "260px" },
    { id: "vote", label: tr("Vote"), width: "96px" },
    { id: "time", label: tr("Time"), width: "176px", dropBelow: "tablet" },
  ]);
  const visibleColumns = $derived(HyTableGetVisibleColumns(baseColumns, containerWidth));
  const tableTemplate = $derived(HyTableGetTemplate(visibleColumns));

  $effect(() => {
    if (tableWrap == null) {
      return;
    }

    return HyTableObserveWidth(tableWrap, (width: number): void => {
      containerWidth = width;
    });
  });
</script>

<div bind:this={tableWrap} class="table-wrap">
  <HyTableHeaderRow columns={tableTemplate}>
    {#if HyTableHasColumn(visibleColumns, "protx")}
      <span class="col col-protx">ProTxHash</span>
    {/if}
    {#if HyTableHasColumn(visibleColumns, "vote")}
      <span class="col col-vote">{tr("Vote")}</span>
    {/if}
    {#if HyTableHasColumn(visibleColumns, "time")}
      <span class="col col-time">{tr("Time")}</span>
    {/if}
  </HyTableHeaderRow>

  <HyScrollBody>
    <div class="table-body" style:--hy-table-columns={tableTemplate}>
      {#each votes as vote (vote.proTxHash + ":" + (vote.voteTime ?? ""))}
        <div
          class="vote-row"
          onclick={(): void => {
            onvalidatorclick?.(vote.proTxHash);
          }}
          onkeydown={(event: KeyboardEvent): void => {
            if (event.key === "Enter") {
              onvalidatorclick?.(vote.proTxHash);
            }
          }}
          role="button"
          tabindex="0"
        >
          {#if HyTableHasColumn(visibleColumns, "protx")}
            <span class="col col-protx mono accent hy-truncate-text">{vote.proTxHash}</span>
          {/if}
          {#if HyTableHasColumn(visibleColumns, "vote")}
            <span class="col col-vote">
              <HyTag
                background={vote.vote === "yes"
                  ? "var(--hy-tag-coinbase-bg)"
                  : vote.vote === "no"
                    ? "var(--overlay-subtle)"
                    : "var(--surface-accent-subtle)"}
                foreground={vote.vote === "yes"
                  ? "var(--hy-tag-coinbase-fg)"
                  : vote.vote === "no"
                    ? "var(--text-muted)"
                    : "var(--accent-blue)"}
                label={vote.vote}
              />
            </span>
          {/if}
          {#if HyTableHasColumn(visibleColumns, "time")}
            <span class="col col-time muted">{vote.voteTime == null ? "—" : HyUtilFormatFullDate(vote.voteTime)}</span>
          {/if}
        </div>
      {/each}
    </div>
  </HyScrollBody>
</div>

<style>
  .table-wrap {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .table-body {
    align-content: start;
    column-gap: var(--hy-browser-column-gap);
    display: grid;
    grid-template-columns: var(--hy-table-columns);
    min-width: 0;
    width: 100%;
  }

  .vote-row {
    align-items: center;
    border-bottom: 1px solid var(--overlay-subtle);
    column-gap: var(--hy-browser-column-gap);
    cursor: pointer;
    display: grid;
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    padding: var(--hy-list-row-padding);
  }

  .vote-row:hover,
  .vote-row:focus-visible {
    background: var(--overlay-subtle);
  }

  .vote-row:focus-visible {
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .col {
    align-items: center;
    display: flex;
    font-size: var(--text-sm);
    min-width: 0;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .accent {
    color: var(--accent-blue);
  }

  .muted {
    color: var(--text-muted);
  }
</style>
