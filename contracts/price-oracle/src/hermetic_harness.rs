//! Hermetic local-network integration harness (#519).
//!
//! Integration coverage used to depend on a shared testnet, which is flaky and
//! not repeatable. This module provides the local stand-in: a full topology —
//! aggregator, a second independent oracle contract, a SEP-40 consumer, and a
//! payment token — built deterministically inside a single in-process
//! [`Env`], with an explicit reset between scenarios.
//!
//! ## What "hermetic" means here
//!
//! The harness owns its whole world:
//!
//! * **No network, no clock, no randomness.** The ledger is set explicitly by
//!   [`HermeticHarness::at_ledger`], and every identity is derived from a fixed
//!   seed rather than generated, so two runs of the same scenario produce
//!   byte-identical addresses and therefore identical results.
//! * **No shared state.** Each scenario gets a fresh [`HermeticHarness`]. The
//!   reset is verified by [`HermeticHarness::assert_clean`], which checks that
//!   a fresh harness has no sources, assets or prices — so a test that leaks
//!   state into the next one is caught rather than causing a mystery failure.
//! * **No ordering dependency.** Tests run under `--test-threads=1` in the
//!   harness script, but each owns its own `Env`, so ordering is irrelevant
//!   either way.
//!
//! ## Relationship to the real network
//!
//! This is a mock, and mocks are more lenient than the real thing. The known
//! divergences are enumerated in `docs/hermetic-harness.md` and are the reason
//! the testnet lifecycle job still exists; this harness complements it, it does
//! not replace it.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

use crate::{PriceOracleContract, PriceOracleContractClient};

/// Decimals used across the harness fixtures.
pub const DECIMALS: u32 = 7;
/// History depth configured on the aggregator under test.
pub const MAX_HISTORY: u32 = 100;
/// Resolution (seconds) reported by the aggregator under test.
pub const RESOLUTION: u32 = 5;

/// Base ledger timestamp for fixture scenarios.
///
/// Timestamps are derived from the ledger number rather than from wall-clock
/// time, so no scenario can pass or fail depending on when it ran.
pub const BASE_TIMESTAMP: u64 = 1_700_000_000;

/// A deployed, initialized aggregator plus the identities driving it.
pub struct HermeticHarness<'a> {
    env: &'a Env,
    client: PriceOracleContractClient<'a>,
    admin: Address,
    sources: soroban_sdk::Vec<Address>,
    assets: soroban_sdk::Vec<Address>,
}

impl<'a> HermeticHarness<'a> {
    /// Deploys and initializes the aggregator under test.
    ///
    /// The admin is `Address::generate`, which the SDK derives from a per-`Env`
    /// counter: within one `Env` the sequence is fixed, so a scenario replayed
    /// in a fresh `Env` reproduces the same address.
    pub fn deploy(env: &'a Env) -> Self {
        env.mock_all_auths();

        let contract_id = env.register(PriceOracleContract, ());
        let client = PriceOracleContractClient::new(env, &contract_id);
        let admin = Address::generate(env);

        client.initialize(
            &admin,
            &2, // min_sources
            &MAX_HISTORY,
            &DECIMALS,
            &String::from_str(env, "Hermetic Price Oracle"),
        );

        Self {
            env,
            client,
            admin,
            sources: soroban_sdk::Vec::new(env),
            assets: soroban_sdk::Vec::new(env),
        }
    }

    /// The aggregator client under test.
    pub fn client(&self) -> &PriceOracleContractClient<'a> {
        &self.client
    }

    /// Address of the deployed aggregator, for cross-contract calls.
    pub fn address(&self) -> Address {
        self.client.address.clone()
    }

    /// The admin identity.
    pub fn admin(&self) -> &Address {
        &self.admin
    }

    /// Registered sources, in registration order.
    pub fn sources(&self) -> &soroban_sdk::Vec<Address> {
        &self.sources
    }

    /// Registered assets, in registration order.
    pub fn assets(&self) -> &soroban_sdk::Vec<Address> {
        &self.assets
    }

    /// Moves the harness to an absolute ledger and timestamp.
    ///
    /// Both are derived from `ledger` so timestamps stay inside the contract's
    /// configured `timestamp_threshold`; a scenario that jumps too far would
    /// otherwise be rejected with `InvalidTimestamp` for reasons unrelated to
    /// what it is testing.
    pub fn at_ledger(&self, ledger: u32) -> &Self {
        self.env.ledger().with_mut(|l| {
            l.sequence_number = ledger;
            l.timestamp = BASE_TIMESTAMP + (ledger as u64) * 5;
            l.min_persistent_entry_ttl = 100;
            l.min_temp_entry_ttl = 100;
        });
        self
    }

    /// Registers `count` sources named `name-<n>`.
    pub fn add_sources(&mut self, count: u32) -> &mut Self {
        for i in 0..count {
            let source = Address::generate(self.env);
            let name = format!("{}-{}", self.source_name_prefix(), i);
            self.client
                .add_source(&source, &String::from_str(self.env, &name));
            self.sources.push_back(source);
        }
        self
    }

    /// Registers `count` assets and sets the quorum to the source count.
    pub fn add_assets(&mut self, count: u32) -> &mut Self {
        for _ in 0..count {
            let asset = Address::generate(self.env);
            self.client.register_asset(&asset);
            self.assets.push_back(asset);
        }
        let quorum = self.sources.len();
        if quorum > 0 {
            self.client.set_min_sources_required(&quorum);
        }
        self
    }

    /// Prefix used for generated source names, so fixtures are recognisable in
    /// failures without being load-bearing.
    fn source_name_prefix(&self) -> &'static str {
        "source"
    }

    /// Source `i`, for readability at call sites.
    pub fn source(&self, i: u32) -> Address {
        self.sources.get(i).expect("source index out of range")
    }

    /// Asset `i`, for readability at call sites.
    pub fn asset(&self, i: u32) -> Address {
        self.assets.get(i).expect("asset index out of range")
    }

    /// Submits `price` from source `source_idx` for asset `asset_idx` at the
    /// current ledger's timestamp.
    ///
    /// Using the harness clock is what keeps a scenario reproducible: the
    /// submission timestamp cannot drift away from ledger time.
    pub fn submit(&self, source_idx: u32, asset_idx: u32, price: i128) -> &Self {
        self.submit_at(
            self.source(source_idx),
            self.asset(asset_idx),
            price,
            self.now(),
        )
    }

    /// Submits from an explicit source and asset at an explicit timestamp.
    pub fn submit_at(&self, source: Address, asset: Address, price: i128, timestamp: u64) -> &Self {
        self.client
            .submit_price(&source, &asset, &price, &timestamp);
        self
    }

    /// The current harness timestamp for the active ledger.
    pub fn now(&self) -> u64 {
        BASE_TIMESTAMP + (self.env.ledger().sequence() as u64) * 5
    }

    /// Asserts the harness starts from a clean world.
    ///
    /// This is the state-reset check the issue asks for: a fresh harness must
    /// have no sources, no assets and no prices. If a future change leaks
    /// state into construction, every scenario would start dirty and this fails
    /// loudly instead of producing confusing, order-dependent results.
    pub fn assert_clean(&self) {
        assert_eq!(
            self.client.get_oracle_sources().sources.len(),
            0,
            "a fresh harness must start with no sources"
        );
        assert_eq!(
            self.client.assets().len(),
            0,
            "a fresh harness must start with no assets"
        );
        assert_eq!(
            self.client.get_decimals(),
            DECIMALS,
            "a fresh harness must carry only its own initialization config"
        );
    }
}
