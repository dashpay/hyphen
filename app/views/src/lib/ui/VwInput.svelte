<script lang="ts">
  /** Text input with the shared field surface styling. */
  type InputType = "text" | "password" | "email" | "number" | "search";

  interface Props {
    readonly type?: InputType;
    readonly value?: string;
    readonly placeholder?: string;
    readonly disabled?: boolean;
    readonly label?: string;
    readonly ariaLabel?: string;
    readonly fullWidth?: boolean;
  }

  let {
    type = "text",
    value = $bindable(""),
    placeholder = "",
    disabled = false,
    label,
    ariaLabel,
    fullWidth = false,
  }: Props = $props();

  const classes = $derived(fullWidth ? "input-wrap full-width" : "input-wrap");
  const fallbackAriaLabel = $derived.by((): string | undefined => {
    if (label != null) {
      return undefined;
    }

    return ariaLabel ?? (placeholder.length > 0 ? placeholder : "VwInput field");
  });

  function handleInput(event: Event): void {
    const target = event.currentTarget;
    if (!(target instanceof HTMLInputElement)) {
      return;
    }
    value = target.value;
  }
</script>

<label class={classes}>
  {#if label}
    <span class="input-label">{label}</span>
  {/if}
  <input
    aria-label={fallbackAriaLabel}
    class="input-field"
    {type}
    {value}
    oninput={handleInput}
    {placeholder}
    {disabled}
  />
</label>

<style>
  .input-wrap {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .full-width {
    width: 100%;
  }

  .input-label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: 12px;
    font-weight: 600;
  }

  .input-field {
    -webkit-backdrop-filter: blur(var(--blur-radius));
    backdrop-filter: blur(var(--blur-radius));
    background: var(--accent-blue-15);
    border: none;
    border-radius: var(--radius-input);
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-md-plus);
    font-weight: 500;
    outline: none;
    padding: 11px 16px 11px 25px;
    transition: box-shadow var(--duration-fast) var(--ease-default);
    width: 100%;
  }

  .input-field::placeholder {
    color: var(--accent-blue);
    font-weight: 600;
  }

  .input-field:focus {
    box-shadow: 0 0 0 2px var(--accent-blue);
  }

  .input-field:disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }
</style>
