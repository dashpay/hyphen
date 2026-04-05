<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount, tick } from "svelte";

  import type { ApiNetworkResponse, ApiProposalDetail } from "$lib";
  import { getNetwork, getProposals, getStatus } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";
  import { HyPrExplorer } from "$lib/proposal/views";
  import { HyPageFrame, HyPageState } from "$lib/views";

  let proposals = $state<readonly ApiProposalDetail[]>([]);
  let network = $state<ApiNetworkResponse | null>(null);
  let blockHeight = $state<number | null>(null);
  let loading = $state(true);
  let loadingMore = $state(false);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let nextCursor = $state<string | null>(null);
  let totalCount = $state<number | null>(null);
  let hasRevealedInitialHash = $state(false);

  const expandedHash = $derived.by((): string | null => {
    return page.url.searchParams.get("hash");
  });

  function getProposalScrollNode(): HTMLDivElement | null {
    const tableScrollBody = globalThis.document.querySelector(".proposals-list .scroll-body");
    if (tableScrollBody instanceof HTMLDivElement && !tableScrollBody.classList.contains("page-mode")) {
      return tableScrollBody;
    }

    const pageFrame = globalThis.document.querySelector(".page-frame");
    if (pageFrame instanceof HTMLDivElement) {
      return pageFrame;
    }

    return null;
  }

  async function restoreProposalScroll(scrollTop: number | null): Promise<void> {
    if (scrollTop == null) {
      return;
    }

    await tick();
    globalThis.requestAnimationFrame((): void => {
      const scrollNode = getProposalScrollNode();
      if (scrollNode != null) {
        scrollNode.scrollTop = scrollTop;
      }
    });
  }

  function toggleProposal(hash: string): void {
    const nextUrl = new URL(page.url);

    if (expandedHash === hash) {
      nextUrl.searchParams.delete("hash");
    } else {
      nextUrl.searchParams.set("hash", hash);
    }

    const scrollTop = getProposalScrollNode()?.scrollTop ?? null;
    void goto(nextUrl.pathname + nextUrl.search, { keepFocus: true, noScroll: true, replaceState: true }).then(
      async (): Promise<void> => {
        await restoreProposalScroll(scrollTop);
      }
    );
  }

  onMount((): void => {
    void Promise.allSettled([getProposals(100), getStatus(), getNetwork()])
      .then((results): void => {
        const proposalsResult = results[0];
        const statusResult = results[1];
        const networkResult = results[2];

        if (proposalsResult.status === "fulfilled") {
          proposals = proposalsResult.value.value;
          totalCount = proposalsResult.value.totalCount;
          nextCursor = proposalsResult.value.nextCursor ?? null;
        } else {
          throw proposalsResult.reason;
        }

        if (statusResult.status === "fulfilled") {
          blockHeight = statusResult.value.blockHeight;
        }

        if (networkResult.status === "fulfilled") {
          network = networkResult.value;
        }
      })
      .catch((caught: unknown): void => {
        errorMessage = HyUtilReadErrorMessage(caught);
        errorStatus = HyUtilReadErrorStatus(caught);
        nextCursor = null;
        totalCount = null;
      })
      .finally((): void => {
        loading = false;
      });
  });

  $effect((): void => {
    const hash = expandedHash;
    if (hash == null) {
      hasRevealedInitialHash = false;
      return;
    }

    if (hasRevealedInitialHash) {
      return;
    }

    hasRevealedInitialHash = true;
    void tick().then((): void => {
      const targetRow = globalThis.document.querySelector(`.p-row[data-proposal-hash="${hash}"]`);
      if (targetRow instanceof HTMLElement) {
        targetRow.scrollIntoView({ block: "nearest" });
      }
    });
  });

  async function loadMore(): Promise<void> {
    if (loadingMore || nextCursor == null) {
      return;
    }

    loadingMore = true;

    try {
      const response = await getProposals(100, nextCursor);
      proposals = [...proposals, ...response.value];
      totalCount = response.totalCount;
      nextCursor = response.nextCursor ?? null;
    } catch {
      nextCursor = null;
    } finally {
      loadingMore = false;
    }
  }
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Proposals"))}</title>
</svelte:head>

<HyPageFrame>
  <HyPageState empty={false} emptyMessage="" emptyTitle="" {errorMessage} {errorStatus} {loading}>
    <HyPrExplorer
      {blockHeight}
      {expandedHash}
      hasMore={nextCursor != null}
      {loadingMore}
      {network}
      onloadmore={loadMore}
      {proposals}
      {totalCount}
      onProposalToggle={toggleProposal}
    />
  </HyPageState>
</HyPageFrame>
