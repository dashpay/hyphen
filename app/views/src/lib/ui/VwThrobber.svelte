<script lang="ts">
  /** Throbber used by loading states without extra copy. */
  interface Props {
    readonly size?: number;
  }

  const { size = 40 }: Props = $props();
</script>

<div
  aria-label="Loading"
  class="throbber"
  role="status"
  style:height={`${String(size)}px`}
  style:width={`${String(size)}px`}
></div>

<style>
  .throbber {
    animation: spin 1s linear infinite;
    border: 3px solid var(--surface-raised);
    border-radius: var(--radius-full);
    border-top-color: var(--accent-blue);
    box-sizing: border-box;
    display: block;
  }

  @keyframes spin {
    from {
      transform: rotate(0deg);
    }

    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .throbber {
      animation: none;
    }
  }
</style>
