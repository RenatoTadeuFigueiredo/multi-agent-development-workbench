use std::fs;
use std::path::{Path, PathBuf};

const SOURCES: &[&str] = &[
    "doc/arch/contracts/workbench-local-protocol.yaml",
    "doc/arch/datamodels/provider-capabilities.schema.json",
    "doc/arch/datamodels/session-event.schema.json",
    "doc/arch/datamodels/session-key-envelope.schema.json",
    "doc/arch/datamodels/workbench-configuration.schema.json",
    "doc/arch/datamodels/workbench-lock.schema.json",
    "doc/arch/schemas/add-a-supervised-acp-adapter-for-an-externally-installed.cue",
    "doc/arch/schemas/add-a-supervised-claude-code-subscription-adapter-that-pins.cue",
    "doc/arch/schemas/build-the-workbench-orchestration-kernel-foundation-as-a.cue",
    "doc/arch/statecharts/session-lifecycle.md",
];

#[test]
fn generated_contract_fixtures_match_the_architecture_corpus() {
    let repository = repository_root();
    let fixture_root = repository.join("crates/workbench-testkit/fixtures/generated");

    for source in SOURCES {
        let source_path = repository.join(source);
        let file_name = source_path.file_name().expect("contract file name");
        let fixture_path = fixture_root.join(file_name);
        let expected = fs::read(&source_path).expect("read source contract");
        let actual = fs::read(&fixture_path).expect("read generated fixture");
        assert_eq!(
            actual,
            expected,
            "generated fixture drifted from {}",
            source_path.display()
        );
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}
