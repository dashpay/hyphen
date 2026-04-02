# REST Development Guide

**Table of Contents**

- [Route Classes](#route-classes)
  - [Historical Routes](#historical-routes)
  - [Live Routes](#live-routes)
  - [Overlaid Routes](#overlaid-routes)
  - [Raw Routes](#raw-routes)
- [URL and Method Design](#url-and-method-design)
- [Response Shape](#response-shape)
- [Pagination and Readiness](#pagination-and-readiness)
- [Search and Identity](#search-and-identity)
- [Errors and Versioning](#errors-and-versioning)

## Route Classes

REST routes are grouped by data source and time model. The route class is part of the contract.

### Historical Routes

Historical routes are snapshot-bound.

- They describe indexed data that has already been accepted into a stable historical view
- They include `asOfHeight` so the caller knows which indexed snapshot produced the response
- They are suitable for blocks, confirmed transactions, address history, balances, metrics, and other indexed products
- They may expose route-relative derived facts when those facts are part of the indexed product itself
  - Example: an address-history route may include `netValue`, `blockTime`, or a `direction` derived from signed net movement for each row
  - Clients should not be forced to reconstruct route-specific semantics from generic base fields

<details>

<summary>Rust example:</summary>

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoricalPage<T> {
  as_of_height: u64,
  value: Vec<T>,
  #[serde(skip_serializing_if = "Option::is_none")]
  next_cursor: Option<String>,
}
```

</details>

<details>

<summary>TypeScript example:</summary>

```typescript
interface HistoricalPage<T> {
  readonly asOfHeight: number;
  readonly value: readonly T[];
  readonly nextCursor?: string;
}
```

</details>

### Live Routes

Live routes are not snapshot-bound.

- They describe the current state of a live source
- They do not include `asOfHeight`
- They may become unavailable when the live capability is disabled or the live source is not ready

<details>

<summary>Rust example:</summary>

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LivePage<T> {
  value: Vec<T>,
  #[serde(skip_serializing_if = "Option::is_none")]
  next_cursor: Option<String>,
}
```

</details>

<details>

<summary>TypeScript example:</summary>

```typescript
interface LivePage<T> {
  readonly value: readonly T[];
  readonly nextCursor?: string;
}
```

</details>

### Overlaid Routes

Overlaid routes combine a historical base with optional live context.

- The top level remains the historical or indexed truth
- Live context is nested under `live`
- The nested `live` object is additive and may be absent or partially populated
- Overlay data is not bound to the historical snapshot and may be fresher than the indexed base
- Overlay fields may include live convenience projections of live facts
  - Example: validator live status may expose both `lastPaidHeight` and `lastPaidAt`

This split keeps durable facts and changing facts distinct.

<details>

<summary>Rust example:</summary>

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransactionDetail {
  as_of_height: u64,
  txid: String,
  tags: TransactionTags,
  #[serde(skip_serializing_if = "Option::is_none")]
  live: Option<TransactionLive>,
}
```

</details>

<details>

<summary>TypeScript example:</summary>

```typescript
interface TransactionDetail {
  readonly asOfHeight: number;
  readonly txid: string;
  readonly tags: TransactionTags;
  readonly live?: TransactionLive;
}
```

</details>

### Raw Routes

Raw routes are payload routes.

- They return raw serialized data or raw payload material
- They may include source or snapshot metadata
- They do not carry nested overlay fields

Raw routes exist to deliver the payload itself, not to behave like a detail route.

## URL and Method Design

- URLs are lowercase
- Multi-word path segments use kebab-case
- Resource collections use plural nouns
- Verbs do not appear in URLs
- The hierarchy stays shallow and predictable

Methods are chosen by action:

| Method | Use |
| --- | --- |
| `GET` | Read a resource or collection |
| `PUT` | Create or replace a whole resource |
| `PATCH` | Apply a partial update |
| `DELETE` | Remove a resource |
| `POST` | Trigger an action or create a server-assigned resource |

All operations are treated as idempotent from the client's point of view. A retry after transport failure should not create a different final state.

## Response Shape

- JSON uses `camelCase`
- Field names and values are case-sensitive
- Date-time values use RFC 3339
- Durations use explicit units in field names
- Absent fields are omitted rather than serialized as `null`
- When one route needs extra page context beyond the shared envelope, that context lives under a typed `meta` object

One exception exists for tags:

- `tags` may summarize useful facts at the top level
- `live.tags` may summarize the live side of an overlaid response
- Full base fields are not duplicated under `live`

## Pagination and Readiness

Cursor pagination is used because deep offset pagination degrades badly on large datasets.

- Collections return `value`
- A further page is indicated by `nextCursor`
- The final page omits `nextCursor`
- Total counts are avoided unless they are part of the product itself

Readiness is part of the contract:

- Historical routes may be unavailable until their indexed dataset is ready
- Live routes may be unavailable when the live capability is disabled or its source is not ready
- The same route may exist in the API description even when it is temporarily unavailable in a running deployment

## Search and Identity

Search results are discriminated by kind.

- Each result includes a `type`
- Each result kind carries only the fields that belong to that kind
- The result `id` is the canonical public identifier of the matched object
- The result `id` is not a copy of the input query when the query has multiple valid forms

This keeps search typed and predictable for clients.

## Errors and Versioning

Errors are structured:

- `code` is stable and machine-readable
- `message` is human-readable
- `target` may identify the field or parameter at fault

Versioning uses a major version in the URL, such as `/api/v1/`.

- Breaking changes do not occur within a published major version
- Removing fields, renaming fields, changing field types, adding new required fields, or changing the meaning of an error code are all breaking changes
