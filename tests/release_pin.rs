//! Tests for `cargo xtask release-pin` (rqm/tooling.md).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use xtask::release_pin::{self, Request};

fn temp_dir(label: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "tophat-release-pin-{}-{}-{}",
        label,
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn repository(label: &str) -> PathBuf {
    let root = temp_dir(label);
    let pin = root.join("template/.tophat/managed/tophat-release.yaml");
    fs::create_dir_all(pin.parent().unwrap()).unwrap();
    fs::write(&pin, "version: unreleased\n").unwrap();
    root
}

fn binary(dir: &Path, name: &str, version: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\nprintf 'tophat {version}\\n'\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn sha256(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path).unwrap());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test] // rq-39d23d3d
fn the_release_pin_is_written_from_the_binaries() {
    let root = repository("write");
    let a = binary(&root, "a", "0.2.0");
    let b = binary(&root, "b", "0.2.0");
    let report = release_pin::release_pin(
        &root,
        &Request {
            version: "0.2.0".into(),
            url: "https://example.org/download/v0.2.0".into(),
            binaries: vec![
                format!("x86_64-unknown-linux-musl={}", a.display()),
                format!("aarch64-unknown-linux-musl={}", b.display()),
            ],
        },
    )
    .unwrap();
    assert!(!report.failed, "{:?}", report.lines);
    let pin =
        fs::read_to_string(root.join("template/.tophat/managed/tophat-release.yaml")).unwrap();
    assert!(pin.contains("version: 0.2.0"), "{pin}");
    assert!(
        pin.contains("url: https://example.org/download/v0.2.0"),
        "{pin}"
    );
    assert!(pin.contains(&sha256(&a)), "{pin}");
    assert!(pin.contains(&sha256(&b)), "{pin}");
}

#[test] // rq-ac4f41eb
fn the_release_pin_refuses_a_missing_target() {
    let root = repository("missing");
    let a = binary(&root, "a", "0.2.0");
    let pin = root.join("template/.tophat/managed/tophat-release.yaml");
    let before = fs::read_to_string(&pin).unwrap();
    let report = release_pin::release_pin(
        &root,
        &Request {
            version: "0.2.0".into(),
            url: "https://example.org".into(),
            binaries: vec![format!("x86_64-unknown-linux-musl={}", a.display())],
        },
    )
    .unwrap();
    assert!(report.failed);
    assert!(
        report
            .lines
            .iter()
            .any(|line| line.contains("aarch64-unknown-linux-musl")),
        "{:?}",
        report.lines
    );
    assert_eq!(fs::read_to_string(&pin).unwrap(), before);
}

#[test] // rq-2ed82bd1
fn the_release_pin_refuses_a_version_mismatch() {
    let root = repository("version");
    let a = binary(&root, "a", "0.1.9");
    let b = binary(&root, "b", "0.2.0");
    let report = release_pin::release_pin(
        &root,
        &Request {
            version: "0.2.0".into(),
            url: "https://example.org".into(),
            binaries: vec![
                format!("x86_64-unknown-linux-musl={}", a.display()),
                format!("aarch64-unknown-linux-musl={}", b.display()),
            ],
        },
    )
    .unwrap();
    let text = report.lines.join("\n");
    assert!(report.failed, "{text}");
    assert!(text.contains("0.1.9"), "{text}");
    assert!(text.contains("0.2.0"), "{text}");
}
