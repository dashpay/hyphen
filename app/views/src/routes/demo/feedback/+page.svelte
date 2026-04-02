<script lang="ts">
  import { VwEmptyState, VwErrorState, VwLoadingState, VwNotFoundState, VwVStack } from "$lib";
</script>

<svelte:head>
  <title>hyphen-views | Feedback</title>
</svelte:head>

<VwVStack gap={24}>
  <VwLoadingState />
  <VwEmptyState />
  <VwErrorState />
  <VwNotFoundState />
</VwVStack>
