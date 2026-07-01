## Plan to implement configurable maximum oracle sources

### Information Gathered
- Current oracle source registry is stored at `DataKey::OracleSources` as `OracleSources { sources: Vec<Address>, metadata: Map<Address, String> }`.
- `sources::add_source` currently checks for duplicate sources and then appends to `oracle_sources.sources` and writes back.
- Admin configuration functions live in `contracts/price-oracle/src/admin.rs` and are exposed via `contracts/price-oracle/src/lib.rs`.
- Error codes live in `contracts/price-oracle/src/errors.rs`.
- `DataKey` variants are in `contracts/price-oracle/src/types.rs`.
- Tests are in `contracts/price-oracle/src/test.rs` and other module-specific files.

### Edit Plan (file-by-file)
1. **types.rs**
   - Add new `DataKey` variant: `MaxSources` (global config).
   - (No change to `OracleSources` struct needed.)

2. **errors.rs**
   - Add new error variant: `MaxSourcesReached` with the next available discriminant.

3. **admin.rs**
   - Add constant `DEFAULT_MAX_SOURCES: u32 = 50`.
   - Implement:
     - `pub fn set_max_sources(env: &Env, count: u32)` with `admin.require_auth()`.
     - `pub fn get_max_sources(env: &Env) -> u32` with default fallback to 50.

4. **lib.rs**
   - Expose admin functions:
     - `pub fn set_max_sources(env: Env, count: u32)`
     - `pub fn get_max_sources(env: Env) -> u32`

5. **sources.rs**
   - In `add_source`, before writing the new source:
     - Load current `oracle_sources` and current `max_sources`.
     - If `oracle_sources.sources.len() >= max_sources`, reject with `panic_with_error!(env, ErrorCode::MaxSourcesReached)`.

6. **test.rs**
   - Add tests verifying:
     - Default `get_max_sources()` is 50.
     - Limit enforcement: after setting max sources to N, adding N sources succeeds, adding (N+1)th source panics with `Error(Contract, <discriminant>)`.

7. **Run test suite**
   - `cargo test` for `contracts/price-oracle` or workspace root.

### Dependent Files to be edited
- `contracts/price-oracle/src/types.rs`
- `contracts/price-oracle/src/errors.rs`
- `contracts/price-oracle/src/admin.rs`
- `contracts/price-oracle/src/lib.rs`
- `contracts/price-oracle/src/sources.rs`
- `contracts/price-oracle/src/test.rs`

### Followup steps after editing
- Run `cargo test` to ensure all existing tests pass.

<ask_followup_question>
Proceed with implementing the plan exactly as described (default max sources=50; enforce in add_source; add ErrorCode::MaxSourcesReached; add/route admin getters+setters; add unit tests in test.rs).
</ask_followup_question>

