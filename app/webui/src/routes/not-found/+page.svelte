<script lang="ts">
  import { tr } from "@repo/hyphen-i18n";
  import { HyUtilWebUiName } from "$lib/HyUtil";
  import { HyHttpError } from "$lib/views";
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Not Found"))}</title>
</svelte:head>

<HyHttpError message={tr("This page could not be found.")} status={404} />
