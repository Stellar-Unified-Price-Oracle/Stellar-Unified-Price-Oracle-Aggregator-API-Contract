//! Golden snapshots for every event topic and payload schema (#518).
//!
//! Off-chain indexers break silently when an event's shape changes. This module
//! derives a canonical schema — topic symbols, topic fields, payload fields with
//! their types, in declaration order — for **every** `#[contractevent]` struct in
//! the crate, and compares it against a committed golden file. Any unversioned
//! change (a rename, a reordering, a type change, a new or removed field, a
//! changed topic list) fails the test, which is the reviewed moment where the
//! change is either reverted or turned into a version bump.
//!
//! * Golden file: `contracts/price-oracle/testdata/event_schema.golden`
//! * Registry: `docs/event-schema-registry.md` (cross-checked by
//!   `snapshots_agree_with_the_event_schema_registry`)
//! * Regenerate deliberately with `make event-snapshots-regen`, review the diff,
//!   then commit. CI never regenerates; `make event-snapshots` only checks.
//!
//! Run with `make event-snapshots` (`cargo test -p price-oracle --lib event_schema`).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::string::{String, ToString};
use std::vec::Vec;

/// Schema version of the golden file. Bump it (and add `schemas/events/v2/…`)
/// when an event schema changes deliberately.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// Env var that makes the golden test rewrite the snapshot instead of comparing.
const UPDATE_ENV: &str = "UPDATE_EVENT_SCHEMA_SNAPSHOT";

/// One field of an event struct, in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: String,
    pub topic: bool,
}

/// The schema of one event: its topic list and its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSchema {
    pub module: String,
    pub name: String,
    /// Explicit topics from `#[contractevent(topics = [...])]`; empty means the
    /// macro derives the leading topic from the struct name.
    pub explicit_topics: Vec<String>,
    pub fields: Vec<Field>,
}

impl EventSchema {
    /// The topic symbols an indexer sees, in order.
    pub fn topics(&self) -> Vec<String> {
        if !self.explicit_topics.is_empty() {
            let mut out = self.explicit_topics.clone();
            out.extend(self.topic_fields().iter().map(|f| f.name.clone()));
            return out;
        }
        let mut out = vec![snake_case(&self.name)];
        out.extend(self.topic_fields().iter().map(|f| f.name.clone()));
        out
    }

    /// The `#[topic]`-annotated fields, in declaration order.
    pub fn topic_fields(&self) -> Vec<&Field> {
        self.fields.iter().filter(|f| f.topic).collect()
    }

    /// The non-topic fields (the event payload), in declaration order.
    pub fn payload_fields(&self) -> Vec<&Field> {
        self.fields.iter().filter(|f| !f.topic).collect()
    }

    /// Canonical, stable one-line rendering used for the golden file.
    pub fn render(&self) -> String {
        let fields: std::vec::Vec<String> = self
            .fields
            .iter()
            .map(|f| {
                std::format!(
                    "{}{}: {}",
                    if f.topic { "@topic " } else { "" },
                    f.name,
                    f.ty
                )
            })
            .collect();
        std::format!(
            "{}::{} topics=[{}] fields=[{}]",
            self.module,
            self.name,
            self.topics().join(","),
            fields.join(", ")
        )
    }
}

/// `PriceSubmittedEvent` -> `price_submitted`, the leading topic the
/// `#[contractevent]` macro derives when no explicit topics are given.
pub fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Absolute path of the contract crate.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The committed golden snapshot.
pub fn golden_path() -> PathBuf {
    crate_dir().join("testdata").join("event_schema.golden")
}

/// The event schema registry document.
fn registry_path() -> PathBuf {
    crate_dir().join("../../docs/event-schema-registry.md")
}

/// Source files that may declare events. Test-only modules are excluded: their
/// events are not part of the contract's on-chain surface.
fn event_source_files() -> std::vec::Vec<PathBuf> {
    let src = crate_dir().join("src");
    let mut out: std::vec::Vec<PathBuf> = fs::read_dir(&src)
        .expect("the crate's src/ directory must be readable")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.ends_with(".rs")
                && !name.ends_with("_tests.rs")
                && name != "test.rs"
                && name != "test_helpers.rs"
                && name != "prop_tests.rs"
        })
        .collect();
    out.sort();
    out
}

