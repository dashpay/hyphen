<script lang="ts">
  /** Donut chart for share and composition breakdowns. */
  interface DonutSegment {
    readonly label: string;
    readonly value: number;
    readonly color?: string;
  }

  interface Props {
    readonly segments: readonly DonutSegment[];
    readonly size?: number;
    readonly strokeWidth?: number;
    readonly centerLabel?: string;
    readonly centerValue?: string;
    readonly centerSubvalue?: string;
    readonly gapDeg?: number;
    readonly hoveredIndex?: number | null;
    readonly onhover?: (index: number | null) => void;
    readonly fill?: boolean;
  }

  const {
    segments,
    size = 180,
    strokeWidth = 28,
    centerLabel,
    centerValue,
    centerSubvalue,
    gapDeg = 2,
    hoveredIndex = null,
    onhover,
    fill = false,
  }: Props = $props();

  const palette = ["#4C7EFF", "#60F6D2", "#FF6B8A", "#FFB74D", "#AB7AFF", "#4DD0E1", "#FF8A65", "#81C784"] as const;

  interface ArcDef {
    readonly path: string;
    readonly stroke: string;
    readonly isCircle: boolean;
    readonly segmentIndex: number;
  }

  function polarToCartesian(cx: number, cy: number, radius: number, angleDeg: number): { x: number; y: number } {
    const angle = ((angleDeg - 90) * Math.PI) / 180;
    return { x: cx + radius * Math.cos(angle), y: cy + radius * Math.sin(angle) };
  }

  function buildArc(cx: number, cy: number, radius: number, startDeg: number, endDeg: number): string {
    const start = polarToCartesian(cx, cy, radius, startDeg);
    const end = polarToCartesian(cx, cy, radius, endDeg);
    const largeArc = endDeg - startDeg > 180 ? 1 : 0;
    return `M ${String(start.x)} ${String(start.y)} A ${String(radius)} ${String(radius)} 0 ${String(largeArc)} 1 ${String(end.x)} ${String(end.y)}`;
  }

  const radius = $derived((size - strokeWidth) / 2);
  const center = $derived(size / 2);
  const renderableSegments = $derived(
    segments.flatMap((segment, index) => {
      if (segment.value <= 0) {
        return [];
      }

      return [{ index, segment }];
    })
  );
  const total = $derived(renderableSegments.reduce((sum, entry) => sum + entry.segment.value, 0));
  const availableDegrees = $derived.by((): number => {
    if (renderableSegments.length <= 1) {
      return 360;
    }

    return Math.max(0, 360 - gapDeg * renderableSegments.length);
  });

  const arcs = $derived.by((): readonly ArcDef[] => {
    if (total <= 0 || renderableSegments.length === 0) {
      return [];
    }

    if (renderableSegments.length === 1) {
      const [entry] = renderableSegments;
      return [
        {
          path: buildArc(center, center, radius, 0, 359.999),
          stroke: entry?.segment.color ?? palette[entry?.index ?? 0] ?? "#4C7EFF",
          isCircle: true,
          segmentIndex: entry?.index ?? 0,
        },
      ];
    }

    let cursor = 0;

    return renderableSegments.map(({ index, segment }) => {
      const degrees = (segment.value / total) * availableDegrees;
      const startDeg = cursor;
      const endDeg = cursor + degrees;
      cursor = endDeg + gapDeg;

      return {
        path: buildArc(center, center, radius, startDeg, endDeg),
        stroke: segment.color ?? palette[index % palette.length] ?? "#4C7EFF",
        isCircle: false,
        segmentIndex: index,
      };
    });
  });

  const displayLabel = $derived.by((): string | undefined => {
    if (hoveredIndex != null) {
      return segments[hoveredIndex]?.label ?? centerLabel;
    }

    return centerLabel;
  });

  const displayValue = $derived.by((): string | undefined => {
    if (hoveredIndex != null && total > 0) {
      const segment = segments[hoveredIndex];

      if (segment != null) {
        return ((segment.value / total) * 100).toFixed(1) + "%";
      }
    }

    return centerValue;
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class:donut-fill={fill}
  class="donut"
  onmouseleave={(): void => {
    onhover?.(null);
  }}
  style={fill ? "" : `height: ${String(size)}px; width: ${String(size)}px;`}
>
  <svg
    aria-hidden="true"
    class="chart"
    style={fill ? "aspect-ratio: 1;" : ""}
    viewBox={`0 0 ${String(size)} ${String(size)}`}
    width={fill ? "100%" : size}
    {...fill ? {} : { height: size }}
  >
    {#each arcs as arc (String(arc.segmentIndex))}
      {#if arc.isCircle}
        <circle
          class="arc"
          cx={center}
          cy={center}
          fill="none"
          onmouseenter={(): void => {
            onhover?.(arc.segmentIndex);
          }}
          r={radius}
          stroke={arc.stroke}
          stroke-width={strokeWidth}
          style={`opacity: ${hoveredIndex != null && hoveredIndex !== arc.segmentIndex ? "0.35" : "1"};`}
        />
      {:else}
        <path
          class="arc"
          d={arc.path}
          fill="none"
          onmouseenter={(): void => {
            onhover?.(arc.segmentIndex);
          }}
          stroke={arc.stroke}
          stroke-linecap="round"
          stroke-width={strokeWidth}
          style={`opacity: ${hoveredIndex != null && hoveredIndex !== arc.segmentIndex ? "0.35" : "1"};`}
        />
      {/if}
    {/each}
  </svg>
  {#if displayLabel !== undefined || displayValue !== undefined || centerSubvalue !== undefined}
    <div class="center-copy">
      {#if displayLabel !== undefined}
        <span class="center-label">{displayLabel}</span>
      {/if}
      {#if displayValue !== undefined}
        <span class="center-value">{displayValue}</span>
      {/if}
      {#if centerSubvalue !== undefined}
        <span class="center-subvalue">{centerSubvalue}</span>
      {/if}
    </div>
  {/if}
</div>
<span class="visually-hidden">
  {#each segments as segment, index (segment.label + ":" + String(index))}
    {segment.label}: {segment.value}.
  {/each}
</span>

<style>
  .donut {
    align-items: center;
    display: inline-flex;
    flex-shrink: 0;
    justify-content: center;
    position: relative;
  }

  .donut-fill {
    aspect-ratio: 1;
    height: auto;
    width: 100%;
  }

  .chart {
    display: block;
    height: auto;
    width: 100%;
  }

  .arc {
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-default);
    vector-effect: non-scaling-stroke;
  }

  .center-copy {
    align-items: center;
    display: flex;
    flex-direction: column;
    gap: 2px;
    inset: 0;
    justify-content: center;
    pointer-events: none;
    position: absolute;
  }

  .center-label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .center-value {
    color: var(--color-fg);
    font-family: var(--font-mono);
    font-size: var(--text-md);
    font-weight: 800;
  }

  .center-subvalue {
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .visually-hidden {
    border: 0;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    height: 1px;
    margin: -1px;
    overflow: hidden;
    padding: 0;
    position: absolute;
    white-space: nowrap;
    width: 1px;
  }
</style>
