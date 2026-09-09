use std::path::Path;

use os_shim::mock::MemorySystem;
use serde_json::json;

use super::{GuardDiagnostic, GuardDiagnosticInner, GuardOutcome, session_guard};
use crate::permissions::pretool_install::{HOOK_MATCHER, HOOK_SUBCOMMAND};

const EXE: &str = "/opt/bin/remargin";

/// A mock whose `PATH` contains a directory holding a `remargin` file, so
/// the on-PATH check passes and only the config check can fail.
fn mock_with_remargin_on_path() -> MemorySystem {
    MemorySystem::new()
        .with_dir(Path::new("/usr/bin"))
        .unwrap()
        .with_file(Path::new("/usr/bin/remargin"), b"")
        .unwrap()
        .with_env("PATH", "/usr/bin")
        .unwrap()
}

/// Project-scope settings declaring a `PreToolUse` entry that runs
/// `command`, under a realm at `/r`.
fn realm_with_hook_command(system: MemorySystem, command: &str) -> MemorySystem {
    let settings = json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": HOOK_MATCHER,
                    "hooks": [ { "type": "command", "command": command } ]
                }
            ]
        }
    });
    system
        .with_dir(Path::new("/r"))
        .unwrap()
        .with_file(
            Path::new("/r/.claude/settings.json"),
            serde_json::to_string_pretty(&settings).unwrap().as_bytes(),
        )
        .unwrap()
}

/// A realm at `/r` whose project-scope settings declare a live entry: an
/// absolute command whose binary is on disk.
fn realm_with_live_hook(system: MemorySystem) -> MemorySystem {
    realm_with_hook_command(
        system.with_file(Path::new(EXE), b"binary").unwrap(),
        &format!("{EXE} {HOOK_SUBCOMMAND}"),
    )
}

/// Destructure a `Fail` outcome without a `panic!` (denied by clippy). The
/// `matches!` assert carries the diagnostic; the else arm is unreachable.
fn expect_fail(outcome: GuardOutcome) -> GuardDiagnostic {
    assert!(
        matches!(outcome, GuardOutcome::Fail(_)),
        "expected GuardOutcome::Fail, got {outcome:?}",
    );
    let GuardOutcome::Fail(diagnostic) = outcome else {
        return GuardDiagnostic {
            hook_specific_output: GuardDiagnosticInner {
                additional_context: String::new(),
                hook_event_name: "SessionStart",
            },
            system_message: String::new(),
        };
    };
    diagnostic
}

/// Case 4: an unparseable realm `.remargin.yaml` above cwd → the guard
/// fails and surfaces a diagnostic naming the parse failure.
#[test]
fn unparseable_realm_config_fails() {
    let system = realm_with_live_hook(mock_with_remargin_on_path())
        .with_file(Path::new("/r/.remargin.yaml"), b": : not valid yaml : :")
        .unwrap();

    let diag = expect_fail(session_guard(&system, Path::new("/r")));
    assert_eq!(diag.hook_specific_output.hook_event_name, "SessionStart");
    assert!(
        diag.hook_specific_output
            .additional_context
            .contains(".remargin.yaml"),
        "diagnostic should name the config: {}",
        diag.hook_specific_output.additional_context,
    );
    assert!(
        diag.system_message.contains("remargin doctor"),
        "system message should point at doctor: {}",
        diag.system_message,
    );
}

/// A live hook entry + parseable config → the session proceeds clean.
#[test]
fn live_hook_and_config_parses_is_ok() {
    let system = realm_with_live_hook(mock_with_remargin_on_path())
        .with_file(
            Path::new("/r/.remargin.yaml"),
            b"identity: alice\ntype: human\n",
        )
        .unwrap();

    assert_eq!(session_guard(&system, Path::new("/r")), GuardOutcome::Ok);
}

/// No `.remargin.yaml` on the walk is not a failure — an absent realm
/// config parses vacuously.
#[test]
fn no_realm_config_is_ok_with_a_live_hook() {
    let system = realm_with_live_hook(mock_with_remargin_on_path());

    assert_eq!(session_guard(&system, Path::new("/r")), GuardOutcome::Ok);
}

/// Neither settings scope declares an entry → the guard fails and names
/// both scopes plus the install command. A `remargin` that resolves on
/// `PATH` proves nothing here: nothing is registered to spawn it, so no
/// tool call is gated.
#[test]
fn no_hook_entry_in_either_scope_fails() {
    let system = mock_with_remargin_on_path()
        .with_env("HOME", "/h")
        .unwrap()
        .with_dir(Path::new("/h"))
        .unwrap()
        .with_dir(Path::new("/r"))
        .unwrap();

    let diag = expect_fail(session_guard(&system, Path::new("/r")));
    let context = diag.hook_specific_output.additional_context;
    assert!(
        context.contains("/h/.claude/settings.json")
            && context.contains("/r/.claude/settings.json")
            && context.contains("remargin claude pretool install"),
        "diagnostic should name both scopes and the install command: {context}",
    );
}

/// A missing `PATH` variable is treated as "not resolvable" → the entry
/// that resolves through it cannot spawn, so the guard fails.
#[test]
fn missing_path_var_fails() {
    let system =
        realm_with_hook_command(MemorySystem::new(), &format!("remargin {HOOK_SUBCOMMAND}"));

    assert!(matches!(
        session_guard(&system, Path::new("/r")),
        GuardOutcome::Fail(_)
    ));
}

/// The installed entry names an absolute binary that is on disk, so the
/// hook will spawn — `PATH` says nothing about it, and an empty `PATH` is
/// no longer a failure.
#[test]
fn absolute_hook_command_is_ok_without_the_binary_on_path() {
    let system = realm_with_live_hook(MemorySystem::new().with_env("PATH", "/usr/bin").unwrap());

    assert_eq!(session_guard(&system, Path::new("/r")), GuardOutcome::Ok);
}

/// The installed entry names an absolute binary that is gone: the hook
/// cannot spawn, so the guard fails and names the binary — even though
/// another `remargin` does resolve on `PATH`.
#[test]
fn stale_absolute_hook_command_fails_and_names_the_binary() {
    let system = realm_with_hook_command(
        mock_with_remargin_on_path(),
        &format!("/gone/remargin {HOOK_SUBCOMMAND}"),
    );

    let diag = expect_fail(session_guard(&system, Path::new("/r")));
    let context = diag.hook_specific_output.additional_context;
    assert!(
        context.contains("/gone/remargin") && context.contains("does not exist"),
        "diagnostic should name the vanished binary: {context}",
    );
}

/// An entry an older install left behind resolves through `PATH`, so that
/// is what the guard checks it against — present here, so the session is
/// clean.
#[test]
fn path_relative_hook_command_falls_back_to_the_path_probe() {
    let legacy = format!("remargin {HOOK_SUBCOMMAND}");
    let clean = realm_with_hook_command(mock_with_remargin_on_path(), &legacy);
    assert_eq!(session_guard(&clean, Path::new("/r")), GuardOutcome::Ok);

    let broken = realm_with_hook_command(
        MemorySystem::new().with_env("PATH", "/usr/bin").unwrap(),
        &legacy,
    );
    let diag = expect_fail(session_guard(&broken, Path::new("/r")));
    assert!(
        diag.hook_specific_output
            .additional_context
            .contains("PATH"),
        "diagnostic should mention PATH: {}",
        diag.hook_specific_output.additional_context,
    );
}
