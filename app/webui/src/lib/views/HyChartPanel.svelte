<script lang="ts">
  /** Chart panel for expanded metric history with range controls. */
  import { VisArea, VisAxis, VisBrush, VisCrosshair, VisTooltip, VisXYContainer } from "@unovis/svelte";
  import { VwPanel, VwText } from "@repo/hyphen-views";

  import { TIME_RANGES, filterByRange, formatTimeRangeLabel, type TimeRange } from "$lib/data/hashrate";
  import type { ChartPoint } from "$lib/data/chart";
  import { HyUtilFormatNumber } from "$lib/HyUtil";

  interface Props {
    readonly label: string;
    readonly value: string;
    readonly data: readonly ChartPoint[];
    readonly blockTimeSeconds: number;
    readonly bucketBlocks?: number;
    readonly selectedRange?: TimeRange;
    readonly unit?: string;
    readonly valueFormatter?: ((value: number) => string) | undefined;
    readonly onrangechange?: ((range: TimeRange) => void) | undefined;
    readonly onclose: () => void;
  }

  const {
    label,
    value,
    data,
    blockTimeSeconds,
    bucketBlocks = 1,
    selectedRange,
    unit = "",
    valueFormatter,
    onrangechange,
    onclose,
  }: Props = $props();

  let internalRange = $state<TimeRange>("1h");
  let brushSelection = $state<[number, number] | null>(null);
  let hoveredPoint = $state<ChartPoint | null>(null);

  const activeRange = $derived(selectedRange ?? internalRange);
  const filteredData = $derived.by((): ChartPoint[] => {
    if (selectedRange != null) {
      return [...data];
    }

    return filterByRange(data, activeRange, blockTimeSeconds, bucketBlocks);
  });
  const chartKey = $derived.by((): string => {
    const firstTimestamp = filteredData[0]?.timestamp ?? 0;
    const lastTimestamp = filteredData[filteredData.length - 1]?.timestamp ?? 0;
    return [activeRange, filteredData.length, firstTimestamp, lastTimestamp].join(":");
  });
  const domainData = $derived.by((): readonly ChartPoint[] => {
    if (brushSelection == null) {
      return filteredData;
    }

    const [start, end] = brushSelection;
    const minTimestamp = Math.min(start, end);
    const maxTimestamp = Math.max(start, end);
    const visiblePoints = filteredData.filter((point): boolean => {
      return point.timestamp >= minTimestamp && point.timestamp <= maxTimestamp;
    });

    return visiblePoints.length > 0 ? visiblePoints : filteredData;
  });
  const yDomain = $derived.by((): [number, number] | undefined => {
    if (domainData.length === 0) {
      return undefined;
    }

    let minValue = Number.POSITIVE_INFINITY;
    let maxValue = Number.NEGATIVE_INFINITY;

    for (const point of domainData) {
      minValue = Math.min(minValue, point.value);
      maxValue = Math.max(maxValue, point.value);
    }

    if (!Number.isFinite(minValue) || !Number.isFinite(maxValue)) {
      return undefined;
    }

    if (minValue === maxValue) {
      const padding = Math.max(Math.abs(minValue) * 0.05, 1);
      return [minValue - padding, maxValue + padding];
    }

    const span = maxValue - minValue;
    const padding = span * 0.12;
    return [minValue - padding, maxValue + padding];
  });
  const x = (datum: ChartPoint): number => datum.timestamp;
  const y = (datum: ChartPoint): number => datum.value;

  function formatHoverValue(valueToFormat: number): string {
    if (valueFormatter != null) {
      return valueFormatter(valueToFormat);
    }

    return valueToFormat.toFixed(2) + (unit.length > 0 ? " " + unit : "");
  }

  function formatPointValue(valueToFormat: number): string {
    return formatHoverValue(valueToFormat);
  }

  const displayValue = $derived.by((): string => {
    const point = hoveredPoint;
    return point != null ? formatPointValue(point.value) : value;
  });

  const displayDate = $derived.by((): string => {
    const timestamp = hoveredPoint?.timestamp ?? data[data.length - 1]?.timestamp;

    if (timestamp == null) {
      return "";
    }

    return "Block #" + HyUtilFormatNumber(timestamp);
  });

  function selectRange(range: TimeRange): void {
    if (onrangechange != null) {
      onrangechange(range);
    } else {
      internalRange = range;
    }

    brushSelection = null;
  }

  function handleBrush(selection: [number, number] | undefined, _event: unknown, userDriven: boolean): void {
    if (!userDriven) {
      return;
    }

    brushSelection = selection ?? null;
  }

  function handleCrosshairMove(_xValue: number | Date | undefined, datum: ChartPoint | undefined): void {
    hoveredPoint = datum ?? null;
  }

  function crosshairTemplate(datum: ChartPoint): string {
    return `<div class="chart-tooltip">
      <span class="tooltip-value">${formatPointValue(datum.value)}</span>
      <span class="tooltip-date">Block #${HyUtilFormatNumber(datum.timestamp)}</span>
    </div>`;
  }

  function formatDateTick(tick: number | Date): string {
    return "#" + HyUtilFormatNumber(Number(tick));
  }

  function formatValueTick(tick: number | Date): string {
    const valueToFormat = Number(tick);

    if (valueFormatter != null) {
      return valueFormatter(valueToFormat);
    }

    return valueToFormat.toFixed(1) + (unit.length > 0 ? " " + unit : "");
  }
