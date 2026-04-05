<script lang="ts">
  import { goto } from "$app/navigation";
  import { localeStore, tr } from "@repo/hyphen-i18n";
  import { onDestroy, onMount } from "svelte";
  import { VwSearchInput, VwText } from "@repo/hyphen-views";
  import {
    getBlocks,
    getProposals,
    getRichList,
    getTransactions,
    getValidators,
    searchExplorer,
    type ApiSearchResponse,
  } from "$lib";
  import HyTag from "./HyTag.svelte";

  type SearchSuggestionKind = "address" | "block" | "proposal" | "transaction" | "validator";

  interface SearchSuggestion {
    readonly href: string;
    readonly id: string;
    readonly kind: SearchSuggestionKind;
    readonly primary: string;
    readonly searchValues: readonly string[];
    readonly secondary?: string;
  }

  interface Props {
    readonly ariaLabel: string;
    readonly disabled?: boolean;
    readonly governanceEnabled: boolean;
    readonly hasAddressIndex: boolean;
    readonly hasValidatorIndex: boolean;
    readonly onfocuschange?: (focused: boolean) => void;
    readonly placeholder: string;
    readonly validatorsEnabled: boolean;
  }

  let {
    ariaLabel,
    disabled = false,
    governanceEnabled,
    hasAddressIndex,
    hasValidatorIndex,
    onfocuschange,
    placeholder,
    validatorsEnabled,
  }: Props = $props();

  const activeLocale = $derived($localeStore);

  let focused = $state(false);
  let highlightedIndex = $state(-1);
  let loadingExact = $state(false);
  let loadingPool = $state(false);
  let query = $state("");
  let exactSuggestions = $state<readonly SearchSuggestion[]>([]);
  let poolSuggestions = $state<readonly SearchSuggestion[]>([]);
  let latestLookupId = 0;
  let lookupTimer: ReturnType<typeof globalThis.setTimeout> | null = null;
  let poolPromise: Promise<void> | null = null;

  const kindLabels = $derived.by((): Readonly<Record<SearchSuggestionKind, string>> => {
    void activeLocale;

    return {
      address: tr("Address"),
      block: tr("Block"),
      proposal: tr("Proposal"),
      transaction: tr("Transaction"),
      validator: tr("Validator"),
    };
  });

  const dropdownSuggestions = $derived.by((): readonly SearchSuggestion[] => {
    void activeLocale;

    const normalizedQuery = normalizeQuery(query);
    if (normalizedQuery.length === 0) {
      return [];
    }

    const scoredPool = poolSuggestions
      .map((suggestion) => ({
        score: scoreSuggestion(suggestion, normalizedQuery),
        suggestion,
      }))
      .filter(
        (entry): entry is { readonly score: number; readonly suggestion: SearchSuggestion } => entry.score != null
      )
      .sort((left, right): number => left.score - right.score)
      .map((entry) => entry.suggestion);

    return dedupeSuggestions([...exactSuggestions, ...scoredPool]).slice(0, 10);
  });

  const dropdownOpen = $derived.by(
    (): boolean => focused && query.trim().length > 0 && (loadingExact || loadingPool || dropdownSuggestions.length > 0)
  );

  $effect((): void => {
    const suggestions = dropdownSuggestions;
    if (!dropdownOpen || suggestions.length === 0) {
      highlightedIndex = -1;
      return;
    }

    if (highlightedIndex < 0 || highlightedIndex >= suggestions.length) {
      highlightedIndex = 0;
    }
  });

  onDestroy((): void => {
    clearLookupTimer();
  });

  onMount((): void => {
    if (governanceEnabled) {
      void ensureSuggestionPool();
    }
  });

  function clearLookupTimer(): void {
    if (lookupTimer != null) {
      globalThis.clearTimeout(lookupTimer);
      lookupTimer = null;
    }
  }

  function normalizeQuery(value: string): string {
    return value.trim().toLowerCase();
  }

  function isHexQuery(value: string): boolean {
    return /^[0-9a-f]+$/i.test(value);
  }

  function isFullHash(value: string): boolean {
    return /^[0-9a-f]{64}$/i.test(value);
  }

  function shouldUseExactLookup(value: string): boolean {
    const trimmedValue = value.trim();
    if (trimmedValue.length === 0) {
      return false;
    }

    if (isHexQuery(trimmedValue)) {
      return isFullHash(trimmedValue);
    }

    return trimmedValue.length >= 3;
  }

  function buildHref(kind: SearchSuggestionKind, id: string): string {
    switch (kind) {
      case "address":
        return "/address?id=" + encodeURIComponent(id);
      case "block":
        return "/block?hash=" + encodeURIComponent(id);
      case "proposal":
        return "/proposals?hash=" + encodeURIComponent(id);
      case "transaction":
        return "/transaction?hash=" + encodeURIComponent(id);
      case "validator":
        return "/validator?hash=" + encodeURIComponent(id);
    }
  }

  function scoreSuggestion(suggestion: SearchSuggestion, normalizedQuery: string): number | null {
    let bestScore: number | null = null;

    for (const [index, value] of suggestion.searchValues.entries()) {
      const normalizedValue = value.toLowerCase();
      let score: number | null = null;

      if (normalizedValue === normalizedQuery) {
        score = index;
      } else if (normalizedValue.startsWith(normalizedQuery)) {
        score = 10 + index;
      } else if (normalizedValue.includes(normalizedQuery)) {
        score = 30 + index;
      } else if (normalizedQuery.startsWith(normalizedValue)) {
        score = 50 + index;
      }

      if (score != null && (bestScore == null || score < bestScore)) {
        bestScore = score;
      }
    }

    return bestScore;
  }

  function dedupeSuggestions(suggestions: readonly SearchSuggestion[]): readonly SearchSuggestion[] {
    const seen = new Set<string>();

    return suggestions.filter((suggestion): boolean => {
      const key = `${suggestion.kind}:${suggestion.id}`;
      if (seen.has(key)) {
        return false;
      }
      seen.add(key);
      return true;
    });
  }

  function mapExactResult(result: ApiSearchResponse): SearchSuggestion | null {
    switch (result.type) {
      case "address":
        return {
          href: buildHref("address", result.id),
          id: result.id,
          kind: "address",
          primary: result.id,
          searchValues: [result.id],
        };
      case "block":
      case "height":
        return {
          href: buildHref("block", result.id),
          id: result.id,
          kind: "block",
          primary: result.id,
          searchValues: [result.id],
        };
      case "proposal":
        return {
          href: buildHref("proposal", result.id),
          id: result.id,
          kind: "proposal",
          primary: result.id,
          searchValues: [result.id],
        };
      case "tx":
        return {
          href: buildHref("transaction", result.id),
          id: result.id,
          kind: "transaction",
          primary: result.id,
          searchValues: [result.id],
        };
      case "validator":
        return {
          href: buildHref("validator", result.id),
          id: result.id,
          kind: "validator",
          primary: result.id,
          searchValues: [result.id],
          ...(result.status == null ? {} : { secondary: result.status }),
        };
      default:
        return null;
    }
  }

  async function ensureSuggestionPool(): Promise<void> {
    if (poolSuggestions.length > 0 || poolPromise != null) {
      await poolPromise;
      return;
    }

    loadingPool = true;

    poolPromise = (async (): Promise<void> => {
      const nextSuggestions: SearchSuggestion[] = [];
      const tasks: Promise<void>[] = [];

      tasks.push(
        getBlocks(100)
          .then((response): void => {
            nextSuggestions.push(
              ...response.value.map(
                (block): SearchSuggestion => ({
                  href: buildHref("block", block.blockHash),
                  id: block.blockHash,
                  kind: "block",
                  primary: block.blockHash,
                  searchValues: [block.blockHash, String(block.blockHeight)],
                  secondary: `#${String(block.blockHeight)}`,
                })
              )
            );
          })
          .catch((): void => {})
      );

      tasks.push(
        getTransactions(50)
          .then((response): void => {
            nextSuggestions.push(
              ...response.value.map(
                (transaction): SearchSuggestion => ({
                  href: buildHref("transaction", transaction.txHash),
                  id: transaction.txHash,
                  kind: "transaction",
                  primary: transaction.txHash,
                  searchValues: [transaction.txHash, String(transaction.blockHeight)],
                  secondary: `#${String(transaction.blockHeight)}`,
                })
              )
            );
          })
          .catch((): void => {})
      );

      if (hasAddressIndex) {
        tasks.push(
          getRichList(50)
            .then((response): void => {
              nextSuggestions.push(
                ...response.value.map(
                  (entry): SearchSuggestion => ({
                    href: buildHref("address", entry.address),
                    id: entry.address,
                    kind: "address",
                    primary: entry.address,
                    searchValues: [entry.address],
                    secondary: `#${String(entry.rank)}`,
                  })
                )
              );
            })
            .catch((): void => {})
        );
      }

      if (validatorsEnabled && hasValidatorIndex) {
        tasks.push(
          getValidators(50)
            .then((response): void => {
              nextSuggestions.push(
                ...response.value.map(
                  (validator): SearchSuggestion => ({
                    href: buildHref("validator", validator.proTxHash),
                    id: validator.proTxHash,
                    kind: "validator",
                    primary: validator.proTxHash,
                    searchValues: [
                      validator.proTxHash,
                      ...(validator.services.coreP2p.length > 0 ? validator.services.coreP2p : []),
                    ],
                    secondary: validator.services.coreP2p[0] ?? validator.status,
                  })
                )
              );
            })
            .catch((): void => {})
        );
      }

      if (governanceEnabled) {
        tasks.push(
          getProposals(100)
            .then((response): void => {
              nextSuggestions.push(
                ...response.value.map(
                  (proposal): SearchSuggestion => ({
                    href: buildHref("proposal", proposal.hash),
                    id: proposal.hash,
                    kind: "proposal",
                    primary: proposal.name,
                    searchValues: [
                      proposal.name,
                      proposal.hash,
                      ...(proposal.paymentAddress == null ? [] : [proposal.paymentAddress]),
                    ],
                    secondary: proposal.hash,
                  })
                )
              );
            })
            .catch((): void => {})
        );
      }

      await Promise.all(tasks);
      poolSuggestions = nextSuggestions;
    })().finally((): void => {
      loadingPool = false;
      poolPromise = null;
    });

    await poolPromise;
  }

  function queueExactLookup(nextQuery: string): void {
    clearLookupTimer();

    const trimmedQuery = nextQuery.trim();
    if (trimmedQuery.length === 0) {
      exactSuggestions = [];
      loadingExact = false;
      return;
    }

    if (!shouldUseExactLookup(trimmedQuery)) {
      exactSuggestions = [];
      loadingExact = false;
      return;
    }

    const requestId = ++latestLookupId;
    loadingExact = true;

    lookupTimer = globalThis.setTimeout((): void => {
      void searchExplorer(trimmedQuery)
        .then((results): void => {
          if (requestId !== latestLookupId) {
            return;
          }

          exactSuggestions = results.map(mapExactResult).filter((result): result is SearchSuggestion => result != null);
        })
        .catch((): void => {
          if (requestId !== latestLookupId) {
            return;
          }

          exactSuggestions = [];
        })
        .finally((): void => {
          if (requestId === latestLookupId) {
            loadingExact = false;
          }
        });
    }, 150);
  }

  function handleInput(nextQuery: string): void {
    query = nextQuery;

    if (nextQuery.trim().length === 0) {
      exactSuggestions = [];
      highlightedIndex = -1;
      return;
    }

    highlightedIndex = 0;
    void ensureSuggestionPool();
    queueExactLookup(nextQuery);
  }

  function handleBlur(): void {
    focused = false;
    onfocuschange?.(false);
  }

  function handleFocus(): void {
    focused = true;
    onfocuschange?.(true);
    if (query.trim().length > 0) {
      void ensureSuggestionPool();
      queueExactLookup(query);
    }
  }

  async function activateSuggestion(suggestion: SearchSuggestion): Promise<void> {
    query = "";
    exactSuggestions = [];
    highlightedIndex = -1;
    focused = false;
    onfocuschange?.(false);
    await goto(suggestion.href);
  }

  async function handleSubmit(currentQuery: string): Promise<void> {
    const suggestions = dropdownSuggestions;
    if (suggestions.length > 0 && highlightedIndex >= 0) {
      const selectedSuggestion = suggestions[Math.min(highlightedIndex, suggestions.length - 1)];
      if (selectedSuggestion != null) {
        await activateSuggestion(selectedSuggestion);
      }
      return;
    }

    if (!shouldUseExactLookup(currentQuery)) {
      return;
    }

    const [result] = await searchExplorer(currentQuery.trim());
    if (result == null) {
      return;
    }

    const suggestion = mapExactResult(result);
    if (suggestion == null) {
      return;
    }

    await activateSuggestion(suggestion);
  }

  function handleKeydown(event: KeyboardEvent): void {
    const suggestions = dropdownSuggestions;

    if (event.key === "ArrowDown" && suggestions.length > 0) {
      event.preventDefault();
      highlightedIndex = highlightedIndex < suggestions.length - 1 ? highlightedIndex + 1 : 0;
      return;
    }

    if (event.key === "ArrowUp" && suggestions.length > 0) {
      event.preventDefault();
      highlightedIndex = highlightedIndex > 0 ? highlightedIndex - 1 : suggestions.length - 1;
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      focused = false;
      highlightedIndex = -1;
      onfocuschange?.(false);
    }
  }
