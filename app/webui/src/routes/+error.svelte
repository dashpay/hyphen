<script lang="ts">
  import { tr } from "@repo/hyphen-i18n";
  import { HyUtilWebUiName } from "$lib/HyUtil";
  import { HyHttpError } from "$lib/views";

  interface Props {
    readonly error: {
      readonly message: string;
    };
    readonly status: number;
  }

  let { error, status }: Props = $props();

  const message = $derived.by((): string => {
    if (status === 404) {
      return tr("This page could not be found.");
    }

    return error.message;
  });
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Request Failure"))}</title>
</svelte:head>

<HyHttpError {message} {status} />
