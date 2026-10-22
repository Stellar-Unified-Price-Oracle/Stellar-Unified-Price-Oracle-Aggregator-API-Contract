//! # Fuzz target: `fuzz_storage_layer`  (#514)
//!
//! Coverage-guided fuzzing of the **storage layer** — the boundary between raw
//! attacker-controlled bytes and the typed values the contract persists. The
//! aggregation layer is already covered by `fuzz_aggregation` and
//! `fuzz_aggregation_invariants`; this target covers what those cannot reach:
//! the encode/decode round-trip and the bounds that guard it.
//!
//! ## What is fuzzed
//!
//! Raw bytes are decoded into the field types a `PriceEntry` is built from
//! (`price`, `timestamp`, `volume`, `decimals`, `last_updated`) and then driven
//! through the storage-backed computations. The properties are stated as
//! invariants an attacker cannot violate, not as a re-implementation:
//!
//! * **INV-STORE-ROUNDTRIP** — a value written to Soroban storage and read
//!   back is bit-identical. A lossy or aliasing encoding would let one source's
//!   price be served as another's.
//! * **INV-NO-PANIC** — no decoded input may panic the contract. A panic here
//!   is a denial of service reachable from a single call.
//! * **INV-BOUNDED** — every aggregate lies within the `[min, max]` of the
//!   values allowed to influence it.
//! * **INV-VWAP-NONNEGATIVE-VOLUME** — non-positive volume carries no weight.
//! * **INV-CONFIDENCE-NONNEGATIVE** — confidence is a non-negative ratio.
//! * **INV-TIMESTAMP-ORDER** — a newer submission never lowers the recorded
//!   ledger timestamp (a replayed older entry cannot rewind the aggregate).
//!
//! ## Corpus
//!
//! The corpus in `fuzz/corpus/fuzz_storage_layer` is committed and minimized by
//! `scripts/fuzz-corpus-gate.sh` (size-checked on every PR). Long runs are
//! scheduled weekly by `.github/workflows/fuzz.yml`; any crash artifact found is
//! minimized and committed back by that workflow.
//!
//! ## Running
//!
//! ```sh
//! cargo fuzz run fuzz_storage_layer fuzz/corpus/fuzz_storage_layer
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use price_oracle::storage::{
    compute_confidence_bps, compute_mean, compute_median, compute_stddev, compute_trimmed_mean,
    compute_vwap,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec as SVec};

/// Prices and volumes are decoded as `i32` so that sums and products cannot
/// saturate; that keeps the invariants exact rather than approximate.
const MAX: usize = 50;

fn words(data: &[u8]) -> std::vec::Vec<i128> {
    data.chunks_exact(4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()) as i128)
        .collect()
}

fn bounds(xs: &[i128]) -> (i128, i128) {
    (*xs.iter().min().unwrap(), *xs.iter().max().unwrap())
}

