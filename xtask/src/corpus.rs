//! Corpus tiers: T0 (imported from an ASM checkout), T1 (the JDK runtime image) and T2 (Maven
//! jars).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::args::Args;
use crate::oracle::{java_home, maven_jar};
use crate::pin::{ASM_COMMIT, ASM_TAG};
use crate::{is_class_file, relative_id, root_path, walk_files};

/// Where the default-options Textifier goldens of ASM's own test suite are imported to.
pub const UPSTREAM_TEXTIFIER: &str = "corpus/t0/upstream/textifier";

/// ASM modules whose test resources contain class files (`Issue*.class`, `SerialVersion*.class`).
const RESOURCE_MODULES: [&str; 3] = ["asm", "asm-analysis", "asm-commons"];

pub fn cli(mut args: Args) -> Result<()> {
    match args.next_positional().as_deref() {
        Some("t0") => {
            let asm = args.value("--asm")?.map(PathBuf::from);
            let allow_dirty = args.flag("--allow-dirty");
            args.finish()?;
            import_t0(&asm.map_or_else(default_asm_checkout, Ok)?, allow_dirty)
        }
        Some("t1") => {
            args.finish()?;
            extract_t1()
        }
        Some("t2") => {
            args.finish()?;
            fetch_t2()
        }
        _ => bail!("usage: cargo xtask corpus <t0|t1|t2>"),
    }
}

fn default_asm_checkout() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("OXASM_ASM_SRC") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(Path::new(&home).join("opt/dev/jvm/asm"))
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("cannot run git")?;
    ensure!(
        output.status.success(),
        "git {} failed in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// The T0 corpus: ASM's precompiled test classes and test resources, plus the upstream Textifier
/// goldens, copied from a checkout at the pinned tag.
fn import_t0(asm: &Path, allow_dirty: bool) -> Result<()> {
    let commit = git(asm, &["rev-parse", "HEAD"])?;
    ensure!(
        commit.starts_with(ASM_COMMIT),
        "{} is at {commit}, not at {ASM_TAG} ({ASM_COMMIT}); run `git -C {} checkout {ASM_TAG}`",
        asm.display(),
        asm.display()
    );
    let dirty = git(asm, &["status", "--porcelain", "--untracked-files=no"])?;
    ensure!(
        dirty.is_empty() || allow_dirty,
        "{} has local changes (pass --allow-dirty to import anyway):\n{dirty}",
        asm.display()
    );

    let t0 = root_path("corpus/t0");
    if t0.exists() {
        fs::remove_dir_all(&t0)?;
    }
    let mut copied = 0usize;

    // Precompiled classes, annotations/ and invalid/*.clazz.
    let resources = asm.join("asm-test/src/main/resources");
    for file in walk_files(&resources)? {
        copy(&file, &t0.join(file.strip_prefix(&resources)?))?;
        copied += 1;
    }

    // Class files from the other modules' test resources.
    for module in RESOURCE_MODULES {
        let dir = asm.join(module).join("src/test/resources");
        for file in walk_files(&dir)?.into_iter().filter(|f| is_class_file(f)) {
            copy(
                &file,
                &t0.join("resources")
                    .join(module)
                    .join(file.strip_prefix(&dir)?),
            )?;
            copied += 1;
        }
    }

    // Textifier goldens; TextifierTest strips '\r' before comparing, and so do we.
    let goldens = asm.join("asm-util/src/test/resources");
    let upstream = root_path(UPSTREAM_TEXTIFIER);
    for file in walk_files(&goldens)? {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        if !name.ends_with(".txt") || name.starts_with("sigtest-") {
            continue;
        }
        let text = fs::read(&file)?
            .into_iter()
            .filter(|&b| b != b'\r')
            .collect::<Vec<_>>();
        let target = upstream.join(file.strip_prefix(&goldens)?);
        fs::create_dir_all(target.parent().expect("file has a parent"))?;
        fs::write(&target, text)?;
        copied += 1;
    }

    fs::write(
        t0.join("PROVENANCE"),
        format!(
            "Imported by `cargo xtask corpus t0` from OW2 ASM (BSD-3-Clause, see NOTICE).\n\
             tag:    {ASM_TAG}\n\
             commit: {commit}\n\
             \n\
             *                  asm-test/src/main/resources/**\n\
             resources/<module> <module>/src/test/resources/*.class\n\
             upstream/textifier asm-util/src/test/resources/*.txt (sigtest-* excluded, \\r \
             stripped)\n"
        ),
    )?;
    println!(
        "corpus t0: {copied} files from {} at {ASM_TAG}",
        asm.display()
    );
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to.parent().expect("file has a parent"))?;
    fs::copy(from, to).with_context(|| format!("cannot copy {}", from.display()))?;
    Ok(())
}

/// The T1 corpus: every class of the JDK runtime image, extracted with `jimage`.
fn extract_t1() -> Result<()> {
    let home = java_home()?;
    let modules = home.join("lib/modules");
    ensure!(modules.is_file(), "{} has no runtime image", home.display());
    let t1 = root_path("corpus/t1");
    if t1.exists() {
        fs::remove_dir_all(&t1)?;
    }
    fs::create_dir_all(&t1)?;
    let status = Command::new(home.join("bin/jimage"))
        .arg("extract")
        .arg("--dir")
        .arg(&t1)
        .arg(&modules)
        .status()
        .context("cannot run jimage")?;
    ensure!(status.success(), "jimage extract failed");
    // Record which JDK the tier came from.
    if let Ok(release) = fs::read(home.join("release")) {
        fs::write(t1.join("release"), release)?;
    }
    let classes = walk_files(&t1)?
        .into_iter()
        .filter(|f| is_class_file(f))
        .count();
    println!("corpus t1: {classes} classes from {}", home.display());
    Ok(())
}

/// The T2 corpus: the jars listed in `corpus/t2.txt`.
fn fetch_t2() -> Result<()> {
    let list = root_path("corpus/t2.txt");
    let text =
        fs::read_to_string(&list).with_context(|| format!("cannot read {}", list.display()))?;
    let t2 = root_path("corpus/t2");
    fs::create_dir_all(&t2)?;
    let mut count = 0usize;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let [group, artifact, version] = line.split(':').collect::<Vec<_>>()[..] else {
            bail!(
                "{}: expected groupId:artifactId:version, got {line:?}",
                list.display()
            );
        };
        let jar = maven_jar(group, artifact, version)?;
        fs::copy(&jar, t2.join(format!("{artifact}-{version}.jar")))?;
        count += 1;
    }
    println!(
        "corpus t2: {count} jars in {}",
        relative_id(crate::workspace_root(), &t2)
    );
    Ok(())
}
