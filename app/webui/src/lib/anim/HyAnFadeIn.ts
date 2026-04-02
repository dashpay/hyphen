import { cubicOut } from "svelte/easing";
import { fade } from "svelte/transition";

import type { TransitionConfig } from "svelte/transition";

interface FadeInParams {
  readonly delay?: number;
  readonly duration?: number;
}

/** Fades content in with Hyphen's default motion timing. */
export function HyAnFadeIn(node: Element, params: FadeInParams = {}): TransitionConfig {
  return fade(node, {
    delay: params.delay ?? 0,
    duration: params.duration ?? 160,
    easing: cubicOut,
  });
}