</script>

<div class="hy-search-box">
  <VwSearchInput
    {ariaLabel}
    {disabled}
    onblur={handleBlur}
    onfocus={handleFocus}
    oninput={handleInput}
    onkeydown={handleKeydown}
    onsearch={(currentQuery: string): void => {
      void handleSubmit(currentQuery);
    }}
    {placeholder}
    value={query}
  />

  {#if dropdownOpen}
    <div aria-label={tr("Search suggestions")} class="hy-search-results" role="listbox">
      {#if dropdownSuggestions.length === 0 && (loadingExact || loadingPool)}
        <div class="hy-search-row hy-search-row-loading">
          <VwText color="muted" size="sm">{tr("Searching...")}</VwText>
        </div>
      {:else}
        {#each dropdownSuggestions as suggestion, index (`${suggestion.kind}:${suggestion.id}`)}
          <button
            aria-selected={index === highlightedIndex}
            class:selected={index === highlightedIndex}
            class="hy-search-row"
            onmousedown={(event: MouseEvent): void => {
              event.preventDefault();
            }}
            onmouseenter={(): void => {
              highlightedIndex = index;
            }}
            onclick={(): void => {
              void activateSuggestion(suggestion);
            }}
            role="option"
            type="button"
          >
            <HyTag
              background="var(--hy-tag-default-bg)"
              foreground="var(--hy-tag-default-fg)"
              label={kindLabels[suggestion.kind]}
            />

            <div class="hy-search-copy">
              <div class="hy-search-primary">
                <VwText size="sm" weight="semibold">{suggestion.primary}</VwText>
              </div>
              {#if suggestion.secondary != null}
                <div class="hy-search-secondary">
                  <VwText color="muted" size="xs">{suggestion.secondary}</VwText>
                </div>
              {/if}
            </div>
          </button>
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .hy-search-box {
    position: relative;
  }

  .hy-search-results {
    background: color-mix(in srgb, var(--surface-base) 94%, transparent);
    border: 1px solid var(--glass-border);
    border-radius: 0 0 18px 18px;
    border-top: none;
    box-shadow: 0 18px 36px color-mix(in srgb, var(--color-bg) 24%, transparent);
    color: var(--color-fg);
    left: 0;
    margin-top: 0;
    max-height: min(420px, 60vh);
    overflow-y: auto;
    padding: 6px 8px 8px;
    position: absolute;
    right: 0;
    top: calc(100% - 8px);
    z-index: 40;
  }

  .hy-search-row {
    align-items: flex-start;
    background: transparent;
    border: none;
    border-radius: 12px;
    color: inherit;
    cursor: pointer;
    display: grid;
    gap: 12px;
    grid-template-columns: max-content minmax(0, 1fr);
    padding: 9px 12px;
    text-align: left;
    width: 100%;
  }

  .hy-search-row-loading {
    display: flex;
    justify-content: flex-start;
  }

  .hy-search-row:hover,
  .selected {
    background: var(--hy-list-row-hover);
  }

  .hy-search-copy {
    min-width: 0;
  }

  .hy-search-primary,
  .hy-search-secondary {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
