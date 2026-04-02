<script lang="ts">
  /** Error shell for configuration and top-level explorer failures. */
  import { VwText } from "@repo/hyphen-views";
  import { HyIcError } from "$lib/icons";

  interface Props {
    readonly message: string;
    readonly title: string;
  }

  const { message, title }: Props = $props();
</script>

<div class="shell-error">
  <HyIcError />
  <div class="error-info">
    <VwText as="p" size="xl" weight="extrabold">{title}</VwText>
    <VwText as="p" color="muted" size="md">{message}</VwText>
  </div>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .shell-error {
    align-items: center;
    display: flex;
    gap: var(--hy-shell-error-gap);
    height: 100%;
    justify-content: center;
    padding: var(--hy-shell-error-padding);
  }

  .error-info {
    display: flex;
    flex-direction: column;
    gap: var(--hy-inline-gap);
  }

  .error-info :global(p:first-child) {
    line-height: 1.2;
    max-width: var(--hy-shell-error-max-width);
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .shell-error {
      flex-direction: column;
      gap: var(--hy-shell-error-gap-mobile);
      padding: var(--hy-shell-error-padding-mobile);
      text-align: center;
    }
  }
</style>
