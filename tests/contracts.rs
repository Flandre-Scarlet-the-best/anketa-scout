#[test]
fn cargo_version_and_toolchain_are_canonical_and_synchronized() {
    let cargo: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let toolchain: toml::Value = toml::from_str(include_str!("../rust-toolchain.toml")).unwrap();
    assert_eq!(
        cargo["package"]["rust-version"].as_str(),
        toolchain["toolchain"]["channel"].as_str()
    );
    assert_eq!(
        cargo["package"]["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(cargo["package"]["name"].as_str(), Some("anketa-scout"));
}

#[test]
fn renovate_extracts_only_canonical_msrv_from_actual_manifest() {
    let config: serde_json::Value = serde_json::from_str(include_str!("../renovate.json")).unwrap();
    let manager = &config["customManagers"][0];
    assert_eq!(manager["datasourceTemplate"], "rust-version");
    assert_eq!(manager["depNameTemplate"], "rust");
    let pattern = regex::Regex::new(manager["matchStrings"][0].as_str().unwrap()).unwrap();
    let captures: Vec<_> = pattern
        .captures_iter(include_str!("../Cargo.toml"))
        .collect();
    assert_eq!(captures.len(), 1);
    assert_eq!(&captures[0]["currentValue"], env!("CARGO_PKG_RUST_VERSION"));
    assert!(
        config["packageRules"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |rule| rule["matchDatasources"] == serde_json::json!(["rust-version"])
                    && rule["rangeStrategy"] == "replace"
            )
    );
}

#[test]
fn error_categories_have_stable_distinct_exit_codes_without_remote_data() {
    use anketa_scout::Failure;
    for (error, code) in [
        (Failure::Config("Ошибка конфигурации"), 2),
        (Failure::Network, 3),
        (Failure::Timeout, 3),
        (Failure::Unavailable, 4),
        (Failure::Access, 4),
        (Failure::RateLimit, 4),
        (Failure::Format, 5),
        (Failure::Partial, 6),
        (Failure::Limit, 7),
        (Failure::Output, 8),
    ] {
        assert_eq!(error.code(), code);
        assert!(!error.to_string().is_empty());
        assert!(!error.to_string().contains("http"));
    }
}
