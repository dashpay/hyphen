/** Shared layout checkpoints for reference-fidelity surfaces. */
export interface HyDonutPreset {
  readonly diameter: number;
  readonly strokeWidth: number;
}

/**
 * Returns stable donut checkpoints so the dashboard keeps the reference ring
 * proportions instead of scaling continuously with container width.
 */
export function HyGetDonutPreset(width: number): HyDonutPreset {
  if (width <= 480) {
    return { diameter: 160, strokeWidth: 22 };
  }

  if (width <= 768) {
    return { diameter: 172, strokeWidth: 23 };
  }

  return { diameter: 180, strokeWidth: 24 };
}

/**
 * Returns a donut checkpoint constrained by the actual panel body dimensions and
 * the space needed for the legend. This keeps dashboard donuts inside their
 * panels instead of sizing them only from the outer dashboard width.
 */
export function HyGetBoundedDonutPreset(
  width: number,
  height: number,
  legendItems: number,
  singleColumnLegend: boolean
): HyDonutPreset {
  const legendColumns = singleColumnLegend ? 1 : 2;
  const legendRows = Math.max(1, Math.ceil(legendItems / legendColumns));
  const legendHeight = legendRows * 30 + Math.max(0, legendRows - 1) * 4;
  const availableWidth = Math.max(0, width - 32);
  const availableHeight = Math.max(0, height - legendHeight - 24);
  const available = Math.min(availableWidth, availableHeight);

  if (available <= 140) {
    return { diameter: 120, strokeWidth: 16 };
  }

  if (available <= 180) {
    return { diameter: 144, strokeWidth: 18 };
  }

  if (available <= 220) {
    return { diameter: 160, strokeWidth: 20 };
  }

  if (available <= 260) {
    return { diameter: 172, strokeWidth: 22 };
  }

  return { diameter: 180, strokeWidth: 24 };
}
