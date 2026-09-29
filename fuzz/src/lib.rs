// Empty lib target so the workspace manifest resolves. The fuzz crate's
// Cargo.toml declares an explicit `[lib]` table (required so it isn't built
// as a cdylib), which makes cargo expect a `src/lib.rs` regardless of the
// standalone `[[bin]]` fuzz targets declared alongside it.