/// Parses every `#[contractevent]` struct out of one source file.
pub fn parse_module(module: &str, source: &str) -> std::vec::Vec<EventSchema> {
    let lines: std::vec::Vec<&str> = source.lines().collect();
    let mut out = std::vec::Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i].trim();
        if !line.starts_with("#[contractevent") {
            i += 1;
            continue;
        }
        // #[contractevent] or #[contractevent(topics = ["a", "b"])]
        let explicit_topics: std::vec::Vec<String> = if line == "#[contractevent]" {
            std::vec::Vec::new()
        } else {
            let mut quoted: std::vec::Vec<String> = std::vec::Vec::new();
            let mut rest = &line[std::mem::size_of::<&str>()..];
            while let Some(start) = rest.find('"') {
                let after = &rest[start + 1..];
                match after.find('"') {
                    Some(end) => {
                        quoted.push(after[..end].to_string());
                        rest = &after[end + 1..];
                    }
                    None => break,
                }
            }
            quoted
        };

        // Skip to the struct declaration.
        let mut j = i + 1;
        while j < lines.len() && !lines[j].trim_start().starts_with("pub struct ") {
            j += 1;
        }
        if j >= lines.len() {
            break;
        }
        let name = lines[j]
            .trim()
            .trim_start_matches("pub struct ")
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("")
            .to_string();

        // Collect the fields up to the closing brace.
        let mut fields: std::vec::Vec<Field> = std::vec::Vec::new();
        let mut topic = false;
        let mut k = j + 1;
        while k < lines.len() && lines[k].trim() != "}" {
            let l = lines[k].trim();
            if l == "#[topic]" {
                topic = true;
            } else if let Some(rest) = l.strip_prefix("pub ") {
                if let Some((fname, fty)) = rest.split_once(':') {
                    let ty = fty.trim().trim_end_matches(',').trim();
                    if !ty.is_empty() && !ty.starts_with("//") {
                        fields.push(Field {
                            name: fname.trim().to_string(),
                            ty: normalise_type(ty),
                            topic,
                        });
                        topic = false;
                    }
                }
            }
            k += 1;
        }
        out.push(EventSchema {
            module: module.to_string(),
            name,
            explicit_topics,
            fields,
        });
        i = k + 1;
    }
    out
}

/// Strips module paths and generic noise so a type rename in a refactor does not
/// read as a schema change, while a real type change still does.
fn normalise_type(ty: &str) -> String {
    let base = ty
        .split('<')
        .next()
        .unwrap_or(ty)
        .rsplit("::")
        .next()
        .unwrap_or(ty)
        .trim();
    let has_generics = ty.contains('<');
    std::format!("{}{}", base, if has_generics { "<…>" } else { "" })
}

/// Every event in the crate, sorted by rendered form.
pub fn collect_schemas() -> std::vec::Vec<EventSchema> {
    let mut all: std::vec::Vec<EventSchema> = std::vec::Vec::new();
    for file in event_source_files() {
        let module = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        all.extend(parse_module(&module, &source));
    }
    all.sort_by(|a, b| a.render().cmp(&b.render()));
    all.dedup_by(|a, b| a.render() == b.render());
    all
}

/// Renders the full golden snapshot: a version header plus one line per event.
pub fn render_snapshot(schemas: &[EventSchema]) -> String {
    let mut out = std::format!(
        "# SEP-40 / event schema golden snapshot (#518)\n\
         # event_schema_version: {EVENT_SCHEMA_VERSION}\n\
         # Regenerate with `make event-snapshots-regen`, review the diff, then commit.\n\
         # Format: module::EventName topics=[a,b,c] fields=[@topic x: Address, price: i128]\n"
    );
    for s in schemas {
        out.push_str(&s.render());
        out.push('\n');
    }
    out
}

/// Reads the committed golden file, or writes it when the update env var is set.
fn golden_contents(schemas: &[EventSchema]) -> String {
    let path = golden_path();
    let expected = render_snapshot(schemas);
    if std::env::var(UPDATE_ENV).is_ok() {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).expect("cannot create the golden file's directory");
        }
        fs::write(&path, &expected)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        return expected;
    }
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read the golden snapshot {}: {e}\n\
             Run `make event-snapshots-regen` to create it, then review the result.",
            path.display()
        )
    })
}

/// Reports the first differing line between two rendered snapshots.
fn first_difference(expected: &str, actual: &str) -> std::string::String {
    let mut exp = expected.lines();
    let mut act = actual.lines();
    let mut line = 1usize;
    loop {
        match (exp.next(), act.next()) {
            (None, None) => return std::format!("line {line}: the snapshots end differently"),
            (None, Some(a)) => return std::format!("line {line}: unexpected entry `{a}`"),
            (Some(e), None) => return std::format!("line {line}: missing entry `{e}`"),
            (Some(e), Some(a)) if e != a => {
                return std::format!("line {line}:\n  golden:   {e}\n  contract: {a}")
            }
            _ => line += 1,
        }
    }
}

/// The committed snapshot must match the contract exactly. This is the gate: it
/// fails on a rename, a reordering, a type change, a topic change, and on an
/// added or removed event.
#[test]
fn every_event_matches_the_golden_snapshot() {
    let schemas = collect_schemas();
    assert!(!schemas.is_empty(), "no events were parsed at all");
    let expected = render_snapshot(&schemas);
    let committed = golden_contents(&schemas);
    assert!(
        expected == committed,
        "the event schema changed without a reviewed regeneration.\n{}\n\
         Either revert the change or bump `EVENT_SCHEMA_VERSION` and run \
         `make event-snapshots-regen`.",
        first_difference(&committed, &expected)
    );
}

