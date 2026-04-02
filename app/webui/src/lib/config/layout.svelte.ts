import { HyUtilGetBreakpointWidth } from "$lib/HyUtil";

/** Observes whether the shell is in mobile dock mode. */
export function observeDockMode(syncValue: (nextValue: boolean) => void): () => void {
  const query = globalThis.matchMedia(`(max-width: ${String(HyUtilGetBreakpointWidth("tablet"))}px)`);
  const sync = (): void => {
    syncValue(query.matches);
  };

  sync();
  query.addEventListener("change", sync);

  return (): void => {
    query.removeEventListener("change", sync);
  };
}
