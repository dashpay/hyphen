<script lang="ts">
  import { goto } from "$app/navigation";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import { observeDockMode } from "$lib/config/layout.svelte";
  import { HyVlTable } from "$lib/validator/views";
  import { HyUtilWebUiName } from "$lib/HyUtil";
  import { HyListPanel, HyPageFrame } from "$lib/views";

  let summary = $state(tr("{count, plural, one {# validator} other {# validators}}", { count: 0 }));
  let dockMode = $state(false);

  onMount((): (() => void) => {
    return observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
    });
  });
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Validators"))}</title>
</svelte:head>

<HyPageFrame>
  <HyListPanel fullHeight={!dockMode} subtitle={summary} title={tr("Validators")}>
    <HyVlTable
      onsummarychange={(nextSummary: string): void => {
        summary = nextSummary;
      }}
      onvalidatorclick={(proTxHash: string): void => {
        void goto("/validator?hash=" + encodeURIComponent(proTxHash));
      }}
      scrollMode={dockMode ? "page" : "self"}
    />
  </HyListPanel>
</HyPageFrame>