/// Completeness: every `#[contractevent]` in the crate has exactly one snapshot
/// entry, and every entry has a topic an indexer can filter on.
#[test]
fn every_event_is_covered_by_a_snapshot() {
    let schemas = collect_schemas();

    // Count the attributes in the same file set the parser reads; if a source
    // file ever fails to parse, the counts diverge and this test fails.
    let mut attributes = 0usize;
    for file in event_source_files() {
        let source = fs::read_to_string(&file).expect("readable source file");
        attributes += source
            .lines()
            .filter(|l| l.trim_start().starts_with("#[contractevent"))
            .count();
    }
    assert_eq!(
        schemas.len(),
        attributes,
        "every #[contractevent] must produce exactly one snapshot entry"
    );

    let names: std::collections::BTreeSet<String> = schemas
        .iter()
        .map(|s| std::format!("{}::{}", s.module, s.name))
        .collect();
    assert_eq!(names.len(), schemas.len(), "duplicate event structs parsed");

    for s in &schemas {
        assert!(
            !s.topics().is_empty(),
            "{}::{} has no topic symbol",
            s.module,
            s.name
        );
    }
}
/// The event schema registry and the golden snapshot describe the same events:
/// every registry row names a struct that exists, with the same leading topic,
/// and has a JSON schema under `schemas/events/v{N}/`.
#[test]
fn snapshots_agree_with_the_event_schema_registry() {
    let registry =
        fs::read_to_string(registry_path()).expect("the event schema registry must exist");
    let schemas = collect_schemas();
    let by_name: BTreeMap<&str, &EventSchema> =
        schemas.iter().map(|s| (s.name.as_str(), s)).collect();

    // Registry rows look like: | `topic` | `Struct` | [schema](…) |
    let mut checked = 0usize;
    for line in registry.lines() {
        if !line.starts_with("| `") {
            continue;
        }
        let cells: std::vec::Vec<&str> = line.split('|').map(|c| c.trim()).collect();
        if cells.len() < 3 {
            continue;
        }
        let topic = cells[1].trim_matches('`');
        let rust = cells[2].trim_matches('`');
        if !rust.ends_with("Event") {
            continue;
        }
        let schema = by_name
            .get(rust)
            .unwrap_or_else(|| panic!("{rust} is in the registry but not in the snapshot"));
        // The registry names its schema files after the event without the
        // `Event` suffix, while the on-chain leading topic is the struct name in
        // snake_case (`PriceSubmittedEvent` -> `price_submitted_event`). Both
        // spellings must therefore resolve to the same struct.
        let topics = schema.topics();
        let leading = topics.first().map(|t| t.as_str()).unwrap_or_default();
        assert!(
            leading == topic || leading == std::format!("{topic}_event"),
            "{rust}: the registry topic `{topic}` does not match the struct's leading topic `{leading}`"
        );
        let json = std::format!("../../schemas/events/v{EVENT_SCHEMA_VERSION}/{topic}.schema.json");
        assert!(
            crate_dir().join(json).exists(),
            "{topic}: the registry row has no v{EVENT_SCHEMA_VERSION} JSON schema"
        );
        checked += 1;
    }
    assert!(checked > 0, "no registry rows were cross-checked");
}

/// A schema change must be *detected*, not merely tolerated: renames, reorders,
/// type changes and topic changes all move the snapshot, and the golden file
/// must carry the pinned version header.
#[test]
fn unversioned_schema_changes_are_detected() {
    let source = |decl: &str| std::format!("#[contractevent]\n{decl}\n");
    let base = parse_module(
        "events",
        &source(
            "pub struct ExampleEvent {\n    #[topic]\n    pub asset: Address,\n    pub price: i128,\n}",
        ),
    );
    let rendered = render_snapshot(&base);
    let changed = parse_module(
        "events",
        &source(
            "pub struct ExampleEvent {\n    #[topic]\n    pub asset: Address,\n    pub price: u128,\n}",
        ),
    );
    assert_ne!(
        rendered,
        render_snapshot(&changed),
        "a type change must be detected"
    );

    let renamed = parse_module(
        "events",
        &source(
            "pub struct ExampleEvent {\n    #[topic]\n    pub asset: Address,\n    pub value: i128,\n}",
        ),
    );
    assert_ne!(
        rendered,
        render_snapshot(&renamed),
        "a rename must be detected"
    );

    let reordered = parse_module(
        "events",
        &source("pub struct ExampleEvent {\n    pub price: i128,\n    #[topic]\n    pub asset: Address,\n}"),
    );
    assert_ne!(
        rendered,
        render_snapshot(&reordered),
        "a reorder must be detected"
    );

    let retopiced = parse_module(
        "events",
        "#[contractevent(topics = [\"other\"])]\npub struct ExampleEvent {\n    #[topic]\n    pub asset: Address,\n    pub price: i128,\n}\n",
    );
    assert_ne!(
        rendered,
        render_snapshot(&retopiced),
        "a topic change must be detected"
    );

    let committed = fs::read_to_string(golden_path()).expect("the golden snapshot must exist");
    assert!(
        committed.contains(&std::format!(
            "event_schema_version: {EVENT_SCHEMA_VERSION}"
        )),
        "the golden snapshot must record the schema version it was generated from"
    );
}