</script>

<div class="chart-wrap">
  <VwPanel border={true} padding={0} radius="sm" variant="glass">
    <div class="chart-header">
      <div class="chart-header-content">
        <div class="chart-copy">
          <VwText color="muted" size="base" weight="medium">{label}</VwText>
          <VwText size="2xl" weight="extrabold">{displayValue}</VwText>
          <VwText color="muted" size="sm" weight="medium">{displayDate}</VwText>
        </div>

        <div class="chart-controls">
          <div class="chart-pill-row">
            {#each TIME_RANGES as range (range)}
              <button
                class="pill"
                class:active={activeRange === range}
                onclick={(): void => {
                  selectRange(range);
                }}
                aria-label={range === "All" ? "Show all history" : "Show last " + range}
                type="button"
              >
                {formatTimeRangeLabel(range)}
              </button>
            {/each}
          </div>
          <button aria-label="Close chart" class="close-btn" onclick={onclose} type="button">&times;</button>
        </div>
      </div>
    </div>

    {#key chartKey}
      <div class="chart-body">
        <div class="main-chart">
          <VisXYContainer
            data={filteredData}
            {...brushSelection != null ? { xDomain: brushSelection } : {}}
            {...yDomain != null ? { yDomain } : {}}
            padding={{ top: 10, bottom: 0, left: 0, right: 0 }}
            scaleByDomain={true}
          >
            <VisArea {x} {y} color="var(--accent-blue)" curveType="monotoneX" opacity={0.15} />
            <VisCrosshair
              {x}
              {y}
              onCrosshairMove={handleCrosshairMove}
              snapToData={true}
              template={crosshairTemplate}
            />
            <VisTooltip />
            <VisAxis
              domainLine={false}
              gridLine={false}
              numTicks={6}
              tickFormat={formatDateTick}
              tickLine={false}
              type="x"
            />
            <VisAxis
              domainLine={false}
              gridLine={true}
              numTicks={5}
              tickFormat={formatValueTick}
              tickLine={false}
              type="y"
            />
          </VisXYContainer>
        </div>

        <div class="nav-chart">
          <VisXYContainer data={filteredData} padding={{ top: 5, bottom: 0, left: 0, right: 0 }}>
            <VisArea {x} {y} color="var(--accent-blue)" curveType="monotoneX" opacity={0.1} />
            <VisBrush draggable={true} handleWidth={8} onBrush={handleBrush} />
            <VisAxis
              domainLine={false}
              gridLine={false}
              numTicks={4}
              tickFormat={formatDateTick}
              tickLine={false}
              type="x"
            />
          </VisXYContainer>
        </div>
      </div>
    {/key}
  </VwPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .chart-wrap {
    height: 100%;
    width: 100%;

    --vis-axis-grid-line-stroke-color: var(--hy-chart-grid);
    --vis-axis-tick-label-color: var(--hy-chart-ticks);
    --vis-area-fill-color: var(--accent-blue);
    --vis-area-stroke-color: var(--accent-blue);
    --vis-area-stroke-width: 1.5;
    --vis-brush-handle-fill-color: var(--accent-blue);
    --vis-brush-selection-fill-color: var(--hy-chart-brush-fill);
    --vis-brush-selection-stroke-color: var(--hy-chart-brush-stroke);
    --vis-crosshair-circle-fill-color: var(--accent-blue);
    --vis-crosshair-circle-stroke-color: var(--color-bg);
    --vis-crosshair-circle-stroke-width: 2;
    --vis-crosshair-line-stroke-color: var(--accent-blue);
    --vis-crosshair-line-stroke-opacity: 0.6;
    --vis-crosshair-line-stroke-width: 1;
    --vis-tooltip-background-color: var(--hy-chart-tooltip-bg);
    --vis-tooltip-border-color: var(--glass-border);
    --vis-tooltip-border-radius: var(--radius-input);
    --vis-tooltip-padding: var(--hy-chart-tooltip-padding);
    --vis-tooltip-shadow-color: var(--hy-chart-tooltip-shadow);
    --vis-tooltip-text-color: var(--color-fg);
  }

  .chart-wrap :global(.panel) {
    display: grid;
    height: 100%;
    grid-template-rows: auto minmax(0, 1fr);
    border-radius: var(--hy-chart-overlay-radius);
  }

  .chart-header {
    border-bottom: 1px solid var(--glass-border);
    flex-shrink: 0;
    position: relative;
    padding: var(--hy-chart-panel-header-padding);
  }

  .chart-header-content {
    align-items: start;
    display: flex;
    gap: 1rem;
    justify-content: space-between;
    min-width: 0;
  }

  .chart-copy {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: 0.25rem;
    min-width: 0;
  }

  .chart-controls {
    align-items: flex-start;
    display: flex;
    flex: 0 1 auto;
    gap: 0.75rem;
    justify-content: flex-end;
    min-width: 0;
  }

  .chart-pill-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    justify-content: flex-end;
    min-width: 0;
  }

  .pill {
    background: var(--hy-chart-pill-bg);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-pill);
    color: var(--text-muted);
    cursor: pointer;
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    padding: var(--hy-chart-pill-padding);
    transition:
      background var(--duration-fast) var(--ease-default),
      color var(--duration-fast) var(--ease-default);
  }

  .pill:hover {
    background: var(--hy-chart-pill-hover);
    color: var(--color-fg);
  }

  .pill.active {
    background: var(--accent-blue);
    border-color: var(--accent-blue);
    color: var(--color-on-accent);
  }

  .close-btn {
    align-items: center;
    background: var(--hy-chart-close-bg);
    border: 1px solid var(--glass-border);
    border-radius: 50%;
    color: var(--color-fg);
    cursor: pointer;
    display: flex;
    flex-shrink: 0;
    font-size: 1.25rem;
    height: var(--hy-chart-close-size);
    justify-content: center;
    transition: background var(--duration-fast) var(--ease-default);
    width: var(--hy-chart-close-size);
  }

  .close-btn:hover {
    background: var(--hy-chart-close-hover);
  }

  .chart-body {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--hy-detail-stat-gap);
    min-height: 0;
    padding: var(--hy-chart-panel-body-padding);
  }

  .main-chart {
    flex: 4;
    min-height: 0;
  }

  .nav-chart {
    flex: 1;
    min-height: 0;
  }

  .main-chart :global(.unovis-xy-container),
  .nav-chart :global(.unovis-xy-container) {
    height: 100%;
  }

  .chart-wrap :global(.chart-tooltip) {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chart-wrap :global(.tooltip-value) {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-base);
    font-weight: 700;
  }

  .chart-wrap :global(.tooltip-date) {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .chart-wrap {
      height: 100dvh;
    }

    .chart-wrap :global(.panel) {
      border-radius: 0;
    }

    .chart-header {
      padding-right: calc(var(--hy-chart-close-size) + 2rem);
    }

    .chart-controls,
    .chart-pill-row {
      align-items: flex-start;
      justify-content: flex-start;
    }

    .chart-header-content {
      display: block;
    }

    .chart-controls {
      display: block;
      margin-top: 0.75rem;
    }

    .close-btn {
      position: absolute;
      right: 1rem;
      top: 1rem;
    }

    .chart-body {
      gap: 0.75rem;
      padding: 0.875rem;
    }

    .main-chart {
      flex: 1 1 auto;
    }

    .nav-chart {
      flex: 0 0 96px;
    }
  }

  /* --hy-breakpoint-phone */
  @include view.media-down(phone) {
    .chart-header {
      padding: 1rem;
    }

    .chart-pill-row {
      gap: 0.375rem;
    }

    .pill {
      font-size: var(--text-xs);
      padding: 0.45rem 0.65rem;
    }

    .chart-body {
      gap: 0.625rem;
      padding: 0.75rem;
    }

    .nav-chart {
      flex-basis: 88px;
    }
  }
</style>
