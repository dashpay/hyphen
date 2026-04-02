# Rust Development Guide

**Table of Contents**

- [Coding Style (Rust)](#coding-style-rust)
  - [Formatting](#formatting)
  - [Naming](#naming)
  - [Type Safety](#type-safety)
  - [Error Handling](#error-handling)
  - [Ownership and Borrowing](#ownership-and-borrowing)
  - [Conversions](#conversions)
  - [Traits and Implementations](#traits-and-implementations)
  - [Generics](#generics)
  - [Iterators](#iterators)
  - [Builder Pattern](#builder-pattern)
  - [Code Comments](#code-comments)
- [Development Guidelines](#development-guidelines)
  - [User Interfaces](#user-interfaces)
    - [Terminal User Interfaces](#terminal-user-interfaces)
  - [Input Validation](#input-validation)
  - [Security](#security)

## Coding Style (Rust)

### Formatting

- Use 2-space indentation, LF line endings
- Files end with a single newline
- No trailing whitespace

<details>

<summary>Example code:</summary>

```rust
fn process_block(
  height: u64,
) -> Result<Block, DecodeError> {
  let raw = fetch_raw_block(height)?;
  let block = decode_block(&raw)?;
  Ok(block)
}
```

</details>

### Naming

| Form                   | Used for                                            |
| ---------------------- | --------------------------------------------------- |
| `UpperCamelCase`       | Types, traits, enum variants, type parameters       |
| `snake_case`           | Functions, methods, variables, modules, crate names |
| `SCREAMING_SNAKE_CASE` | Constants and statics                               |
| `'lowercase`           | Lifetimes (`'a`, `'de`, `'src`)                     |

- Treat acronyms as whole words: write `HttpUrl`, not `HTTPUrl`; `TxId`, not `TXID`
- Getters omit the `get_` prefix because the field name alone is unambiguous; a mutable getter appends `_mut`
- Primary constructors are named `new`; alternatives use descriptive names (`open`, `connect`) or `with_` suffixes (`with_capacity`)
- Conversion constructors use `from_` followed by the source type: `Error::from_raw_os_error`
- Error type names follow verb-object-error word order so they sort and read predictably: `ParseIntError`, not `IntParseError`

<details>

<summary>Example code:</summary>

```rust
struct ChainTip {
  height: u64,
  hash: BlockHash,
}

impl ChainTip {
  // Getter omits the get_ prefix; the field name alone is unambiguous.
  fn height(&self) -> u64 {
    self.height
  }

  // Mutable getter appends _mut to distinguish from the immutable form.
  fn hash_mut(&mut self) -> &mut BlockHash {
    &mut self.hash
  }
}

const MAX_CONNECTIONS: u32 = 128;
```

</details>

### Type Safety

The type system is the first line of defence. A constraint expressed as a type is checked at compile time and costs nothing at runtime.

- Wrap primitive types in newtypes when two values of the same underlying type carry different semantics; this prevents accidental transposition of arguments
- Prefer enums over booleans for function parameters because `Widget::new(Small, Round)` communicates intent where `Widget::new(true, false)` does not
- Make invalid states unrepresentable; if a combination of fields is logically impossible, restructure the type so the compiler rejects it
- Derive common traits eagerly on public types: `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, `Default`; the orphan rule prevents downstream crates from adding them later
- All public types implement `Debug` because diagnostic output, logging, and test assertions depend on it; for types holding sensitive data, provide a custom implementation that redacts the secret

> [!TIP]
> Prefer `#[derive]` for standard trait implementations. A manual implementation is warranted only when the derived behaviour would be incorrect or when redaction is needed.

<details>

<summary>Example code:</summary>

```rust
// Good: newtypes prevent mixing up amounts and heights at the call site.
struct Satoshis(u64);
struct BlockHeight(u64);

fn confirm_at(
  amount: Satoshis,
  height: BlockHeight,
) {
  // The compiler rejects confirm_at(height, amount); the types differ.
}

// Good: enum parameters are self-documenting; the call site reads naturally.
enum Size { Small, Large }
enum Shape { Round, Square }

fn create_widget(
  size: Size,
  shape: Shape,
) -> Widget {
  // ...
}
```

```rust
// Good: custom Debug redacts the secret from log output and panic messages.
struct ApiKey(String);

impl std::fmt::Debug for ApiKey {
  fn fmt(
    &self,
    f: &mut std::fmt::Formatter<'_>,
  ) -> std::fmt::Result {
    write!(f, "ApiKey(..)")
  }
}
```

</details>

### Error Handling

> [!IMPORTANT]
> Never call `.unwrap()` or `.expect()` on `Result` or `Option`. A panic converts a recoverable failure into process-level failure; depending on the panic strategy, the code may unwind or abort, but either outcome is unacceptable for routine error handling. Propagate errors with `?` or handle them explicitly with `match`.

- Define domain-specific error enums that implement `std::error::Error` and remain usable as `Send + Sync`; this makes errors composable across crate boundaries and usable in threaded contexts
- Error messages are lowercase and carry no trailing punctuation; they may be embedded in larger messages by callers, so capitalization and periods would read awkwardly in the middle of a sentence
- Keep `?` chains short; if a function contains more than a few `?` calls on unrelated operations, each operation likely belongs in its own function
- Use `#[expect]` instead of `#[allow]` when suppressing a lint; `expect` causes a warning if the suppression becomes unnecessary, preventing stale overrides from accumulating silently
- The `.todo()`, `.unimplemented()`, and `.unreachable()` family of panicking stubs are not permitted; if a branch is genuinely unreachable, restructure the types so the compiler can prove it

<details>

<summary>Example code:</summary>

```rust
use thiserror::Error;

#[derive(Debug, Error)]
enum WalletError {
  #[error("insufficient balance: need {need}, have {have}")]
  InsufficientBalance { need: u64, have: u64 },

  #[error("address failed checksum validation")]
  InvalidAddress,

  #[error(transparent)]
  Io(#[from] std::io::Error),
}

// Good: propagation with ?; the caller decides how to handle the error.
fn send(
  wallet: &Wallet,
  to: &Address,
  amount: Satoshis,
) -> Result<TxId, WalletError> {
  let balance = wallet.confirmed_balance()?;
  if balance.0 < amount.0 {
    return Err(WalletError::InsufficientBalance {
      need: amount.0,
      have: balance.0,
    });
  }
  let tx = wallet.build_transaction(to, amount)?;
  Ok(tx.id())
}
```

```rust
// Bad: unwrap turns a recoverable error into immediate process termination.
fn send(
  wallet: &Wallet,
  to: &Address,
  amount: Satoshis,
) -> TxId {
  let balance =
    wallet.confirmed_balance().unwrap();
  wallet
    .build_transaction(to, amount)
    .unwrap()
    .id()
}
```

</details>

### Ownership and Borrowing

Rust's ownership model eliminates data races and use-after-free at compile time. Working with it, rather than around it, produces code that is both safe and efficient.

- Prefer borrowing over cloning; cloning allocates and copies, and signals to readers that the caller needs an independent copy
- Accept the most general reference that satisfies the function: `&str` rather than `&String`, `&[T]` rather than `&Vec<T>`; this lets callers pass any type that dereferences to the expected slice
- Return owned values when the caller needs ownership; returning a reference to a local is a compile error, and this is intentional
- Use `Cow<'_, str>` when a function sometimes borrows and sometimes allocates; it avoids unnecessary copies on the common path
- Let the caller decide when to clone; a function that silently clones its input adds hidden cost and prevents the caller from choosing a cheaper alternative
- Generic reader and writer functions accept by value (`R: Read`) because `Read` is implemented for `&mut R`; the caller can pass a mutable reference without giving up ownership

<details>

<summary>Example code:</summary>

```rust
// Good: accepts &str; works transparently with String, &str, and Cow alike.
fn normalize_label(label: &str) -> String {
  label.trim().to_lowercase()
}

// Bad: forces the caller to own a String even when a borrow would suffice.
fn normalize_label(label: &String) -> String {
  label.trim().to_lowercase()
}
```

```rust
use std::borrow::Cow;

// Good: avoids allocation when the input already carries the required prefix.
fn ensure_prefix(s: &str) -> Cow<'_, str> {
  if s.starts_with("0x") {
    Cow::Borrowed(s)
  } else {
    Cow::Owned(format!("0x{s}"))
  }
}
```

</details>

### Conversions

Consistent conversion names tell the reader the cost and ownership semantics of an operation at a glance.

| Prefix  | Cost                  | Ownership      | Example                |
| ------- | --------------------- | -------------- | ---------------------- |
| `as_`   | Free                  | `&T` to `&U`   | `str::as_bytes()`      |
| `to_`   | Allocates or computes | Borrows input  | `str::to_lowercase()`  |
| `into_` | Variable              | Consumes input | `String::into_bytes()` |

- Implement `From<T>` for infallible conversions; the blanket impl provides `Into<T>` automatically, so we never implement `Into` directly
- Implement `TryFrom<T>` for conversions that can fail; the associated error type documents what can go wrong
- Implement `AsRef<T>` and `AsMut<T>` for cheap reference-to-reference conversions; generic functions bounded on `AsRef<Path>` accept `&str`, `String`, `PathBuf`, and `&Path` without overloads

<details>

<summary>Example code:</summary>

```rust
struct BlockHash([u8; 32]);

// Good: From for an infallible conversion; no error case to handle.
impl From<[u8; 32]> for BlockHash {
  fn from(bytes: [u8; 32]) -> Self {
    Self(bytes)
  }
}

// Good: TryFrom for a fallible conversion; the error type documents failure.
impl TryFrom<&[u8]> for BlockHash {
  type Error = InvalidHashLength;

  fn try_from(
    slice: &[u8],
  ) -> Result<Self, Self::Error> {
    let arr: [u8; 32] = slice
      .try_into()
      .map_err(|_| InvalidHashLength(slice.len()))?;
    Ok(Self(arr))
  }
}

// Good: as_ prefix signals a free borrow conversion with no allocation.
impl BlockHash {
  fn as_bytes(&self) -> &[u8; 32] {
    &self.0
  }
}
```

</details>

### Traits and Implementations

- Derive standard traits eagerly; the orphan rule prevents downstream crates from adding them, so we provide everything applicable up front
- Place `Serialize` and `Deserialize` behind an optional Cargo feature so crates that do not need serialization avoid the dependency
- `Send` and `Sync` are implemented automatically for most types; verify with a compile-time assertion when the type contains raw pointers or interior mutability
- Design traits to be object-safe (no generic methods, no `Self` in return position) unless there is a concrete reason to require static dispatch; object safety enables heterogeneous collections and smaller binaries
- Sealed traits prevent downstream implementations; this allows adding methods in future versions without a breaking change

<details>

<summary>Example code:</summary>

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
  feature = "serde",
  derive(serde::Serialize, serde::Deserialize),
)]
pub struct Peer {
  pub address: SocketAddr,
  pub protocol_version: u32,
}
```

```rust
// Sealed trait: only types inside this crate are permitted to implement it.
pub trait Encoder: private::Sealed {
  fn encode(&self, buf: &mut Vec<u8>);
}

mod private {
  pub trait Sealed {}
  impl Sealed for super::CompactEncoder {}
  impl Sealed for super::VerboseEncoder {}
}
```

</details>

### Generics

- Accept generic parameters to widen the set of types a function works with; `impl AsRef<Path>` is more useful than `&Path` because it also accepts `&str` and `String`
- Use `impl Trait` in argument position for simple, single-use bounds; use named type parameters when the same bound appears in multiple arguments or the return type
- Prefer static dispatch for performance-critical paths; use `dyn Trait` when the set of concrete types is open or when reducing binary size matters more than call overhead
- Do not add trait bounds to struct definitions unless the bound is required for the struct's own invariants; bounds on impls are sufficient and avoid constraining downstream code

<details>

<summary>Example code:</summary>

```rust
use std::path::Path;

// Good: accepts &str, String, PathBuf, and &Path through a single parameter.
fn read_config(
  path: impl AsRef<Path>,
) -> Result<Config, ConfigError> {
  let bytes = std::fs::read(path.as_ref())?;
  parse_config(&bytes)
}
```

```rust
// Good: dyn Trait collects heterogeneous filter implementations in one slice.
fn apply_filters(
  data: &mut Vec<Record>,
  filters: &[Box<dyn Filter>],
) {
  for filter in filters {
    data.retain(|r| filter.matches(r));
  }
}
```

</details>

### Iterators

Iterator chains express data transformations declaratively. The compiler often optimises them into tight loops with no intermediate allocations.

- Implement `iter()`, `iter_mut()`, and `into_iter()` on collection types; the return types are named `Iter`, `IterMut`, and `IntoIter` respectively
- Implement `FromIterator` and `Extend` so the collection works with `.collect()` and `.extend()`
- Prefer `filter_map()` when filtering and mapping happen together; it expresses the intent in one place
- Avoid `.collect()` when the result is only iterated once; return `impl Iterator<Item = T>` instead to defer allocation
- Implement `size_hint()` on custom iterators so downstream consumers can pre-allocate accurately

<details>

<summary>Example code:</summary>

```rust
// Good: filter_map fuses the filter and map steps into a single pass.
fn confirmed_amounts(
  txs: &[Transaction],
) -> impl Iterator<Item = u64> + '_ {
  txs.iter().filter_map(|tx| {
    if tx.confirmations > 0 {
      Some(tx.amount)
    } else {
      None
    }
  })
}

// Less efficient: separate filter and map produce two iterator adapters.
fn confirmed_amounts_v2(
  txs: &[Transaction],
) -> impl Iterator<Item = u64> + '_ {
  txs.iter()
    .filter(|tx| tx.confirmations > 0)
    .map(|tx| tx.amount)
}
```

</details>

### Builder Pattern

The builder pattern separates complex construction from usage. It is the right tool when a type has many optional parameters or when construction requires validation.

- Prefer non-consuming builders: methods take `&mut self` and return `&mut Self`, which allows both chained and multi-statement construction
- Perform validation in the terminal `build()` method and return `Result`; this concentrates all invariant checks in one place
- Name the builder `TypeNameBuilder` and provide a `TypeName::builder()` associated function for discoverability

<details>

<summary>Example code:</summary>

```rust
struct ServerConfig {
  host: String,
  port: u16,
  max_connections: u32,
}

struct ServerConfigBuilder {
  host: Option<String>,
  port: Option<u16>,
  max_connections: u32,
}

impl ServerConfigBuilder {
  fn new() -> Self {
    Self {
      host: None,
      port: None,
      max_connections: 128,
    }
  }

  fn host(
    &mut self,
    h: &str,
  ) -> &mut Self {
    self.host = Some(h.to_owned());
    self
  }

  fn port(
    &mut self,
    p: u16,
  ) -> &mut Self {
    self.port = Some(p);
    self
  }

  fn build(
    &self,
  ) -> Result<ServerConfig, ConfigError> {
    Ok(ServerConfig {
      host: self
        .host
        .clone()
        .ok_or(ConfigError::MissingField("host"))?,
      port: self
        .port
        .ok_or(ConfigError::MissingField("port"))?,
      max_connections: self.max_connections,
    })
  }
}
```

</details>

### Code Comments

Comments explain intent and context that the code alone cannot convey. Restating what the code does adds noise and drifts out of sync with the implementation.

#### Inline Comments

- Line comments (`//`) must not exceed 80 characters wide and 3 lines tall
- An extremely complex algorithm may use two short paragraphs separated by a blank comment line
- Focus on _why_ a decision was made, not _what_ the code does

#### Rustdoc Comments

- Documentation comments (`///`) must not exceed 80 characters wide
- The summary is at most 3 lines; do not restate the function name or signature in prose because the reader can see them directly above
- Document `# Errors` when the function returns `Result`, listing each error variant and its cause
- Document `# Panics` only when the function can panic, which should be rare
- Pad lines so right columns align evenly for visual consistency
- Use `?` in doc examples instead of `.unwrap()` so readers copy safe patterns

<details>

<summary>Example code:</summary>

```rust
/// Decode a compact-encoded block header from the raw byte representation,
/// also verifying the proof-of-work target against the declared difficulty.
///
/// # Errors
///
/// Returns `TooShort` when the slice holds fewer than 80 bytes, or
/// `BadTarget` when the header fails the proof-of-work threshold.
fn decode_header(
  raw: &[u8],
) -> Result<Header, DecodeError> {
  // We validate length before field access to prevent out-of-bounds
  // reads when the slice comes from malformed or adversarial input.
  if raw.len() < 80 {
    return Err(DecodeError::TooShort);
  }
  // ...
}
```

```rust
// Bad: restates the signature, wall of text.

/// This function is called decode_header.
/// It takes a byte slice called raw and
/// returns a Result containing either a
/// Header or a DecodeError. The raw
/// parameter is the bytes to decode. If
/// decoding succeeds it returns Ok with
/// the header inside.
fn decode_header(
  raw: &[u8],
) -> Result<Header, DecodeError> {
  // ...
}
```

</details>

## Development Guidelines

### User Interfaces

- Build for the least capable device in the support matrix first; if the interface works on a constrained terminal over a slow link, it will be comfortable everywhere else
- Separate data acquisition from presentation; a display component receives its data through arguments, never fetches it, which enables testing without I/O and reuse in different contexts
- Never display raw hashes or identifiers to end users; truncate, label, or format them because long hexadecimal strings are hostile to humans

#### Terminal User Interfaces

A terminal interface runs inside constraints that graphical interfaces do not face: fixed-width cells, limited colour palettes, and no mouse by default. Designing within these constraints from the start avoids costly retrofitting.

- Separate the application into three phases: input handling, state update, and rendering; mixing these leads to flickering and inconsistent state
- Handle terminal resize events and reflow the layout; never assume a fixed terminal width or height
- Restore terminal state on exit, including on panic; a guard type or panic hook that resets raw mode and the alternate screen prevents leaving the terminal unusable after a crash
- Use colours to convey meaning, not decoration; always provide a monochrome fallback because some terminals and screen readers do not support colour
- Buffer output with double-buffering (render to an off-screen buffer, then flush the difference) to eliminate flicker during redraws
- Handle `SIGINT` and `SIGTERM` for graceful shutdown; the handler should set a flag that the event loop checks, not call `std::process::exit` directly, so destructors run and resources are released
- Provide discoverable keyboard shortcuts; a persistent status line or a `?` binding that shows available keys reduces the learning curve
- Rate-limit rendering to approximately 30 frames per second; faster redraws waste CPU without visible benefit on a text grid

> [!WARNING]
> Failing to restore terminal state on panic leaves the terminal in raw mode with no echo and no line buffering. The user must reset the terminal manually, which is a poor experience.

<details>

<summary>Example code:</summary>

```rust
use std::io;

// Guard restores terminal state on drop regardless of how the caller exits.
struct TerminalGuard;

impl TerminalGuard {
  fn new() -> io::Result<Self> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
      io::stdout(),
      crossterm::terminal::EnterAlternateScreen,
    )?;
    Ok(Self)
  }
}

impl Drop for TerminalGuard {
  fn drop(&mut self) {
    let _ =
      crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
      io::stdout(),
      crossterm::terminal::LeaveAlternateScreen,
    );
  }
}

fn main() -> io::Result<()> {
  let _guard = TerminalGuard::new()?;
  run_event_loop()
}
```

</details>

### Input Validation

> [!CAUTION]
> Assume every value that crosses a trust boundary is adversarial. Trust boundaries include: HTTP request bodies, query parameters, headers, URL path segments, WebSocket frames, file contents, and environment variables.

- **Validate early, fail loudly.** Check shape, type, length, and range before the value reaches domain logic; return a clear error that tells the caller exactly what is wrong
- **Encode validation in types.** A newtype that can only be constructed through a validating constructor carries its proof of validity with it; downstream code never needs to re-check
- **Limit input size.** Set maximum lengths for strings, maximum sizes for payloads, and maximum element counts for collections; unbounded input is an invitation for resource exhaustion
- **Do not trust client-side validation.** Client-side checks improve the user experience but provide no security guarantee; the server must re-validate independently
- **Sanitise output.** When rendering user-supplied values in any output format, encode or escape them to prevent injection

<details>

<summary>Example code:</summary>

```rust
use thiserror::Error;

// Good: validated newtype; proof of validity is carried by the type itself.
struct Username(String);

#[derive(Debug, Error)]
#[error(
  "username must be 1 to 32 ASCII \
   alphanumeric characters"
)]
struct InvalidUsername;

impl Username {
  fn new(
    raw: &str,
  ) -> Result<Self, InvalidUsername> {
    if raw.is_empty()
      || raw.len() > 32
      || !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric())
    {
      return Err(InvalidUsername);
    }
    Ok(Self(raw.to_owned()))
  }

  fn as_str(&self) -> &str {
    &self.0
  }
}

// Downstream code accepts Username, not a raw string; no need to re-validate.
fn create_account(
  name: &Username,
) -> Result<Account, AccountError> {
  // The type guarantees the name is valid; no further checks are needed.
}
```

</details>

### Security

- **Never log secrets.** Passwords, private keys, API tokens, and session identifiers must never appear in log output, debug strings, or error messages
- **Implement `Debug` to redact sensitive fields.** A custom `Debug` that prints a placeholder prevents accidental exposure through `{:?}` formatting in logs or panics
- **Use constant-time comparison for secrets.** Timing side-channels in byte-by-byte comparison leak information about secret values; use a constant-time equality function from a vetted cryptographic library
- **Zero sensitive memory after use.** Stack and heap buffers that held secrets should be zeroised before deallocation to reduce the window of exposure; the `zeroize` crate provides a `Zeroize` trait and a `ZeroizeOnDrop` derive for this purpose
- **Prefer explicit failure over silent defaults.** A default value for a missing secret silently degrades to an insecure state; failing explicitly is always safer than falling back to a placeholder

> [!NOTE]
> Audit dependencies regularly. A single compromised or unmaintained transitive dependency can undermine all other precautions in the codebase.

<details>

<summary>Example code:</summary>

```rust
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Zeroize, ZeroizeOnDrop)]
struct SessionToken(Vec<u8>);

// Custom Debug prevents the token from appearing in logs or panic output.
impl std::fmt::Debug for SessionToken {
  fn fmt(
    &self,
    f: &mut std::fmt::Formatter<'_>,
  ) -> std::fmt::Result {
    write!(f, "SessionToken(..)")
  }
}

// Constant-time comparison prevents timing attacks against secret material.
fn verify_token(
  provided: &SessionToken,
  expected: &SessionToken,
) -> bool {
  use subtle::ConstantTimeEq;
  provided.0.ct_eq(&expected.0).into()
}
```

</details>
