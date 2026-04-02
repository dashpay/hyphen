<script lang="ts">
  /** Sparkline for compact inline history previews. */
  interface Props {
    readonly points: readonly number[];
    readonly stroke?: string;
    readonly strokeWidth?: number;
    readonly fill?: boolean;
    readonly fillOpacity?: number;
    readonly smooth?: boolean;
  }

  const {
    points,
    stroke = "var(--accent-blue)",
    strokeWidth = 2,
    fill = false,
    fillOpacity = 0.12,
    smooth = true,
  }: Props = $props();

  const width = 100;
  const height = 32;

  interface Point {
    readonly x: number;
    readonly y: number;
  }

  function buildCoordinates(values: readonly number[]): Point[] {
    if (values.length === 0) {
      return [];
    }

    const minimum = Math.min(...values);
    const maximum = Math.max(...values);
    const span = maximum - minimum || 1;

    return values.map((value, index) => {
      const x = values.length === 1 ? width / 2 : (index / (values.length - 1)) * width;
      const y = height - ((value - minimum) / span) * height;
      return { x, y };
    });
  }

  function buildLine(values: readonly Point[]): string {
    if (values.length === 0) {
      return "";
    }

    if (!smooth || values.length < 3) {
      return values.map((point, index) => `${index === 0 ? "M" : "L"} ${String(point.x)} ${String(point.y)}`).join(" ");
    }

    let path = `M ${String(values[0]?.x ?? 0)} ${String(values[0]?.y ?? 0)}`;

    for (let index = 0; index < values.length - 1; index += 1) {
      const current = values[index];
      const next = values[index + 1];
      if (current === undefined || next === undefined) {
        continue;
      }

      const controlX = (current.x + next.x) / 2;
      path += ` C ${String(controlX)} ${String(current.y)}, ${String(controlX)} ${String(next.y)}, ${String(next.x)} ${String(next.y)}`;
    }

    return path;
  }

  function buildArea(values: readonly Point[]): string {
    if (values.length === 0) {
      return "";
    }

    const line = buildLine(values);
    const first = values[0];
    const last = values[values.length - 1];
    if (first === undefined || last === undefined) {
      return "";
    }

    return `${line} L ${String(last.x)} ${String(height)} L ${String(first.x)} ${String(height)} Z`;
  }

  const coordinates = $derived(buildCoordinates(points));
  const linePath = $derived(buildLine(coordinates));
  const areaPath = $derived(buildArea(coordinates));
</script>

<svg aria-hidden="true" class="sparkline" viewBox={`0 0 ${String(width)} ${String(height)}`}>
  {#if fill && areaPath.length > 0}
    <path d={areaPath} fill={stroke} fill-opacity={fillOpacity}></path>
  {/if}
  {#if linePath.length > 0}
    <path class="line" d={linePath} fill="none" stroke-linecap="round" stroke-width={strokeWidth} {stroke}></path>
  {/if}
</svg>

<style>
  .sparkline {
    display: block;
    height: auto;
    width: 100%;
  }

  .line {
    vector-effect: non-scaling-stroke;
  }
</style>
