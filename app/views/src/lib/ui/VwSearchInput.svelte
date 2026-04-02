<script lang="ts">
  /** Search field with the shared leading-icon treatment. */
  interface Props {
    readonly value?: string;
    readonly placeholder?: string;
    readonly ariaLabel?: string;
    readonly disabled?: boolean;
    readonly onblur?: () => void;
    readonly onfocus?: () => void;
    readonly oninput?: (query: string) => void;
    readonly onkeydown?: (event: KeyboardEvent) => void;
    readonly onsearch?: (query: string) => void;
  }

  let {
    value = $bindable(""),
    placeholder = "Search...",
    ariaLabel = "Search",
    disabled = false,
    onblur,
    onfocus,
    oninput,
    onkeydown,
    onsearch,
  }: Props = $props();

  function handleInput(event: Event): void {
    const target = event.currentTarget;
    if (!(target instanceof HTMLInputElement)) {
      return;
    }
    value = target.value;
    oninput?.(value);
  }

  function handleKeydown(event: KeyboardEvent): void {
    onkeydown?.(event);
    if (event.defaultPrevented) {
      return;
    }
    const query = value.trim();
    if (event.key !== "Enter" || query.length === 0) {
      return;
    }
    onsearch?.(query);
  }
</script>

<div class:disabled class="search-input">
  <svg aria-hidden="true" class="icon" fill="none" viewBox="0 0 16 16">
    <circle cx="7" cy="7" r="5.5" stroke="var(--text-tertiary)" stroke-linecap="round" stroke-width="1.5" />
    <path d="M11 11L14 14" stroke="var(--text-tertiary)" stroke-linecap="round" stroke-width="1.5" />
  </svg>
  <input
    aria-label={ariaLabel}
    class="field"
    {disabled}
    {onblur}
    {onfocus}
    oninput={handleInput}
    onkeydown={handleKeydown}
    {placeholder}
    type="search"
    {value}
  />
</div>

<style>
  .search-input {
    align-items: center;
    background: var(--search-bg);
    border: 1px solid var(--search-border);
    border-radius: var(--radius-input);
    display: flex;
    gap: var(--space-3);
    padding: 15px;
    transition: box-shadow var(--duration-fast) var(--ease-default);
  }

  .search-input:focus-within {
    box-shadow: 0 0 0 2px var(--accent-blue);
  }

  .disabled {
    opacity: 0.4;
    pointer-events: none;
  }

  .icon {
    flex-shrink: 0;
    height: 16px;
    width: 16px;
  }

  .field {
    background: transparent;
    border: none;
    color: var(--color-fg);
    flex: 1;
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 500;
    outline: none;
  }

  .field::placeholder {
    color: var(--text-tertiary);
  }
</style>
