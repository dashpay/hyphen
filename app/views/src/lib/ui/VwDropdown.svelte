<script lang="ts" module>
  /** Dropdown field for selecting one option from a list. */
  let dropdownIdSequence = 0;
</script>

<script lang="ts">
  interface DropdownOption {
    readonly value: string;
    readonly label: string;
  }

  interface Props {
    readonly options: DropdownOption[];
    readonly value?: string;
    readonly disabled?: boolean;
    readonly onchange?: (value: string) => void;
    readonly placeholder?: string;
    readonly label?: string;
    readonly compact?: boolean;
  }

  let {
    options,
    value = $bindable(""),
    disabled = false,
    onchange,
    placeholder = "Select...",
    label,
    compact = false,
  }: Props = $props();

  let open = $state(false);
  let dropdownEl: HTMLDivElement | undefined = $state();
  const labelId = `dropdown-label-${String(++dropdownIdSequence)}`;

  const selectedLabel = $derived(options.find((option) => option.value === value)?.label ?? placeholder);

  function toggle(): void {
    if (disabled) {
      return;
    }
    open = !open;
  }

  function select(option: DropdownOption): void {
    value = option.value;
    open = false;
    onchange?.(option.value);
  }

  function handleTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      open = false;
    }
  }

  function handleOptionKeydown(event: KeyboardEvent, option: DropdownOption): void {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }

    event.preventDefault();
    select(option);
  }

  function handleWindowClick(event: MouseEvent): void {
    const target = event.target;
    if (!(target instanceof HTMLElement)) {
      return;
    }

    if (dropdownEl == null || dropdownEl.contains(target)) {
      return;
    }

    if (open) {
      open = false;
    }
  }
</script>

<svelte:window onclick={handleWindowClick} />

<div bind:this={dropdownEl} class:disabled class:compact class="dropdown">
  {#if label}
    <span class="dropdown-label" id={labelId}>{label}</span>
  {/if}
  <button
    aria-expanded={open}
    aria-haspopup="listbox"
    aria-labelledby={label != null ? labelId : undefined}
    class:open
    class="dropdown-trigger"
    {disabled}
    onclick={toggle}
    onkeydown={handleTriggerKeydown}
    type="button"
  >
    <span class:placeholder={!value} class="dropdown-value">{selectedLabel}</span>
    <span class:rotated={open} class="dropdown-arrow">&#9662;</span>
  </button>
  {#if open}
    <div class="dropdown-menu" role="listbox">
      {#each options as option (option.value)}
        <div
          aria-selected={option.value === value}
          class:selected={option.value === value}
          class="dropdown-option"
          onclick={(): void => {
            select(option);
          }}
          onkeydown={(event: KeyboardEvent): void => {
            handleOptionKeydown(event, option);
          }}
          role="option"
          tabindex={0}
        >
          {option.label}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .dropdown {
    display: inline-flex;
    flex-direction: column;
    gap: 6px;
    position: relative;
  }

  .disabled {
    opacity: 0.4;
    pointer-events: none;
  }

  .dropdown-label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: 12px;
    font-weight: 600;
  }

  .dropdown-trigger {
    align-items: center;
    background: var(--accent-blue-15);
    border: none;
    border-radius: var(--radius-input);
    color: var(--color-fg);
    cursor: pointer;
    display: flex;
    font-family: var(--font-primary);
    font-size: 14px;
    font-weight: 500;
    gap: var(--space-2);
    justify-content: space-between;
    min-width: 180px;
    padding: 12px 16px;
    transition: box-shadow var(--duration-fast) var(--ease-default);
  }

  .dropdown-trigger:focus-visible,
  .open {
    box-shadow: 0 0 0 2px var(--accent-blue);
  }

  .placeholder {
    color: var(--text-tertiary);
  }

  .dropdown-arrow {
    font-size: 10px;
    transition: transform var(--duration-fast) var(--ease-default);
  }

  .rotated {
    transform: rotate(180deg);
  }

  .dropdown-menu {
    background: var(--surface-base);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-input);
    box-shadow: var(--shadow-card);
    left: 0;
    margin-top: 4px;
    padding: 4px;
    position: absolute;
    right: 0;
    top: 100%;
    z-index: 10;
  }

  .dropdown-option {
    border-radius: 8px;
    color: var(--color-fg);
    cursor: pointer;
    font-family: var(--font-primary);
    font-size: 14px;
    font-weight: 500;
    padding: 10px 12px;
    transition: background var(--duration-fast) var(--ease-default);
  }

  .dropdown-option:hover {
    background: var(--accent-blue-15);
  }

  .selected {
    color: var(--accent-blue);
    font-weight: 600;
  }

  .dropdown-option:focus-visible {
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .compact .dropdown-trigger {
    border-radius: var(--radius-pill);
    font-size: var(--text-sm-plus);
    min-width: auto;
    padding: 8px 16px;
  }

  .compact .dropdown-trigger:hover {
    background: rgba(76, 126, 255, 0.25);
  }

  .compact .dropdown-menu {
    border-radius: var(--radius-nav);
    min-width: 160px;
    padding: var(--space-2);
  }

  .compact .dropdown-option {
    border-radius: var(--radius-input);
    font-size: var(--text-sm);
    padding: 8px 12px;
  }
</style>
