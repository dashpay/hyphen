<script lang="ts">
  import { onMount } from "svelte";
  import { tr } from "@repo/hyphen-i18n";

  import type { ApiMetrics, ApiMempoolSummary, ApiNetworkResponse, ApiValidatorSummary } from "$lib";
  import { getMetricsHistory, getMempool, getNetwork, getValidators, getWebUiConfig } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";
  import { HyNtDashboard } from "$lib/network/views";
  import { HyPageFrame, HyPageState } from "$lib/views";

  let snapshot = $state<ApiNetworkResponse | null>(null);
  let mempool = $state<ApiMempoolSummary | null>(null);
  let validators = $state<readonly ApiValidatorSummary[]>([]);
  let metricsHistory = $state<readonly ApiMetrics[]>([]);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let loading = $state(true);
  const validatorsEnabled = $derived(getWebUiConfig().features.validators.enabled);

  async function readAllValidators(): Promise<readonly ApiValidatorSummary[]> {
    const allValidators: ApiValidatorSummary[] = [];
    let cursor: string | null = null;

    do {
      const response = await getValidators(100, cursor ?? undefined);
      allValidators.push(...response.value);
      cursor = response.nextCursor ?? null;
    } while (cursor != null);

    return allValidators;
  }

  async function loadNetworkSnapshot(): Promise<void> {
    try {
      snapshot = await getNetwork();
      errorMessage = null;
      errorStatus = null;
    } catch (caught: unknown) {
      errorMessage = HyUtilReadErrorMessage(caught);
      errorStatus = HyUtilReadErrorStatus(caught);
    }
  }

  async function loadOptionalDashboardData(): Promise<void> {
    const tasks: Promise<void>[] = [
      getMempool().then((nextMempool): void => {
        mempool = nextMempool;
      }),
      getMetricsHistory(undefined, 300).then((metricsResponse): void => {
        metricsHistory = metricsResponse.value;
      }),
    ];

    if (validatorsEnabled) {
      tasks.push(
        readAllValidators().then((nextValidators): void => {
          validators = nextValidators;
        })
      );
    }

    await Promise.allSettled(tasks);
  }

  onMount((): void => {
    void loadNetworkSnapshot()
      .then(async (): Promise<void> => {
        if (errorMessage != null) {
          return;
        }

        await loadOptionalDashboardData();
      })
      .finally((): void => {
        loading = false;
      });
  });
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Network"))}</title>
</svelte:head>

<HyPageFrame>
  <HyPageState
    empty={snapshot == null}
    emptyMessage="No live network data is available."
    emptyTitle="No network data"
    {errorMessage}
    {errorStatus}
    {loading}
  >
    {#if snapshot != null}
      <HyNtDashboard {mempool} {metricsHistory} {snapshot} {validators} {validatorsEnabled} />
    {/if}
  </HyPageState>
</HyPageFrame>
