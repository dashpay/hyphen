export type HyGraphPresentationMode = "fullscreen" | "overlay";

export const HY_GRAPH_FULLSCREEN_MAX_WIDTH = 768;

/** Chooses the expanded chart presentation mode for the current viewport width. */
export function HyGraphGetPresentationMode(viewportWidth: number): HyGraphPresentationMode {
  return viewportWidth <= HY_GRAPH_FULLSCREEN_MAX_WIDTH ? "fullscreen" : "overlay";
}
