<script lang="ts">
  /** Transaction tag chip for shared type and tone rendering. */
  import type { TransactionTone } from "$lib/tx/display";
  import { HyTag } from "$lib/views";

  interface Props {
    readonly label: string;
    readonly tone: TransactionTone;
  }

  const { label, tone }: Props = $props();

  function getToneColors(nextTone: TransactionTone): { background: string; foreground: string } {
    switch (nextTone) {
      case "default":
        return { background: "var(--hy-tag-default-bg)", foreground: "var(--hy-tag-default-fg)" };
      case "assetlock":
        return { background: "var(--hy-tag-assetlock-bg)", foreground: "var(--hy-tag-assetlock-fg)" };
      case "assetunlock":
        return { background: "var(--hy-tag-assetunlock-bg)", foreground: "var(--hy-tag-assetunlock-fg)" };
      case "coinbase":
        return { background: "var(--hy-tag-coinbase-bg)", foreground: "var(--hy-tag-coinbase-fg)" };
      case "data":
        return { background: "var(--hy-tag-data-bg)", foreground: "var(--hy-tag-data-fg)" };
      case "mnhf":
        return { background: "var(--hy-tag-mnhf-bg)", foreground: "var(--hy-tag-mnhf-fg)" };
      case "proreg":
        return { background: "var(--hy-tag-proreg-bg)", foreground: "var(--hy-tag-proreg-fg)" };
      case "proupreg":
        return { background: "var(--hy-tag-proupreg-bg)", foreground: "var(--hy-tag-proupreg-fg)" };
      case "prouprev":
        return { background: "var(--hy-tag-prouprev-bg)", foreground: "var(--hy-tag-prouprev-fg)" };
      case "proupserv":
        return { background: "var(--hy-tag-proupserv-bg)", foreground: "var(--hy-tag-proupserv-fg)" };
      case "quorum":
        return { background: "var(--hy-tag-quorum-bg)", foreground: "var(--hy-tag-quorum-fg)" };
    }
  }

  const colors = $derived.by((): { background: string; foreground: string } => getToneColors(tone));
</script>

<span class="transaction-tag">
  <HyTag background={colors.background} foreground={colors.foreground} {label} />
</span>

<style>
  .transaction-tag {
    display: inline-flex;
    min-width: 0;
  }
</style>
