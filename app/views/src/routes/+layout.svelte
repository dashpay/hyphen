<script lang="ts">
  import type { Snippet } from "svelte";
  import "../app.css";

  const { children }: { children: Snippet } = $props();
</script>

<svelte:head>
  <link rel="preconnect" href="https://fonts.googleapis.com" />
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="anonymous" />
  <link
    href="https://fonts.googleapis.com/css2?family=Huninn&family=JetBrains+Mono:wght@400;500;700&family=Manrope:wght@300;500;600;800&display=swap"
    rel="stylesheet"
  />
</svelte:head>

{@render children()}
