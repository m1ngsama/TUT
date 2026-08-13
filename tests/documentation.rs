use tut::{HELP, MAX_FILE_BYTES};

const README: &str = include_str!("../README.md");
const MANUAL: &str = include_str!("../docs/tut.1");
const PROJECT: &str = include_str!("../docs/PROJECT.md");
const CHANGELOG: &str = include_str!("../CHANGELOG.md");
const CONTRIBUTING: &str = include_str!("../CONTRIBUTING.md");
const SECURITY: &str = include_str!("../SECURITY.md");
const SECURITY_AUDIT: &str = include_str!("../.github/workflows/security-audit.yml");
const MANIFEST: &str = include_str!("../Cargo.toml");

fn plain_manual() -> String {
    MANUAL.replace("\\-", "-")
}

fn readme_help_block() -> &'static str {
    let (_, remainder) = README
        .split_once("<!-- BEGIN TUT HELP -->")
        .expect("README contains the help-block start marker");
    let (block, _) = remainder
        .split_once("<!-- END TUT HELP -->")
        .expect("README contains the help-block end marker");
    block
        .trim()
        .strip_prefix("```text\n")
        .and_then(|block| block.strip_suffix("\n```"))
        .expect("README help block is a fenced text block")
}

#[test]
fn primary_documents_cover_the_public_cli_contract() {
    assert_eq!(readme_help_block(), HELP.trim_end());
    let manual = plain_manual();
    for interface in ["--help", "--version", "--log-file", "TUT_LOG_FILE"] {
        assert!(HELP.contains(interface), "--help omits {interface}");
        assert!(README.contains(interface), "README omits {interface}");
        assert!(manual.contains(interface), "manual omits {interface}");
    }

    for interface in ["standard input", "/dev/tty", "UTF-8"] {
        assert!(README.contains(interface), "README omits {interface}");
        assert!(manual.contains(interface), "manual omits {interface}");
    }
}

#[test]
fn package_metadata_describes_the_documented_product() {
    assert_eq!(
        env!("CARGO_PKG_DESCRIPTION"),
        "A local plain-text reader for the terminal"
    );
    assert_eq!(
        env!("CARGO_PKG_HOMEPAGE"),
        "https://github.com/m1ngsama/TUT"
    );
    assert_eq!(
        env!("CARGO_PKG_REPOSITORY"),
        "https://github.com/m1ngsama/TUT"
    );
    assert_eq!(env!("CARGO_PKG_LICENSE"), "MIT");
    assert_eq!(env!("CARGO_PKG_RUST_VERSION"), "1.88.0");
    assert!(MANIFEST.contains("readme = \"README.md\""));
    assert!(README.contains("Linux and macOS"));
    assert!(README.contains("not yet a complete replacement for `less`"));
}

#[test]
fn user_documents_cover_the_interactive_contract() {
    let manual = plain_manual();
    for key in [
        "F1",
        "Ctrl-C",
        "Ctrl-D",
        "Ctrl-U",
        "PageDown",
        "Backspace",
        "9999",
    ] {
        assert!(README.contains(key), "README omits {key}");
        assert!(manual.contains(key), "manual omits {key}");
    }

    for behavior in ["literal", "case-sensitive", "soft wrap"] {
        assert!(README.contains(behavior), "README omits {behavior}");
        assert!(manual.contains(behavior), "manual omits {behavior}");
    }
}

#[test]
fn user_documents_record_supported_limits_and_exit_statuses() {
    assert_eq!(MAX_FILE_BYTES, 32 * 1024 * 1024);
    let manual = plain_manual();
    for limit in ["32 MiB", "4096", "16 columns by 4 rows"] {
        assert!(README.contains(limit), "README omits {limit}");
        assert!(manual.contains(limit), "manual omits {limit}");
    }

    for status in [".B 0", ".B 1", ".B 2", ".B 129, 130, 131, 143"] {
        assert!(MANUAL.contains(status), "manual omits exit status {status}");
    }
}

#[test]
fn release_and_maintenance_contracts_ship_with_the_crate() {
    assert!(CHANGELOG.contains("## Unreleased"));
    let current_release = format!("## {} -", env!("CARGO_PKG_VERSION"));
    assert!(
        CHANGELOG.contains(&current_release),
        "CHANGELOG has no release entry for {}",
        env!("CARGO_PKG_VERSION")
    );

    for principle in [
        "Do one job well",
        "Compose like a Unix tool",
        "Keep work bounded and interruptible",
        "Admit features deliberately",
    ] {
        assert!(
            PROJECT.contains(principle),
            "project contract omits {principle}"
        );
    }

    assert!(CONTRIBUTING.contains("cargo test --all-targets --locked"));
    assert!(CONTRIBUTING.contains("docs/tut.1"));
    assert!(SECURITY.contains("contact@m1ng.space"));
    assert!(SECURITY.contains("Do not open a public issue"));
}

#[test]
fn dependency_audit_commands_stay_pinned_and_discoverable() {
    for contract in [
        "cargo-audit --locked --version '=0.22.2'",
        "audit --file Cargo.lock",
    ] {
        assert!(
            CONTRIBUTING.contains(contract),
            "contributor guidance omits {contract}"
        );
        assert!(
            SECURITY_AUDIT.contains(contract),
            "security audit workflow omits {contract}"
        );
    }
    assert!(
        README.contains("cargo audit --file Cargo.lock"),
        "README local checks omit the dependency audit gate"
    );
}
