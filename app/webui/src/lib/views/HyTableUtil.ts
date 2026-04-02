import type { HyUtilBreakpointName } from "$lib/HyUtil";
import { HyUtilGetBreakpointWidth } from "$lib/HyUtil";

export type HyTableAlign = "start" | "end";

export interface HyTableColumnSpec<HyTableColumnId extends string = string> {
  readonly align?: HyTableAlign;
  readonly compactBelow?: HyUtilBreakpointName;
  readonly dropBelow?: HyUtilBreakpointName;
  readonly elastic?: boolean;
  readonly flexible?: boolean;
  readonly headerWidth?: string;
  readonly id: HyTableColumnId;
  readonly label: string;
  readonly minWidth?: string;
  readonly width?: string;
}

/** Filters one column spec list for the current measured container width. */
export function HyTableGetVisibleColumns<HyTableColumnId extends string>(
  columns: readonly HyTableColumnSpec<HyTableColumnId>[],
  containerWidth: number
): readonly HyTableColumnSpec<HyTableColumnId>[] {
  const safeWidth = Number.isFinite(containerWidth) && containerWidth > 0 ? containerWidth : Number.POSITIVE_INFINITY;

  return columns.filter((column: HyTableColumnSpec<HyTableColumnId>): boolean => {
    if (column.dropBelow == null) {
      return true;
    }

    return safeWidth >= HyUtilGetBreakpointWidth(column.dropBelow);
  });
}

/** Builds one shared grid template string from the active column list. */
export function HyTableGetTemplate<HyTableColumnId extends string>(
  columns: readonly HyTableColumnSpec<HyTableColumnId>[],
  _unusedContainerWidth?: number
): string {
  void _unusedContainerWidth;

  return columns
    .map((column: HyTableColumnSpec<HyTableColumnId>): string => {
      if (column.flexible) {
        return `minmax(${column.minWidth ?? "0px"}, 1fr)`;
      }

      const width = column.width ?? "max-content";
      return column.elastic ? `minmax(${width}, 1fr)` : width;
    })
    .join(" ");
}

/** Builds one shared header grid template string from the active column list. */
export function HyTableGetHeaderTemplate<HyTableColumnId extends string>(
  columns: readonly HyTableColumnSpec<HyTableColumnId>[],
  _unusedContainerWidth?: number
): string {
  void _unusedContainerWidth;

  return columns
    .map((column: HyTableColumnSpec<HyTableColumnId>): string => {
      if (column.flexible) {
        return `minmax(${column.minWidth ?? "0px"}, 1fr)`;
      }

      const headerWidth = column.headerWidth ?? column.width ?? "max-content";
      return column.elastic ? `minmax(${headerWidth}, 1fr)` : headerWidth;
    })
    .join(" ");
}

/** Returns whether the active column list contains one column id. */
export function HyTableHasColumn<HyTableColumnId extends string>(
  columns: readonly HyTableColumnSpec<HyTableColumnId>[],
  columnId: HyTableColumnId
): boolean {
  return columns.some((column: HyTableColumnSpec<HyTableColumnId>): boolean => column.id === columnId);
}

/** Observes one table container and reports its inline size. */
export function HyTableObserveWidth(node: Element, onWidthChange: (width: number) => void): () => void {
  const observer = new ResizeObserver((entries: readonly ResizeObserverEntry[]): void => {
    const entry = entries[0];
    onWidthChange(entry?.contentRect.width ?? node.clientWidth);
  });

  observer.observe(node);
  onWidthChange(node.clientWidth);

  return (): void => {
    observer.disconnect();
  };
}

/** Observes one table subtree for resize and text-content changes. */
export function HyTableObserveLayout(node: Element, onLayoutChange: () => void): () => void {
  const resizeObserver = new ResizeObserver((): void => {
    onLayoutChange();
  });
  const mutationObserver = new MutationObserver((): void => {
    onLayoutChange();
  });

  resizeObserver.observe(node);
  mutationObserver.observe(node, {
    characterData: true,
    childList: true,
    subtree: true,
  });
  onLayoutChange();

  return (): void => {
    resizeObserver.disconnect();
    mutationObserver.disconnect();
  };
}

/** Reads the computed font shorthand for one table cell selector. */
export function HyTableReadFont(node: Element, selector: string): string | null {
  const sample = node.querySelector<HTMLElement>(selector);

  if (sample == null) {
    return null;
  }

  return getComputedStyle(sample).font;
}

function resolveWidthValue(width: number | string): number {
  if (typeof width === "number") {
    return width;
  }

  const variableMatch = width.match(/^var\((--[^)]+)\)$/);

  if (variableMatch?.[1] != null) {
    const resolvedWidth = Number.parseFloat(
      getComputedStyle(document.documentElement).getPropertyValue(variableMatch[1])
    );
    return Number.isFinite(resolvedWidth) ? resolvedWidth : 0;
  }

  const parsedWidth = Number.parseFloat(width);
  return Number.isFinite(parsedWidth) ? parsedWidth : 0;
}

/** Measures intrinsic text width and applies one shared lane floor plus buffer. */
export function HyTableMeasureTextWidth(
  texts: readonly string[],
  font: string,
  minWidth: number | string,
  buffer: number = 20
): number | null {
  if (typeof document === "undefined") {
    return null;
  }

  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");

  if (context == null) {
    return null;
  }

  context.font = font;

  const widths = texts
    .map((text: string): number => Math.ceil(context.measureText(text).width + buffer))
    .filter((width: number): boolean => width > 0);

  if (widths.length === 0) {
    return null;
  }

  const measuredWidth = Math.max(resolveWidthValue(minWidth), Math.max(...widths));
  return measuredWidth;
}

/** Returns whether one column is in its compact-width state. */
export function HyTableIsCompactColumn<HyTableColumnId extends string>(
  columns: readonly HyTableColumnSpec<HyTableColumnId>[],
  columnId: HyTableColumnId,
  containerWidth: number
): boolean {
  const column = columns.find((candidate: HyTableColumnSpec<HyTableColumnId>): boolean => candidate.id === columnId);

  if (column?.compactBelow == null) {
    return false;
  }

  return containerWidth < HyUtilGetBreakpointWidth(column.compactBelow);
}
