//! `cargo xtask compat --host-ref <ref>`: this build's backend against the session host
//! of another version.
//!
//! An application that is updated finds the host the previous version left running,
//! and has to work with it until it is restarted. This builds `lume-host` as it was at
//! `<ref>` (a tag, a branch or a commit), from a copy of the source at that point in
//! `target/compat/`, and runs the backend's case for another build's host against it.
//!
//! The copy is made with `git archive`, so nothing in the repository changes. It is
//! built against the terminal engine this tree has built (`target/vt/<triple>`): a ref
//! that pins another version of the engine needs it built for that version first.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{fetch, paths, run};

/// The backend's test file and case that run against another build's host.
const TEST_FILE: &str = "daemon";
const TEST_CASE: &str = "the_backend_works_with_the_host_another_build_left_running";

/// The variable that names the other host to the test.
const COMPAT_HOST: &str = "LUME_COMPAT_HOST";

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(paths::root())
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)
        .context("git's output is not UTF-8")?
        .trim()
        .to_owned())
}

/// The source as it was at `commit`, unpacked once into `target/compat/<commit>`.
fn source_at(commit: &str) -> Result<PathBuf> {
    let base = paths::root().join("target").join("compat");
    let dir = base.join(&commit[..12.min(commit.len())]);
    let done = dir.join(".unpacked");
    if done.is_file() {
        return Ok(dir);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let archive = base.join(format!("{commit}.tar"));
    let archive_text = archive.display().to_string();
    git(&["archive", "--format=tar", "-o", &archive_text, commit])?;
    let mut tar = Command::new(fetch::system_tar());
    tar.arg("-x").arg("-f").arg(&archive).arg("-C").arg(&dir);
    run::check(tar, "tar")?;
    std::fs::remove_file(&archive).ok();
    std::fs::write(&done, commit).with_context(|| format!("writing {}", done.display()))?;
    Ok(dir)
}

/// Builds the host in `source`, into a target folder shared by every ref.
fn build_host(source: &Path) -> Result<PathBuf> {
    let target = paths::root().join("target").join("compat").join("target");
    let engine = paths::root()
        .join("target")
        .join("vt")
        .join(paths::host_triple()?);
    let mut build = run::cargo_command();
    build
        .args(["build", "-p", "lume-host", "--bin", "lume-host"])
        .current_dir(source)
        .env("CARGO_TARGET_DIR", &target)
        .env("LUME_VT_DIR", &engine);
    run::check(build, "cargo build -p lume-host (at the ref)")?;
    let host = target.join("debug").join(exe("lume-host"));
    if !host.is_file() {
        bail!("the build left no host at {}", host.display());
    }
    Ok(host)
}

/// The newest release tag whose host can run on its own, which is the host an updated
/// application may find running.
fn latest_release_with_a_daemon() -> Result<Option<String>> {
    let tags = git(&["tag", "--list", "v*", "--sort=-v:refname"])?;
    for tag in tags.lines().map(str::trim).filter(|tag| !tag.is_empty()) {
        let host = git(&[
            "show",
            &format!("{tag}:crates/lume-host/src/bin/lume-host.rs"),
        ]);
        if host.is_ok_and(|text| text.contains("\"serve\"")) {
            return Ok(Some(tag.to_owned()));
        }
    }
    Ok(None)
}

/// Runs against the newest release whose host runs on its own, or says why nothing ran.
pub fn run_latest_release() -> Result<()> {
    match latest_release_with_a_daemon()? {
        Some(tag) => run(&tag),
        None => {
            println!(
                "compat: skipped: no release tag (v*) has a session host that runs on its own \
                 yet, so no application can find one left running"
            );
            Ok(())
        }
    }
}

pub fn run(host_ref: &str) -> Result<()> {
    let commit = git(&["rev-parse", "--verify", &format!("{host_ref}^{{commit}}")])?;
    println!("compat: the host as it was at {host_ref} ({commit})");
    let source = source_at(&commit)?;
    let host = build_host(&source)?;

    // The whole workspace's test build, so that what is tested is this tree's backend.
    let mut test = run::cargo_command();
    test.args([
        "test",
        "-p",
        "lume-session",
        "--test",
        TEST_FILE,
        "--",
        "--ignored",
        "--nocapture",
        "--exact",
        TEST_CASE,
    ])
    .current_dir(paths::root())
    .env(COMPAT_HOST, &host);
    run::check(test, "the backend against the other host")?;
    println!("compat: this build's backend works with the host of {host_ref}");
    Ok(())
}
