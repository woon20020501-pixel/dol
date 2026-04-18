//! Byte-exact parity against the **actual** `pacifica-fi/python-sdk`
//! `common/utils.prepare_message` — not a reimplementation, not a fixture.
//!
//! We spawn a Python subprocess with `PYTHONPATH` pointing at the
//! git-submodule checkout, stream JSON test cases on stdin, and compare
//! each emitted message against `bot_adapters::pacifica_sign::prepare_message`
//! byte-for-byte.
//!
//! Gated behind the submodule: if `bot-rs/third_party/pacifica-python-sdk/
//! common/utils.py` is missing, the test fails with a clear remedy
//! ("run `git submodule update --init --recursive`") rather than silently
//! passing. The `prepare_message` function in the SDK depends only on the
//! Python standard library (`json`), so no pip-install step is required.
//!
//! To run:
//! ```text
//! git submodule update --init --recursive
//! cargo test -p bot-adapters --test pacifica_sdk_subprocess
//! ```
//!
//! Reference: <https://github.com/pacifica-fi/python-sdk>
//!   commit pinned in `.gitmodules`.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use bot_adapters::pacifica_sign::prepare_message;
use serde::Deserialize;
use serde_json::Value;

/// Path to the submodule, resolved at compile time from the crate root.
fn sdk_path() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/bot-adapters; walk up two to bot-rs/.
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_root
        .parent()
        .expect("crates dir")
        .parent()
        .expect("bot-rs root")
        .join("third_party")
        .join("pacifica-python-sdk")
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    name: String,
    header: Value,
    payload: Value,
    expected_message: String,
}

fn load_cases() -> Vec<FixtureCase> {
    let raw = std::fs::read_to_string("tests/fixtures/pacifica_signing.json")
        .expect("read pacifica_signing.json");
    serde_json::from_str(&raw).expect("parse pacifica_signing.json")
}

/// Try to find a Python 3 interpreter. Windows typically ships `py`;
/// POSIX usually has `python3`. We try both and return the first that
/// responds to `--version`.
fn find_python() -> Option<String> {
    for candidate in ["py", "python3", "python"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Driver script: reads one JSON line per case from stdin, writes the
/// Python SDK's `prepare_message` output on stdout.
///
/// `common/utils.py` has `import base58` at the top for `sign_message`
/// even though `prepare_message` itself only uses `json`. We stub out
/// `base58` in `sys.modules` before loading so no pip install is needed
/// — the functions we actually call remain the SDK's real code.
const DRIVER_PY: &str = r#"
import sys, json, importlib.util, types
sdk_root = sys.argv[1]
sys.path.insert(0, sdk_root)

# Stub base58 so `import base58` at the top of common/utils.py succeeds.
# `prepare_message` never references base58, so this cannot bias the test.
sys.modules.setdefault("base58", types.ModuleType("base58"))

from common.utils import prepare_message  # noqa: E402

for line in sys.stdin:
    line = line.rstrip("\n")
    if not line:
        continue
    case = json.loads(line)
    msg = prepare_message(case["header"], case["payload"])
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()
"#;

#[test]
fn pacifica_sdk_subprocess_byte_exact_parity() {
    let sdk = sdk_path();
    let utils_py = sdk.join("common").join("utils.py");
    assert!(
        utils_py.exists(),
        "Pacifica SDK submodule not initialized. \
         Expected {} to exist. \
         Run: `git submodule update --init --recursive`",
        utils_py.display()
    );

    let python =
        find_python().expect("no Python 3 interpreter on PATH (tried `py`, `python3`, `python`)");

    let cases = load_cases();
    assert!(!cases.is_empty(), "fixture must have at least one case");

    // Build stdin: one JSON line per case (header+payload only).
    let mut stdin_input = String::new();
    for c in &cases {
        let obj = serde_json::json!({ "header": c.header, "payload": c.payload });
        stdin_input.push_str(&serde_json::to_string(&obj).expect("serialize case"));
        stdin_input.push('\n');
    }

    // Spawn Python with the driver inline, passing the SDK path as argv[1].
    let mut child = Command::new(&python)
        .arg("-c")
        .arg(DRIVER_PY)
        .arg(sdk.to_str().expect("sdk path UTF-8"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python");

    // Pipe cases to stdin.
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin_input.as_bytes())
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait python");

    assert!(
        output.status.success(),
        "python driver failed: status={:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("python stdout UTF-8");
    let sdk_outputs: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        sdk_outputs.len(),
        cases.len(),
        "expected {} lines from Python SDK, got {}",
        cases.len(),
        sdk_outputs.len()
    );

    // Double-check: for each case, compare
    //   1. Rust prepare_message output
    //   2. Python SDK subprocess output
    //   3. The committed fixture `expected_message`
    // All three must match byte-for-byte.
    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let rust_out = prepare_message(&case.header, &case.payload)
            .unwrap_or_else(|e| panic!("Rust prepare_message err on {}: {}", case.name, e));
        let sdk_out = sdk_outputs[i];

        if rust_out != sdk_out {
            failures.push(format!(
                "case `{}`: Rust↔SDK divergence\n  rust: {}\n  sdk:  {}",
                case.name, rust_out, sdk_out
            ));
        }
        if sdk_out != case.expected_message {
            failures.push(format!(
                "case `{}`: SDK↔fixture divergence — fixture is stale\n  sdk:     {}\n  fixture: {}",
                case.name, sdk_out, case.expected_message
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} case(s) diverged from the live Pacifica Python SDK:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
