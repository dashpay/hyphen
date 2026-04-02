<script lang="ts">
  import { tr } from "@repo/hyphen-i18n";
  import { HyUtilWebUiName } from "$lib/HyUtil";
  import { HyHttpError } from "$lib/views";
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Request Failure"))}</title>
</svelte:head>

<HyHttpError message={tr("This request could not be completed. Please try again later.")} status={500} />
