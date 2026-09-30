# Reproducible Builds and Dependency Pinning

**Issue:** #501 — pin every dependency to an exact version with a committed
lockfile and hashes, and produce byte-reproducible builds so the deployed WASM
can be independently rebuilt and verified.

A contract that cannot be rebuilt from source cannot be verified. Supply-chain
pinning plus reproducibility is the foundation of independent auditability.

| Claim | Enforced by |
|---|---|
| Every dependency is pinned exactly, transitively | `services/pinned_deps/pin_check.py::check_requirements`, asserted in `test_every_workspace_requirement_is_exact` and the parametrized `test_floating_requirements_are_rejected` |
| The lockfile is committed and hashed | `check_lockfile`, asserted in `test_lockfile_is_committed_and_hashed` and `test_lock_without_checksum_is_rejected` |
| Resolution drift is rejected | `check_lockfile`, asserted in `test_pin_that_disagrees_with_the_lock_is_drift` and `test_stale_lock_missing_a_direct_dependency_is_rejected` |
| Two clean builds produce identical artifacts | `scripts/reproducible-build.sh`, run in CI on every PR |
| The canonical environment is documented and containerized | `docker/Dockerfile.canonical-build`, asserted in `test_container_image_pins_the_same_toolchain_as_the_toolchain_file` |

```bash
make check-pins           # or: python -m services.pinned_deps.pin_check .
make reproducible-build   # or: ./scripts/reproducible-build.sh
```

---

## 1. What is pinned

| Input | Pinned by | Change it by |
|---|---|---|
| Direct dependencies (`soroban-sdk`, `proptest`, `ed25519-dalek`, `libfuzzer-sys`) | exact `=x.y.z` requirements in every member manifest | editing the manifest (see §5) |
| Transitive dependencies | the committed `Cargo.lock`, with a `checksum` for every registry crate | `cargo update -p <crate> --precise <version>`, committed with the manifest |
| Toolchain | `rust-toolchain.toml` (`channel = "1.91.0"`, `targets = ["wasm32v1-none"]`) | editing `rust-toolchain.toml` |
| Link flags | `contracts/price-oracle/.cargo/config.toml` | editing that file |
| Build timestamps | `SOURCE_DATE_EPOCH` = commit time, set by the build script | never set by hand |

Every requirement in the deployed workspace is now fully qualified:

```toml
soroban-sdk = { version = "=26.0.1" }
proptest     = { version = "=1.11.0", default-features = false, features = ["std"] }
```

Previously these were `"26"`, `"1.5.0"` and `"2.1.1"` — compatible ranges, not
pins. A compatible range means the artifact is a function of *when* it was
built, which is exactly the property that makes independent verification
impossible.

`pin_check` rejects `"*"`, `"1"`, `"1.5"`, `"^1.5.0"`, `"~1.5.0"` and any range
(`>=1.5, <2`), and requires a git dependency to pin an immutable `rev` rather
than a branch. Path dependencies (workspace members) are exempt: they are
pinned by the workspace itself and by the lockfile.

**Scope note.** The check covers the `[workspace] members` that produce the
deployed artifact. `sdk/rust` is a standalone client library outside the
workspace with no lockfile; it is not part of the deployed graph and is not
gated here.

## 2. The canonical build environment

| Variable | Value | Why |
|---|---|---|
| toolchain | `1.91.0` from `rust-toolchain.toml` | a different rustc can emit a different binary; the script refuses to run on a mismatch |
| `CARGO_INCREMENTAL=0` | off | incremental state can leak across builds |
| `SOURCE_DATE_EPOCH` | commit timestamp | any timestamp embedded in the artifact becomes fixed |
| `LC_ALL` / `LANG` / `TZ` | `C` / `UTC` | locale-dependent formatting and ordering must not vary |
| resolution | `--locked` | cargo consumes the committed `Cargo.lock` or fails |
| `CARGO_TERM_COLOR` | `never` | keeps captured output byte-stable |

`make build` now passes `--locked` too, so the artifact CI publishes is the
artifact the release path builds — there is no second, looser build command.

The environment is containerized in `docker/Dockerfile.canonical-build`
(`rust:1.91.0-bookworm` + the `wasm32v1-none` target + the same neutralized
variables), so an auditor can reproduce the build without matching a local
toolchain by hand:

```bash
docker build -f docker/Dockerfile.canonical-build -t price-oracle-build .
docker run --rm -v "$PWD":/src -w /src price-oracle-build
```

## 3. Proving reproducibility

`scripts/reproducible-build.sh` builds the contract **twice**, each into its own
clean `CARGO_TARGET_DIR`, in the canonical environment, and fails unless the two
artifacts are byte-identical:

```
build A sha256 == build B sha256  ->  reproducible
build A sha256 != build B sha256  ->  exit 1, both digests reported
```

It also verifies the toolchain against `rust-toolchain.toml` before doing any
work, and writes `build-artifacts/wasm-hashes.txt` (the digest, toolchain and
`SOURCE_DATE_EPOCH`) plus `build-artifacts/reproducible-build.md`.

CI runs this on every PR as the `reproducible-build` job and uploads
`build-artifacts/` as an artifact, so **the artifact hashes are published for
verification** by anyone who wants to rebuild and compare:

```bash
sha256sum target/wasm32v1-none/release/price_oracle.wasm
# must equal the digest in the run's build-artifacts/wasm-hashes.txt
```

A digest mismatch is a hard failure and must be triaged as a supply-chain
incident, not retried: something in the build stopped depending only on the
committed inputs.

## 4. What the checks reject

| Drift | How it is caught |
|---|---|
| A floating requirement reintroduced | `check_requirements` (test: `test_floating_requirements_are_rejected`) |
| Lockfile regenerated with different versions | `check_lockfile` pin-vs-lock comparison |
| A direct dependency missing from the lock | `check_lockfile` |
| A registry crate without a checksum | `check_lockfile` |
| A git dependency tracking a branch | `check_requirements` |
| A build that is not byte-stable | `scripts/reproducible-build.sh` |
| A toolchain that is not the pinned one | the build script's toolchain assertion |

## 5. Procedure for an intentional dependency update

1. `cargo update -p <crate> --precise <version>` (never a bare `cargo update`).
2. Update the exact requirement in the affected member manifest to `=x.y.z`.
3. `make check-pins` — must be clean.
4. `./scripts/reproducible-build.sh` — the artifact must still be byte-stable
   (the digest will *change*, the two builds must still agree).
5. `make build test lint` and the full CI.
6. Commit `Cargo.lock`, the manifest and the new
   `build-artifacts/wasm-hashes.txt` **in one commit**, so the pin change, the
   resolved graph and the published digest move together.
7. Note the new digest in the release record (`docs/release-provenance.md`).

If step 4 fails, the new dependency embeds something environment-dependent.
That is a blocker, not a nuisance: stop and find it before shipping.

## 6. Out of scope

Third-party reproducible-build attestation services (in-toto, SLSA provenance
signing). This document stops at "two clean builds agree, and the digest is
published".
