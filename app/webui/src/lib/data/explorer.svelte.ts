import { getBlocks, getMetricsLatest, getStatus } from "../api";
import type { ApiBlockSummary, ApiMetrics, ApiStatus } from "../api";

export type ExplorerLoadState = "idle" | "loading" | "ready" | "error";

class ExplorerStore {
  blocks = $state<readonly ApiBlockSummary[]>([]);
  latestMetrics = $state<ApiMetrics | null>(null);
  status = $state<ApiStatus | null>(null);
  loadState = $state<ExplorerLoadState>("idle");
  error = $state<string | null>(null);

  private initialized = false;
  private blocksTimer: ReturnType<typeof setInterval> | null = null;
  private metricsTimer: ReturnType<typeof setInterval> | null = null;
  private statusTimer: ReturnType<typeof setInterval> | null = null;

  private readErrorMessage(caught: unknown): string {
    return caught instanceof Error ? caught.message : String(caught);
  }

  get blockHeight(): number {
    return this.latestMetrics?.height ?? this.blocks[0]?.height ?? 0;
  }

  get difficulty(): number {
    return this.latestMetrics?.difficulty ?? 0;
  }

  get circulatingSupply(): number {
    return this.latestMetrics?.circulatingSupply ?? 0;
  }

  get recentBlockCount(): number {
    return this.blocks.length;
  }

  get latestTransactionCount(): number {
    return this.latestMetrics?.txCount ?? 0;
  }

  get latestBlockSize(): number {
    return this.blocks[0]?.size ?? 0;
  }

  init(): void {
    if (this.initialized) {
      return;
    }

    this.initialized = true;
    this.loadState = "loading";
    this.error = null;

    void this.bootstrap();
  }

  markReady(): void {
    this.loadState = "ready";
  }

  setError(message: string): void {
    this.error = message;
    this.loadState = "error";
  }

  clearError(): void {
    this.error = null;
    this.loadState = "ready";
  }

  reset(): void {
    if (this.blocksTimer != null) {
      clearInterval(this.blocksTimer);
      this.blocksTimer = null;
    }

    if (this.metricsTimer != null) {
      clearInterval(this.metricsTimer);
      this.metricsTimer = null;
    }

    if (this.statusTimer != null) {
      clearInterval(this.statusTimer);
      this.statusTimer = null;
    }

    this.blocks = [];
    this.latestMetrics = null;
    this.status = null;
    this.loadState = "idle";
    this.error = null;
    this.initialized = false;
  }

  private async bootstrap(): Promise<void> {
    try {
      await Promise.all([this.refreshStatus(), this.refreshLatestMetrics(), this.refreshBlocks()]);
      this.markReady();
      this.startPolling();
    } catch (caught: unknown) {
      this.setError(this.readErrorMessage(caught));
    }
  }

  private startPolling(): void {
    this.statusTimer = setInterval((): void => {
      void this.poll(this.refreshStatus.bind(this));
    }, 15_000);

    this.metricsTimer = setInterval((): void => {
      void this.poll(this.refreshLatestMetrics.bind(this));
    }, 15_000);

    this.blocksTimer = setInterval((): void => {
      void this.poll(this.refreshBlocks.bind(this));
    }, 15_000);
  }

  private async poll(task: () => Promise<void>): Promise<void> {
    try {
      await task();
      if (this.loadState === "error") {
        this.clearError();
      }
    } catch (caught: unknown) {
      this.setError(this.readErrorMessage(caught));
    }
  }

  private async refreshLatestMetrics(): Promise<void> {
    this.latestMetrics = await getMetricsLatest();
  }

  private async refreshStatus(): Promise<void> {
    this.status = await getStatus();
  }

  private async refreshBlocks(): Promise<void> {
    const response = await getBlocks(12);
    this.blocks = response.value;
  }
}

export const explorer = new ExplorerStore();