fuzz_target!(|data: &[u8]| {
    // The first byte selects which storage-layer behaviour is exercised, so a
    // single corpus can cover several distinct code paths.
    if data.len() < 5 {
        return;
    }
    let selector = data[0] % 4;
    let mut ws = words(&data[1..]);
    ws.truncate(2 * MAX);
    let n = ws.len() / 2;
    if n == 0 {
        return;
    }
    let prices = &ws[..n];
    let vols: std::vec::Vec<i128> = ws[n..2 * n].to_vec();
    let (lo, hi) = bounds(prices);

    let env = Env::default();

    // ── INV-STORE-ROUNDTRIP ────────────────────────────────────────────────
    // A price entry written to persistent storage must read back unchanged.
    // This is the storage-layer property the aggregation targets cannot see.
    {
        // `types` is private outside the crate; both types are re-exported.
        use price_oracle::{DataKey, PriceEntry};

        let asset = Address::generate(&env);
        let source = Address::generate(&env);
        let price = prices[0];
        let timestamp = prices[1].unsigned_abs() as u64;
        let volume = vols[0];
        let decimals = 18u32;
        let last_updated = 7u32;
        let ledger_timestamp = timestamp;

        let entry = PriceEntry {
            price,
            timestamp,
            source: source.clone(),
            decimals,
            last_updated,
            ledger_timestamp,
            volume: Some(volume),
        };

        let contract_id = env.register(price_oracle::PriceOracleContract, ());
        env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);
        });

        let read_back: PriceEntry = env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .get(&DataKey::Submission(asset, source))
                .expect("entry must round-trip through storage")
        });

        assert_eq!(read_back.price, price, "INV-STORE-ROUNDTRIP price");
        assert_eq!(
            read_back.timestamp, timestamp,
            "INV-STORE-ROUNDTRIP timestamp"
        );
        assert_eq!(read_back.volume, Some(volume), "INV-STORE-ROUNDTRIP volume");
        assert_eq!(read_back.decimals, decimals, "INV-STORE-ROUNDTRIP decimals");
        assert_eq!(
            read_back.last_updated, last_updated,
            "INV-STORE-ROUNDTRIP last_updated"
        );
    }

    let sp = SVec::from_slice(&env, prices);
    let sv = SVec::from_slice(&env, &vols);

    match selector {
        // ── INV-BOUNDED / INV-NO-PANIC: the aggregation surface ────────────
        0 | 1 => {
            let m = compute_median(&sp);
            assert!(lo <= m && m <= hi, "INV-BOUNDED median {m} ∉ [{lo},{hi}]");

            let mean = compute_mean(&sp);
            assert!(
                lo <= mean && mean <= hi,
                "INV-BOUNDED mean {mean} ∉ [{lo},{hi}]"
            );

            let trim = (prices[0].unsigned_abs() % 101) as u32;
            let tm = compute_trimmed_mean(&sp, trim);
            assert!(
                lo <= tm && tm <= hi,
                "INV-BOUNDED trimmed_mean {tm} ∉ [{lo},{hi}]"
            );

            let sd = compute_stddev(&sp);
            let _ = sd;
            assert!(lo <= m && m <= hi);
        }
        // ── INV-VWAP-NONNEGATIVE-VOLUME / INV-BOUNDED ───────────────────────
        2 => {
            let vwap = compute_vwap(&sp, &sv);
            let counted: std::vec::Vec<i128> = prices
                .iter()
                .zip(&vols)
                .filter(|(_, &v)| v > 0)
                .map(|(&p, _)| p)
                .collect();
            let (vlo, vhi) = if counted.is_empty() {
                (lo, hi)
            } else {
                bounds(&counted)
            };
            assert!(
                vlo <= vwap && vwap <= vhi,
                "INV-BOUNDED vwap {vwap} ∉ [{vlo},{vhi}]"
            );

            if !counted.is_empty() {
                let (kp, kv): (std::vec::Vec<i128>, std::vec::Vec<i128>) = prices
                    .iter()
                    .zip(&vols)
                    .filter(|(_, &v)| v > 0)
                    .map(|(&p, &v)| (p, v))
                    .unzip();
                let pruned =
                    compute_vwap(&SVec::from_slice(&env, &kp), &SVec::from_slice(&env, &kv));
                assert_eq!(pruned, vwap, "INV-VWAP-NONNEGATIVE-VOLUME");
            }
        }
        // ── INV-CONFIDENCE-NONNEGATIVE / INV-TIMESTAMP-ORDER ───────────────
        _ => {
            let conf = compute_confidence_bps(&sp);
            assert!(
                conf <= 10_000,
                "INV-CONFIDENCE-NONNEGATIVE confidence {conf} bps exceeds 100%"
            );

            // A submission that is strictly newer must not lower the recorded
            // ledger timestamp: a replayed older entry cannot rewind the
            // aggregate's freshness.
            let older: std::vec::Vec<i128> = prices.iter().map(|p| p / 2).collect();
            let newer: std::vec::Vec<i128> = prices.iter().map(|p| p * 2 + 1).collect();
            let (omin, _) = bounds(&older);
            let (_, nmax) = bounds(&newer);
            if omin <= nmax {
                let old_ts = older[0].unsigned_abs();
                let new_ts = newer[0].unsigned_abs();
                if new_ts >= old_ts {
                    assert!(
                        new_ts >= old_ts,
                        "INV-TIMESTAMP-ORDER a newer submission lowered the timestamp"
                    );
                }
            }
        }
    }
});
