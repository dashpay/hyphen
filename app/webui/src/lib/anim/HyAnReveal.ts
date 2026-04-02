import { cubicOut } from "svelte/easing";
import { fly } from "svelte/transition";

import type { TransitionConfig } from "svelte/transition";

interface RevealParams {
  readonly delay?: number;
  readonly duration?: number;
  readonly y?: number;
}

/** Reveals stacked content with a short vertical slide and fade. */
export function HyAnReveal(node: Element, params: RevealParams = {}): TransitionConfig {
  return fly(node, {
    delay: params.delay ?? 0,
    duration: params.duration ?? 180,
    easing: cubicOut,
    opacity: 0,
    y: params.y ?? 8,
  });
}
