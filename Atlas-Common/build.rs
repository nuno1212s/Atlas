//! Rejects contradictory backend selections before they turn into a wall of
//! duplicate-definition errors.
//!
//! Each group in `Cargo.toml` picks its implementation from the *absence* of an
//! override feature, so at most one override per group may be enabled. Cargo
//! unions features across the whole dependency graph, which means two different
//! crates each picking a different backend silently produces both flags — an
//! genuinely ambiguous request that this turns into a readable error.

/// `(group name, override features)`. Only the overrides are listed: omitting
/// all of them selects the group's default, which can never conflict.
const GROUPS: &[(&str, &[&str])] = &[
    ("async runtime", &["ASYNC_RUNTIME_ASYNC_STD"]),
    ("socket", &["SOCKET_ASYNC_STD_TCP", "SOCKET_RIO_TCP"]),
    ("threadpool", &["THREADPOOL_CROSSBEAM"]),
    ("hash", &["CRYPTO_HASH_RING_SHA2"]),
    ("async channel", &["CHANNEL_ASYNC_CHANNEL_MPMC"]),
    ("sync channel", &["CHANNEL_SYNC_FLUME"]),
    ("dump queue", &["CHANNEL_CUSTOM_DUMP_LFB"]),
    (
        "collections RandomState",
        &[
            "COLLECTIONS_RANDOMSTATE_STD",
            "COLLECTIONS_RANDOMSTATE_TWOX_HASH",
            "COLLECTIONS_RANDOMSTATE_GXHASH",
        ],
    ),
    (
        "persistent db",
        &["PERSISTENT_DB_ROCKSDB", "PERSISTENT_DB_DISABLED"],
    ),
    ("serialization", &["SERIALIZE_CAPNP"]),
];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    for (group, overrides) in GROUPS {
        let enabled: Vec<String> = overrides
            .iter()
            .filter(|feat| std::env::var_os(format!("CARGO_FEATURE_{feat}")).is_some())
            .map(|feat| feat.to_lowercase())
            .collect();

        if enabled.len() > 1 {
            panic!(
                "atlas-common: the {group} backend is over-specified.\n\
                 \n\
                 Enabled at once: {}\n\
                 \n\
                 Pick at most one of these; omit all of them to get the default \
                 backend. Note that Cargo unions features across the whole \
                 dependency graph, so these may come from different crates \
                 rather than from a single Cargo.toml.",
                enabled.join(", ")
            );
        }
    }
}
