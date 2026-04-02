<script lang="ts" module>
  /** Binary toggle with accessible button semantics. */
  let toggleIdSequence = 0;
</script>

<script lang="ts">
  interface Props {
    checked?: boolean;
    readonly disabled?: boolean;
    readonly onchange?: (checked: boolean) => void;
    readonly label?: string;
  }

  let { checked = $bindable(false), disabled = false, onchange, label }: Props = $props();
  const labelId = `toggle-label-${String(++toggleIdSequence)}`;

  function handleClick(): void {
    if (disabled) {
      return;
    }
    checked = !checked;
    onchange?.(checked);
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key !== " " && event.key !== "Enter") {
      return;
    }
    event.preventDefault();
    handleClick();
  }
</script>

<div class:disabled class="toggle-wrap">
  {#if label}
    <span class="toggle-label" id={labelId}>{label}</span>
  {/if}
  <div
    aria-checked={checked}
    aria-disabled={disabled}
    aria-label={label == null ? "VwToggle" : undefined}
    aria-labelledby={label != null ? labelId : undefined}
    class:checked
    class="toggle-track"
    onclick={handleClick}
    onkeydown={handleKeydown}
    role="switch"
    tabindex={disabled ? -1 : 0}
  >
    <div class="toggle-thumb"></div>
  </div>
</div>

<style>
  .toggle-wrap {
    align-items: center;
    display: inline-flex;
    gap: var(--space-3);
  }

  .disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }

  .toggle-label {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: 14px;
    font-weight: 500;
  }

  .toggle-track {
    background: var(--switch-track);
    border-radius: var(--radius-full);
    cursor: pointer;
    height: 26px;
    position: relative;
    transition: background var(--duration-fast) var(--ease-default);
    width: 48px;
  }

  .toggle-track:focus-visible {
    outline: 2px solid var(--accent-blue);
    outline-offset: 2px;
  }

  .disabled .toggle-track {
    cursor: not-allowed;
  }

  .toggle-thumb {
    background: var(--switch-knob);
    border-radius: var(--radius-full);
    height: 26px;
    left: 0;
    position: absolute;
    top: 0;
    transition: transform var(--duration-fast) var(--ease-default);
    width: 26px;
  }

  .checked .toggle-thumb {
    transform: translateX(22px);
  }
</style>
