//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::ffi::OsString;

/// What a Snakemake-backed test does, given `TOPHAT_TEST_SNAKEMAKE` and `TOPHAT_REQUIRE_SNAKEMAKE`
/// (see `rqm/testing.md`): run with the given Snakemake, skip, or fail because Snakemake is
/// required.
pub fn snakemake_from(
    test: Option<OsString>,
    require: Option<OsString>,
) -> Result<Option<OsString>, String> {
    match (test, require) {
        (Some(path), _) if !path.is_empty() => Ok(Some(path)),
        (_, Some(flag)) if flag == "1" => Err(
            "Snakemake is required (TOPHAT_REQUIRE_SNAKEMAKE=1) but TOPHAT_TEST_SNAKEMAKE is unset"
                .into(),
        ),
        _ => Ok(None),
    }
}

/// The Snakemake executable for a Snakemake-backed test, or `None` (after printing a skip notice)
/// when the test should pass without running. Panics when `TOPHAT_REQUIRE_SNAKEMAKE=1` and no
/// Snakemake is given, so a CI job meant to run these tests cannot pass without running them.
pub fn snakemake() -> Option<OsString> {
    match snakemake_from(
        std::env::var_os("TOPHAT_TEST_SNAKEMAKE"),
        std::env::var_os("TOPHAT_REQUIRE_SNAKEMAKE"),
    ) {
        Ok(Some(path)) => Some(path),
        Ok(None) => {
            eprintln!(
                "skipped: set TOPHAT_TEST_SNAKEMAKE to a Snakemake executable to run this test"
            );
            None
        }
        Err(message) => panic!("{message}"),
    }
}

/// Whether a Copier-backed test runs: `copier` must be on `PATH`. Without it the test prints a
/// skip notice and passes, unless `TOPHAT_REQUIRE_COPIER=1`, in which case it panics, so a CI job
/// meant to run these tests cannot pass without running them.
pub fn copier() -> bool {
    let found = std::process::Command::new("copier")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    match (found, std::env::var_os("TOPHAT_REQUIRE_COPIER")) {
        (true, _) => true,
        (false, Some(flag)) if flag == "1" => {
            panic!("Copier is required (TOPHAT_REQUIRE_COPIER=1) but `copier` is not on PATH")
        }
        (false, _) => {
            eprintln!("skipped: install Copier 9 (`copier` on PATH) to run this test");
            false
        }
    }
}
