<script lang="ts" generics="TRow">
  import type { Snippet } from "svelte";

  /** Typed table shell for data-display layouts. */
  interface TableColumn<TRowValue> {
    readonly key: string;
    readonly label: string;
    readonly align?: "left" | "right" | "center";
    readonly width?: string;
    readonly render: (row: TRowValue) => Snippet;
  }

  interface Props {
    readonly columns: readonly TableColumn<TRow>[];
    readonly rows: readonly TRow[];
    readonly getRowKey?: (row: TRow) => string;
  }

  const { columns, rows, getRowKey }: Props = $props();

  function alignClass(align: TableColumn<TRow>["align"]): string {
    return align === undefined ? "align-left" : `align-${align}`;
  }
</script>

<div class="table-shell">
  <table class="table">
    <thead>
      <tr>
        {#each columns as column (column.key)}
          <th class={alignClass(column.align)} style:width={column.width}>
            {column.label}
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row (getRowKey?.(row) ?? JSON.stringify(row))}
        <tr>
          {#each columns as column (column.key)}
            <td class={alignClass(column.align)}>
              {@render column.render(row)()}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .table-shell {
    background: var(--surface-base);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    overflow-x: auto;
  }

  .table {
    border-collapse: collapse;
    min-width: 100%;
    width: 100%;
  }

  th,
  td {
    border-bottom: 1px solid var(--border-subtle);
    font-family: var(--font-primary);
    padding: 16px 20px;
  }

  th {
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  td {
    color: var(--color-fg);
    font-size: var(--text-base);
    font-weight: 500;
  }

  tbody tr:last-child td {
    border-bottom: none;
  }

  .align-left {
    text-align: left;
  }

  .align-center {
    text-align: center;
  }

  .align-right {
    text-align: right;
  }
</style>
