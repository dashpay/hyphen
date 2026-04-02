# TypeScript Development Guide

**Table of Contents**

- [Coding Style (TypeScript)](#coding-style-typescript)
  - [Formatting](#formatting)
  - [Naming](#naming)
  - [Type Safety](#type-safety)
  - [Code Discipline](#code-discipline)
  - [Explicit Return Types](#explicit-return-types)
  - [Null and Undefined](#null-and-undefined)
  - [Unions, Narrowing and Exhaustiveness](#unions-narrowing-and-exhaustiveness)
  - [Imports](#imports)
  - [Interfaces Over Type Aliases](#interfaces-over-type-aliases)
  - [Readonly and Immutability](#readonly-and-immutability)
  - [Enums](#enums)
  - [Error Handling](#error-handling)
  - [Promises and Async](#promises-and-async)
  - [Code Comments](#code-comments)
- [Coding Style (Svelte)](#coding-style-svelte)
  - [Component Structure](#component-structure)
  - [Reactivity and State](#reactivity-and-state)
  - [Props and Snippets](#props-and-snippets)
  - [Scoped Styles](#scoped-styles)
  - [Responsiveness](#responsiveness)
  - [Performance](#performance)
  - [Accessibility](#accessibility)
- [Development Guidelines](#development-guidelines)
  - [User Interfaces](#user-interfaces)
  - [Input Validation](#input-validation)
  - [Security](#security)

## Coding Style (TypeScript)

### Formatting

- All source files must be TypeScript. JavaScript files are not permitted in the codebase
- Use 2-space indentation, LF line endings
- Lines must not exceed 120 characters
- Statements end with semicolons
- Strings use double quotes
- Trailing commas appear in multi-line structures where ES5 allows them
- Files must end with a newline
- No trailing whitespace

<details>

<summary>Example code:</summary>

```typescript
const endpoints: string[] = ["/api/v1/wallets", "/api/v1/transactions"];

function formatBalance(sats: number): string {
  return (sats / 1e8).toFixed(8);
}
```

</details>

### Naming

| Form               | Used for                                              |
| ------------------ | ----------------------------------------------------- |
| `PascalCase`       | Classes, interfaces, type aliases, enums, components  |
| `camelCase`        | Variables, functions, methods, parameters, properties |
| `UPPER_SNAKE_CASE` | Module-level constants and enum members               |

- Treat acronyms as whole words: write `httpUrl`, not `httpURL`; `txId`, not `txID`
- Do not prefix interfaces with `I`
- Do not use Hungarian notation (`nCount`, `strName`)
- Do not use leading underscores for private members

<details>

<summary>Example code:</summary>

```typescript
// Good: correct naming and casing
interface WalletBalance {
  readonly confirmed: number;
  readonly pending: number;
}

const MAX_RETRY_COUNT = 3;

function fetchWalletBalance(walletId: string): Promise<WalletBalance> {
  // ...
}

// Bad: prefix, wrong casing, Hungarian notation
interface IWalletBalance {
  readonly nConfirmed: number;
}
const maxRetryCount = 3;
function FetchWalletBalance(wallet_id: string): Promise<IWalletBalance> {
  // ...
}
```

</details>

### Type Safety

> [!IMPORTANT]
> `any` is forbidden. Use `unknown` and narrow before access. Because `any` silently opts out of every downstream check, a single untyped boundary can propagate corrupt data through an entire call chain before anyone notices.

- A value typed `any` silently disables every check downstream
- A value typed `unknown` forces the code to prove its shape before use
- Do not use non-null assertions (`!`) unless the value is provably present. If an assertion is necessary, add a comment explaining why it is safe
- Prefer type annotations on object literals rather than `as` assertions. An annotation catches surplus or missing properties; an assertion hides them

<details>

<summary>Example code:</summary>

```typescript
// Good: unknown + narrowing
function parsePayload(raw: unknown): string {
  if (typeof raw !== "object" || raw === null) {
    throw new Error("Expected object payload");
  }
  if (!("message" in raw) || typeof raw.message !== "string") {
    throw new Error("Missing 'message' field");
  }
  return raw.message;
}

// Bad: any silently bypasses every check
function parsePayload(raw: any): string {
  return raw.message;
}
```

```typescript
// Good: annotation catches mistakes at the definition site
const config: ServerConfig = {
  port: 8080,
  host: "0.0.0.0",
};

// Bad: assertion hides a missing 'host' field
const config = {
  port: 8080,
} as ServerConfig;
```

</details>

### Code Discipline

The type system already enforces many invariants. When the code also checks them manually, readers have to wonder whether the compiler missed something or the check is simply stale.

- Do not write type assertions the compiler already knows. If the type is already narrowed, an extra `as` is noise that can mask future regressions when the underlying type changes
- Do not write conditions the type system already guarantees. An `if (x !== undefined)` after a non-optional declaration is dead code that misleads readers into thinking the value could be absent
- Do not use void-returning expressions in value positions. For example, do not use the result of `array.forEach(...)` or `map.set(...)` in an assignment or return, because the intent is ambiguous and the result is always `undefined`
- Use bracket notation (`obj["key"]`) for properties that come from an index signature. Dot notation is reserved for statically known properties; bracket notation signals to readers that the key may not exist
- When overriding a base-class method, mark it with the `override` keyword. This makes the intent explicit and causes a compile error if the base method is renamed or removed
- Declare no unused variables or parameters. Dead bindings add noise and suggest unfinished work; if a parameter must exist for signature compatibility, prefix it with an underscore

<details>

<summary>Example code:</summary>

```typescript
// Good: bracket notation for index-signature access
interface Headers {
  [name: string]: string;
}
function getContentType(headers: Headers): string | undefined {
  return headers["content-type"];
}

// Bad: dot notation implies a known property
function getContentType(headers: Headers): string | undefined {
  return headers.contentType;
}
```

```typescript
// Good: override makes the relationship explicit
class JsonRpcTransport extends BaseTransport {
  override async send(payload: unknown): Promise<void> {
    // ...
  }
}

// Bad: silent override breaks if the base method is renamed
class JsonRpcTransport extends BaseTransport {
  async send(payload: unknown): Promise<void> {
    // ...
  }
}
```

</details>

### Explicit Return Types

- Every function must declare its return type, not just exported functions but internal helpers too. A drift in the inferred type of a helper silently propagates to every caller within the module
- This serves as a contract between the author and every call site
- If the implementation changes and the inferred type drifts, the explicit annotation catches the mismatch immediately
- Every code path in a function must return a value. If a branch can exit without a `return`, the compiler flags it, preventing subtle bugs where `undefined` leaks into a typed result

<details>

<summary>Example code:</summary>

```typescript
// Good: return type is explicit
export function computeFee(sats: number, rate: number): number {
  return Math.ceil(sats * rate);
}

// Bad: caller must read the body to learn the return type
export function computeFee(sats: number, rate: number) {
  return Math.ceil(sats * rate);
}
```

</details>

### Null and Undefined

- Use strict null checks: every nullable value must be handled at the point of access, not assumed away
- Prefer optional properties (`field?: T`) over explicit union (`field: T | undefined`)
- Never explicitly assign `undefined` to an optional property. Optional properties may only be _omitted_, not set to `undefined`; this distinction matters because downstream code may check `"key" in obj` to decide whether a field was provided, and an explicit `undefined` would produce a false positive
- Use `== null` to check for both `null` and `undefined` in a single comparison
- When an indexed access might not exist, the compiler requires handling `undefined` before use. Do not circumvent this with type assertions

<details>

<summary>Example code:</summary>

```typescript
// Good: guard before access
const users: Map<string, User> = new Map();
const user = users.get(id);
if (user == null) {
  throw new Error(`Unknown user: ${id}`);
}
console.log(user.name);

// Bad: non-null assertion hides a potential crash
const user = users.get(id)!;
console.log(user.name);
```

</details>

### Unions, Narrowing and Exhaustiveness

- When a value can be one of several types, use a discriminated union
- `switch` statements over discriminated unions must be exhaustive
- If a new variant is added later, every switch that forgot it will fail to compile
- Switch cases must not fall through to the next case. Each case must end with `break`, `return`, or `throw`; implicit fallthrough is a well-known source of bugs where adding a new case silently inherits the behaviour of the one below it

> [!TIP]
> Omit the `default` case in switches over discriminated unions. The compiler will error on missing variants, helping us catch unhandled cases at build time instead of at runtime.

<details>

<summary>Example code:</summary>

```typescript
interface Transfer {
  readonly kind: "transfer";
  readonly toAddress: string;
  readonly amount: number;
}

interface Deposit {
  readonly kind: "deposit";
  readonly blockHeight: number;
}

type Transaction = Transfer | Deposit;

function describeTransaction(tx: Transaction): string {
  switch (tx.kind) {
    case "transfer":
      return `Send ${tx.amount} to ${tx.toAddress}`;
    case "deposit":
      return `Deposit at block ${tx.blockHeight}`;
    // No default. If a new variant is added, the compiler
    // will error here until it is handled.
  }
}
```

</details>

### Imports

- Use ES module `import` / `export` exclusively. Never use `require()` or `namespace`
- Use `import type` when the symbol is used only as a type. This makes the boundary between runtime and compile-time dependencies explicit and guarantees the import is erased from the emitted JavaScript
- Prefer named exports over default exports. Named exports make rename-refactors and auto-imports reliable

<details>

<summary>Example code:</summary>

```typescript
// Good: named export, type-only import
import type { Snippet } from "svelte";
import { writable } from "svelte/store";

export function createCounter(): Writable<number> {
  return writable(0);
}

// Bad: default export, runtime import of a type
import Snippet from "svelte";
export default function createCounter() {
  /* ... */
}
```

</details>

### Interfaces Over Type Aliases

- Use `interface` for object shapes
- Reserve `type` for unions, intersections, tuples, mapped types, and other constructs that `interface` cannot express
- Interfaces can be extended and merged, which aids library design
- Type aliases for plain object shapes offer no advantage and reduce consistency

<details>

<summary>Example code:</summary>

```typescript
// Good: interface for an object shape
interface Block {
  readonly hash: string;
  readonly height: number;
  readonly timestamp: number;
}

// Good: type alias for a union
type NetworkStatus = "connected" | "syncing" | "disconnected";

// Bad: type alias for a plain object shape
type Block = {
  readonly hash: string;
  readonly height: number;
  readonly timestamp: number;
};
```

</details>

### Readonly and Immutability

- Mark properties `readonly` when they should not change after construction
- Prefer `readonly` arrays and tuples for function parameters
- This prevents accidental mutation and communicates intent to callers

<details>

<summary>Example code:</summary>

```typescript
// Good: caller's array cannot be mutated
function median(values: readonly number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[mid - 1]! + sorted[mid]!) / 2 : sorted[mid]!;
}

// Bad: function silently sorts the caller's array in place
function median(values: number[]): number {
  values.sort((a, b) => a - b);
  const mid = Math.floor(values.length / 2);
  return values[mid]!;
}
```

</details>

### Enums

- Prefer `enum` with `UPPER_SNAKE_CASE` members for fixed sets of related constants
- Do not use `const enum`, as it complicates cross-module boundaries and debugging

<details>

<summary>Example code:</summary>

```typescript
enum ConnectionState {
  DISCONNECTED = "disconnected",
  CONNECTING = "connecting",
  CONNECTED = "connected",
}
```

</details>

### Error Handling

- Always throw `Error` objects (or subclasses), never primitives. This preserves stack traces, which are essential for diagnosis
- Use custom error classes when callers need to distinguish error kinds
- Keep `try` blocks narrow: wrap only the code that can throw, not entire function bodies

<details>

<summary>Example code:</summary>

```typescript
// Good: typed error with preserved stack trace
class RpcError extends Error {
  constructor(
    message: string,
    readonly code: number
  ) {
    super(message);
    this.name = "RpcError";
  }
}

async function getBlock(hash: string): Promise<Block> {
  const response = await fetch(`/api/v1/blocks/${hash}`);
  if (!response.ok) {
    throw new RpcError(`Block fetch failed: ${response.statusText}`, response.status);
  }
  return (await response.json()) as Block;
}

// Bad: no stack trace, impossible to catch selectively
throw "block not found";
```

</details>

### Promises and Async

- Mark a function `async` only if it contains `await`
- Never call an `async` function without `await`-ing or returning its promise. Floating promises hide failures silently
- Do not `await` a value that is not a `Promise`
- Do not pass a promise-returning function to a callback that expects `void`. Event handlers, `forEach` callbacks, and similar APIs discard the return value, so any rejection will go unhandled
- Do not use a promise directly in a boolean condition (`if (somePromise)`). A `Promise` object is always truthy regardless of its resolved value, so the check does not do what it appears to

> [!WARNING]
> A floating promise is a silent failure. If the promise rejects, no caller will ever see the error.

<details>

<summary>Example code:</summary>

```typescript
// Good: promise is awaited, error surfaces
async function syncChain(): Promise<void> {
  const tip = await fetchChainTip();
  await processBlocks(tip);
}

// Bad: fire-and-forget hides any rejection
function syncChain(): void {
  fetchChainTip().then(processBlocks);
}
```

</details>

### Code Comments

- Use JSDoc (`/** ... */`) for documentation on exports
- Use line comments (`//`) for implementation notes
- A comment should explain _why_, not _what_. Do not restate the function's name or signature in prose
- Keep comments within 80 characters wide and at most 3 lines tall
- Extremely complex concepts may use two paragraphs at most
- Use `@param` and `@returns` only when the name and type do not already communicate the meaning

<details>

<summary>Example code:</summary>

```typescript
/**
 * Converts satoshis to a user-facing display string
 * with the given locale's decimal separator.
 */
export function formatSats(sats: number, locale: string): string {
  return (sats / 1e8).toLocaleString(locale, {
    minimumFractionDigits: 8,
  });
}

/**
 * @param   depth - Ancestor distance from chain tip (0 = tip).
 * @returns The block hash, or undefined if depth exceeds
 *          the chain height.
 */
export function getBlockHashByDepth(depth: number): string | undefined {
  // ...
}
```

```typescript
// Bad: restates the obvious, wall of text
/**
 * This function is called getBlockHashByDepth.
 * It takes a number parameter called depth and
 * returns a string or undefined. The depth parameter
 * represents how far from the tip to go.
 * It returns the hash of the block at that depth.
 * If the depth is too large it returns undefined.
 * @param depth number the depth
 * @returns string | undefined the hash
 */
export function getBlockHashByDepth(depth: number): string | undefined {
  // ...
}
```

</details>

## Coding Style (Svelte)

These rules target Svelte 5 with SvelteKit. The aim is fast first paints, small bundle impact, and correct behaviour on constrained devices.

### Component Structure

- Order sections as: `<script>`, markup, `<style>`
- The script block must use `lang="ts"`
- Keep components small and focused on one responsibility
- If a component exceeds roughly 200 lines, consider extracting sub-components

<details>

<summary>Example code:</summary>

```svelte
<script lang="ts">
  import type { WalletBalance } from "$lib/types";

  const { balance }: { balance: WalletBalance } = $props();
</script>

<div class="balance-card">
  <span class="confirmed">{balance.confirmed}</span>
  <span class="pending">{balance.pending}</span>
</div>

<style>
  .balance-card {
    display: flex;
    gap: 1rem;
  }
</style>
```

</details>

### Reactivity and State

- Use the Svelte 5 runes API: `$state`, `$derived`, `$effect`
- Do not import legacy store functions (`writable`, `derived`) in new component code
- Avoid `$effect` for anything that can be expressed as `$derived`. Effects that run on every re-render are a common source of performance problems
- When an effect is genuinely needed, keep the body narrow and never mutate state that feeds back into the same effect

> [!CAUTION]
> An `$effect` that writes to `$state` which it also reads will loop infinitely. When this pattern appears in the code, it almost certainly should be a `$derived` instead.

<details>

<summary>Example code:</summary>

```svelte
<script lang="ts">
  let count = $state(0);
  let doubled = $derived(count * 2);

  // Good: effect with a clear external side-effect
  $effect(() => {
    document.title = `Count: ${count}`;
  });
</script>

<button onclick={() => count++}>
  {count} (doubled: {doubled})
</button>
```

```svelte
<script lang="ts">
  // Bad: effect where $derived would suffice
  let count = $state(0);
  let doubled = $state(0);
  $effect(() => {
    doubled = count * 2;
  });
</script>
```

</details>

### Props and Snippets

- Declare props with `$props()` and destructure with explicit types
- Use Svelte 5 snippets (`Snippet`) instead of slots for typed child content

<details>

<summary>Example code:</summary>

```svelte
<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    children: Snippet;
    aside?: Snippet;
  }

  const { title, children, aside }: Props = $props();
</script>

<article>
  <h2>{title}</h2>
  {@render children()}
  {#if aside}
    <aside>{@render aside()}</aside>
  {/if}
</article>
```

</details>

### Scoped Styles

- All component CSS is scoped by default
- Do not use `:global()` unless styling a third-party element that cannot be targeted otherwise
- Use CSS custom properties (variables) for theming
- Do not hard-code colours, font sizes, or spacing values. A theme layer provides these at `:root`

<details>

<summary>Example code:</summary>

```svelte
<style>
  /* Good: uses design tokens */
  .card {
    background: var(--surface-primary);
    border-radius: var(--radius-md);
    padding: var(--space-md);
    color: var(--text-primary);
  }

  /* Bad: hard-coded values, fragile to theme changes */
  .card {
    background: #1a1a2e;
    border-radius: 12px;
    padding: 16px;
    color: #eee;
  }
</style>
```

</details>

### Responsiveness

- Design for the smallest supported viewport first, then add breakpoints
- Use relative units (`rem`, `%`, `dvh`) rather than fixed `px` for layout dimensions
- Prefer CSS Grid and Flexbox over absolute positioning
- Avoid layout shifts by reserving space for images and async content

<details>

<summary>Example code:</summary>

```svelte
<style>
  .dashboard {
    display: grid;
    grid-template-columns: 1fr;
    gap: var(--space-md);
    padding: var(--space-sm);
  }

  @media (min-width: 48rem) {
    .dashboard {
      grid-template-columns: 1fr 1fr;
    }
  }

  @media (min-width: 80rem) {
    .dashboard {
      grid-template-columns: 1fr 1fr 1fr;
    }
  }
</style>
```

</details>

### Performance

Every operation inside the render path directly affects time-to-paint.

- **Avoid expensive computations during render.** Move filtering, sorting, and heavy transforms into `$derived` values so they re-run only when their dependencies change
- **Minimise DOM nodes.** Fewer nodes means faster layout, faster paint, and lower memory
  - Use `{#if}` to mount content only when it is needed
  - Use `{#each}` with a key expression so Svelte can reuse nodes
- **Defer non-critical work.** Use `requestIdleCallback` or split large lists with virtual scrolling
  - Never block the main thread with synchronous processing of unbounded data
- **Limit reactive subscriptions.** Each subscription is a potential re-render trigger; a component with thirty subscriptions is thirty chances to repaint for one state change

<details>

<summary>Example code:</summary>

```svelte
<script lang="ts">
  import type { Transaction } from "$lib/types";

  const { transactions }: { transactions: Transaction[] } = $props();

  // Good: sort once when the input changes, not on every render
  let sorted = $derived([...transactions].sort((a, b) => b.timestamp - a.timestamp));
</script>

{#each sorted as tx (tx.id)}
  <TransactionRow {tx} />
{/each}
```

```svelte
<!-- Bad: sort runs every time any state changes -->
{#each [...transactions].sort((a, b) => b.timestamp - a.timestamp) as tx}
  <TransactionRow {tx} />
{/each}
```

</details>

### Accessibility

- Use semantic HTML elements (`<button>`, `<nav>`, `<main>`) rather than `<div>` with click handlers
- Ensure all interactive elements are reachable by keyboard
- Provide `aria-label` where visible text is absent

## Development Guidelines

### User Interfaces

- Build interfaces for the least capable device in the support matrix. If the UI is usable on a low-end mobile browser over a metered connection, it will be pleasant everywhere else
- Separate data-fetching from presentation. A display component should receive its data via props, never fetch it; this enables testing without network access and reuse in different contexts
- Avoid displaying raw identifiers or hashes to end users. Truncate, format, or label them; long hexadecimal strings are hostile to humans

### Input Validation

> [!CAUTION]
> Assume every value that crosses a trust boundary is adversarial. Trust boundaries include: HTTP request bodies, query parameters, headers, URL path segments, WebSocket messages, file uploads, and clipboard content.

- **Validate early, fail loudly.** Check the shape, type, length, and range of every input before it reaches business logic; return a clear error that tells the caller exactly what is wrong
- **Do not trust client-side validation.** Client-side checks improve the user experience but offer no security; the server must re-validate every input regardless
- **Limit input size.** Set maximum lengths for strings, maximum sizes for payloads, and maximum element counts for arrays; unbounded input is an invitation for denial-of-service
- **Sanitise output.** When rendering user-supplied values in HTML, encode them to prevent cross-site scripting
  - Svelte's `{expression}` syntax escapes by default
  - Never use `{@html}` with untrusted content

<details>

<summary>Example code:</summary>

```typescript
// Good: strict validation at the boundary
function parseTransferRequest(raw: unknown): Transfer {
  if (typeof raw !== "object" || raw === null) {
    throw new ValidationError("Body must be a JSON object");
  }
  const body = raw as Record<string, unknown>;

  const toAddress = body["toAddress"];
  if (typeof toAddress !== "string" || toAddress.length > 64) {
    throw new ValidationError("toAddress must be a string, max 64 chars");
  }

  const amount = body["amount"];
  if (typeof amount !== "number" || !Number.isInteger(amount) || amount <= 0) {
    throw new ValidationError("amount must be a positive integer");
  }

  return { kind: "transfer", toAddress, amount };
}

// Bad: trusts the shape, allows injection and overflow
function parseTransferRequest(raw: any): Transfer {
  return {
    kind: "transfer",
    toAddress: raw.toAddress,
    amount: raw.amount,
  };
}
```

</details>

### Security

- **Never log secrets.** Passwords, private keys, authentication tokens, and session identifiers must never appear in log output
- **Never return secrets in GET responses.** If a secret must be communicated, do so via POST and only to an authenticated, authorised caller
- **Prefer nullish coalescing over logical OR for defaults.** The `||` operator treats `0`, `""`, and `false` as falsy, which can override legitimate values; use `??` which only activates on `null` or `undefined`
- **Prefer optional chaining over manual checks.** Optional chaining (`?.`) is shorter, clearer, and avoids accidentally truthiness-checking a valid falsy value

Nullish coalescing preserves legitimate falsy values like `0`, `""`, and `false` that `||` would silently discard:

<details>

<summary>Example code:</summary>

```typescript
// Good: 0 is preserved as a valid timeout
const timeout = userTimeout ?? 30;

// Bad: user explicitly setting 0 gets overridden
const timeout = userTimeout || 30;
```

```typescript
// Good: optional chaining
const city = user?.address?.city;

// Bad: verbose manual null checks
const city = user && user.address && user.address.city;
```

</details>
