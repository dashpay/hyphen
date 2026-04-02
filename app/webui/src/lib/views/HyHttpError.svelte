<script lang="ts">
  /** Render an HTTP-style error state with the shared Hy shell error art. */
  import { tr } from "@repo/hyphen-i18n";
  import HyCenteredState from "./HyCenteredState.svelte";
  import HyShellError from "./HyShellError.svelte";

  interface Props {
    readonly message: string;
    readonly status: number;
  }

  const { message, status }: Props = $props();

  const title = $derived.by((): string => {
    if (message === `HTTP ${String(status)}`) {
      if (status === 404) {
        return tr("This page could not be found.");
      }

      return tr("This request could not be completed. Please try again later.");
    }

    return message;
  });
</script>

<HyCenteredState>
  <HyShellError message={`HTTP ${String(status)}`} {title} />
</HyCenteredState>
