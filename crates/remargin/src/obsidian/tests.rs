use os_shim::mock::MockSystem;

use super::*;

/// Stub `main.js` bytes used by every install test so we never hit the
/// network. Contents are arbitrary -- only the length and identity
/// matter for the assertions.
const STUB_MAIN_JS: &[u8] = b"// stub main.js\nconsole.log('remargin');\n";
/// Stub `manifest.json` bytes. Valid JSON but also arbitrary.
const STUB_MANIFEST: &[u8] = br#"{"id":"remargin","name":"Remargin","version":"0.0.0-test"}"#;

fn seed_vault(fs: &MockSystem, vault: &Path) {
    fs.create_dir_all(&vault.join(".obsidian")).unwrap();
}

fn install_stub(fs: &MockSystem, cwd: &Path, vault_path: Option<&Path>) -> anyhow::Result<Report> {
    install_from_bytes(fs, cwd, vault_path, STUB_MAIN_JS, STUB_MANIFEST)
}

#[test]
fn install_creates_plugin_dir_with_artifacts() {
    let fs = MockSystem::new();
    let vault = PathBuf::from("/home/user/vault");
    seed_vault(&fs, &vault);

    let report = install_stub(&fs, &vault, None).unwrap();
    assert_eq!(report.plugin_dir, vault.join(PLUGIN_REL_PATH));
    assert!(report.preserved_data_bytes.is_none());
    assert_eq!(report.main_js_bytes, STUB_MAIN_JS.len());
    assert_eq!(report.manifest_bytes, STUB_MANIFEST.len());
    assert!(fs.exists(&report.plugin_dir.join("main.js")).unwrap());
    assert!(fs.exists(&report.plugin_dir.join("manifest.json")).unwrap());
}

#[test]
fn install_writes_exact_bytes() {
    let fs = MockSystem::new();
    let vault = PathBuf::from("/home/user/vault");
    seed_vault(&fs, &vault);

    let report = install_stub(&fs, &vault, None).unwrap();
    let main_js = fs
        .read_to_string(&report.plugin_dir.join("main.js"))
        .unwrap();
    let manifest = fs
        .read_to_string(&report.plugin_dir.join("manifest.json"))
        .unwrap();
    assert_eq!(main_js.as_bytes(), STUB_MAIN_JS);
    assert_eq!(manifest.as_bytes(), STUB_MANIFEST);
}

#[test]
fn install_errors_when_not_a_vault() {
    let fs = MockSystem::new();
    let cwd = Path::new("/home/user/docs");
    fs.create_dir_all(cwd).unwrap();

    let err = install_stub(&fs, cwd, None).unwrap_err();
    assert!(err.to_string().contains("not an Obsidian vault"));
}

#[test]
fn install_is_idempotent() {
    let fs = MockSystem::new();
    let vault = PathBuf::from("/home/user/vault");
    seed_vault(&fs, &vault);

    install_stub(&fs, &vault, None).unwrap();
    let second = install_stub(&fs, &vault, None).unwrap();
    assert!(fs.exists(&second.plugin_dir.join("main.js")).unwrap());
}

#[test]
fn install_preserves_data_json() {
    let fs = MockSystem::new();
    let vault = PathBuf::from("/home/user/vault");
    seed_vault(&fs, &vault);

    // Seed an existing install with a data.json.
    let plugin_dir = vault.join(PLUGIN_REL_PATH);
    fs.create_dir_all(&plugin_dir).unwrap();
    let data_json = plugin_dir.join("data.json");
    let payload = br#"{"identity":"alice","sidebarSide":"left"}"#;
    fs.write(&data_json, payload).unwrap();

    let report = install_stub(&fs, &vault, None).unwrap();
    assert_eq!(report.preserved_data_bytes, Some(payload.len()));
    let preserved = fs.read_to_string(&data_json).unwrap();
    assert_eq!(preserved.as_bytes(), payload);
}

#[test]
fn install_with_explicit_vault_path() {
    let fs = MockSystem::new();
    let cwd = PathBuf::from("/tmp/anywhere");
    let vault = PathBuf::from("/home/user/other-vault");
    fs.create_dir_all(&cwd).unwrap();
    seed_vault(&fs, &vault);

    let report = install_stub(&fs, &cwd, Some(&vault)).unwrap();
    assert_eq!(report.plugin_dir, vault.join(PLUGIN_REL_PATH));
}

#[test]
fn uninstall_errors_when_not_a_vault() {
    let fs = MockSystem::new();
    let cwd = Path::new("/home/user/docs");
    fs.create_dir_all(cwd).unwrap();

    let err = uninstall(&fs, cwd, None).unwrap_err();
    assert!(err.to_string().contains("not an Obsidian vault"));
}

#[test]
fn uninstall_is_noop_when_not_installed() {
    let fs = MockSystem::new();
    let vault = PathBuf::from("/home/user/vault");
    seed_vault(&fs, &vault);

    let status = uninstall(&fs, &vault, None).unwrap();
    assert!(matches!(status, UninstallStatus::NotInstalled { .. }));
}

#[test]
fn uninstall_removes_plugin_dir() {
    let fs = MockSystem::new();
    let vault = PathBuf::from("/home/user/vault");
    seed_vault(&fs, &vault);

    install_stub(&fs, &vault, None).unwrap();
    let status = uninstall(&fs, &vault, None).unwrap();
    assert!(matches!(status, UninstallStatus::Removed { .. }));
    if let UninstallStatus::Removed { plugin_dir } = status {
        assert!(!fs.exists(&plugin_dir).unwrap());
    }
}
