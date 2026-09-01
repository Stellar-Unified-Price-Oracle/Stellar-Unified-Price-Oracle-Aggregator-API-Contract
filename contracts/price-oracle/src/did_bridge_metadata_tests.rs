#![cfg(test)]

use soroban_sdk::{Address, Env, String};

use crate::test_helpers::*;
use crate::{
    BridgeOracleConfig, BridgedPrice, DidDocument, DidVerification, EcosystemMetadata,
    FeedMetadata, SourceDidLink,
};

mod did_tests {
    use super::*;

    #[test]
    fn test_register_and_verify_did() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let client = create_contract(&e);

        client.initialize(
            &admin,
            &1u32,
            &50u32,
            &18u32,
            &String::from_str(&e, "DID Test"),
        );

        let did = Address::generate(&e);
        let document = String::from_str(&e, r#"{"id":"did:stellar:test"}"#);

        client.did_register(&did, &document);
        assert!(client.did_verify(&did));
        let doc = client.did_get_document(&did);
        assert!(doc.is_some());
        assert_eq!(doc.unwrap(), document);
    }

    #[test]
    fn test_link_source_did() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let client = create_contract(&e);

        client.initialize(
            &admin,
            &1u32,
            &50u32,
            &18u32,
            &String::from_str(&e, "DID Link"),
        );

        let source = Address::generate(&e);
        let did = Address::generate(&e);

        client.did_register(&did, &String::from_str(&e, "{}"));
        client.add_source(&source, &String::from_str(&e, "Source A"));
        client.did_link_source(&source, &did, true);

        let link = client.did_get_source_link(&source);
        assert!(link.is_some());
        assert!(link.unwrap().verified);
    }
}

mod bridge_oracle_tests {
    use super::*;

    #[test]
    fn test_register_bridge_oracle_and_submit_price() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let client = create_contract(&e);

        client.initialize(
            &admin,
            &1u32,
            &50u32,
            &18u32,
            &String::from_str(&e, "Bridge"),
        );

        let source_asset = Address::generate(&e);
        let target_asset = Address::generate(&e);
        let oracle_contract = Address::generate(&e);

        let config = BridgeOracleConfig {
            source_asset: source_asset.clone(),
            target_asset: target_asset.clone(),
            oracle_contract: oracle_contract.clone(),
            decimals: 18,
            enabled: true,
        };

        client.bridge_register_oracle(&config);
        let fetched = client.bridge_get_oracle(&source_asset, &target_asset);
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().oracle_contract, oracle_contract);
    }

    #[test]
    fn test_bridged_price_none_when_unregistered() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let client = create_contract(&e);

        client.initialize(
            &admin,
            &1u32,
            &50u32,
            &18u32,
            &String::from_str(&e, "Bridge"),
        );

        let a = Address::generate(&e);
        let b = Address::generate(&e);
        let price = client.bridge_get_price(&a, &b);
        assert!(price.is_none());
    }
}

mod ecosystem_metadata_tests {
    use super::*;

    #[test]
    fn test_register_and_list_feeds() {
        let e = Env::default();
        let admin = Address::generate(&e);
        let client = create_contract(&e);

        client.initialize(&admin, &1u32, &50u32, &18u32, &String::from_str(&e, "Meta"));

        let asset = Address::generate(&e);
        let metadata = EcosystemMetadata {
            contract_id: client.address(),
            name: String::from_str(&e, "Test Oracle"),
            description: String::from_str(&e, "Test description"),
            version: String::from_str(&e, "1.0.0"),
            feeds: Vec::new(&e),
            registered_at: 1234567890,
        };

        client.metadata_register(metadata.clone());
        let fetched = client.metadata_get();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().name, String::from_str(&e, "Test Oracle"));

        let feed = FeedMetadata {
            asset: asset.clone(),
            symbol: String::from_str(&e, "TEST"),
            description: String::from_str(&e, "Test feed"),
            decimals: 18,
            updated_at: 1234567890,
        };

        client.metadata_register_feed(&feed);
        let feeds = client.metadata_list_feeds();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds.get_unchecked(0).symbol, String::from_str(&e, "TEST"));
    }
}

mod event_streaming_tests {
    use super::*;
    use crate::event_streaming::{
        OracleEventEnvelope, CLICKHOUSE_MIGRATION_SQL, POSTGRES_MIGRATION_SQL,
    };

    #[test]
    fn test_event_envelope_creation() {
        let envelope = OracleEventEnvelope::new(
            12345,
            67890,
            String::from_str(&Env::default(), "contract_id"),
            String::from_str(&Env::default(), "price_updated"),
            serde_json::json!({"asset": "G..."}),
        );
        assert_eq!(envelope.ledger, 12345);
        assert_eq!(envelope.topic, "price_updated");
    }

    #[test]
    fn test_postgres_migration_sql_is_present() {
        assert!(POSTGRES_MIGRATION_SQL.contains("CREATE TABLE"));
        assert!(POSTGRES_MIGRATION_SQL.contains("oracle_events"));
    }

    #[test]
    fn test_clickhouse_migration_sql_is_present() {
        assert!(CLICKHOUSE_MIGRATION_SQL.contains("CREATE TABLE"));
        assert!(CLICKHOUSE_MIGRATION_SQL.contains("MergeTree"));
    }
}
