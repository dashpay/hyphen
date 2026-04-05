export interface ChartPoint {
  readonly timestamp: number;
  readonly value: number;
}

/** Converts history points into a smooth SVG path for sparkline charts. */
export function chartToSmoothPath(points: readonly ChartPoint[], width: number = 300, height: number = 65): string {
  const safeWidth = Number.isFinite(width) && width > 0 ? width : 300;
  const safeHeight = Number.isFinite(height) && height > 0 ? height : 65;
  const validPoints = points.filter((point): boolean => {
    return Number.isFinite(point.timestamp) && Number.isFinite(point.value);
  });
  const pointCount = validPoints.length;

  if (pointCount === 0) {
    return "M0," + String(safeHeight) + "L" + String(safeWidth) + "," + String(safeHeight);
  }

  if (pointCount === 1) {
    const midpoint = safeHeight / 2;
    return "M0," + String(midpoint) + "L" + String(safeWidth) + "," + String(midpoint);
  }

  const values = validPoints.map((point: ChartPoint): number => point.value);
  const minimum = Math.min(...values);
  const maximum = Math.max(...values);
  const range = maximum - minimum || 1;
  const step = safeWidth / Math.max(pointCount - 1, 1);

  const xs: number[] = Array.from({ length: pointCount }, (_, index: number): number => index * step);
  const ys: number[] = values.map((value: number): number => safeHeight - ((value - minimum) / range) * safeHeight);

  const at = (entries: readonly number[], index: number): number => entries[index] ?? 0;

  const deltas: number[] = [];
  const secants: number[] = [];

  for (let index = 0; index < pointCount - 1; index += 1) {
    const deltaX = at(xs, index + 1) - at(xs, index);
    deltas.push(deltaX);
    secants.push((at(ys, index + 1) - at(ys, index)) / deltaX);
  }

  const tangents: number[] = new Array<number>(pointCount).fill(0);
  tangents[0] = at(secants, 0);
  tangents[pointCount - 1] = at(secants, pointCount - 2);

  for (let index = 1; index < pointCount - 1; index += 1) {
    const previousSecant = at(secants, index - 1);
    const nextSecant = at(secants, index);
    tangents[index] = previousSecant * nextSecant <= 0 ? 0 : (previousSecant + nextSecant) / 2;
  }

  for (let index = 0; index < pointCount - 1; index += 1) {
    const secant = at(secants, index);

    if (Math.abs(secant) < 1e-12) {
      tangents[index] = 0;
      tangents[index + 1] = 0;
      continue;
    }

    const alpha = at(tangents, index) / secant;
    const beta = at(tangents, index + 1) / secant;
    const sum = alpha * alpha + beta * beta;

    if (sum > 9) {
      const tau = 3 / Math.sqrt(sum);
      tangents[index] = tau * alpha * secant;
      tangents[index + 1] = tau * beta * secant;
    }
  }

  const parts: string[] = ["M" + at(xs, 0).toFixed(1) + "," + at(ys, 0).toFixed(1)];

  for (let index = 0; index < pointCount - 1; index += 1) {
    const deltaX = at(deltas, index) / 3;
    const controlPointOneX = at(xs, index) + deltaX;
    const controlPointOneY = at(ys, index) + at(tangents, index) * deltaX;
    const controlPointTwoX = at(xs, index + 1) - deltaX;
    const controlPointTwoY = at(ys, index + 1) - at(tangents, index + 1) * deltaX;

    parts.push(
      "C" +
        controlPointOneX.toFixed(1) +
        "," +
        controlPointOneY.toFixed(1) +
        " " +
        controlPointTwoX.toFixed(1) +
        "," +
        controlPointTwoY.toFixed(1) +
        " " +
        at(xs, index + 1).toFixed(1) +
        "," +
        at(ys, index + 1).toFixed(1)
    );
  }

  return parts.join("");
}
