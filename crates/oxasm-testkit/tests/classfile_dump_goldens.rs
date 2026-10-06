//! The `ClassFile` dumper reproduces ASM's own dump of every T0 class byte for byte: the committed
//! `goldens/dump/<name>.{txt,err}` that `cargo xtask golden --regen` writes from the oracle.

use std::fs;
use std::path::{Path, PathBuf};

use oxasm_testkit::classfile_dump;
use oxasm_testkit::corpus_t0;
use oxasm_testkit::golden::{self, Golden};

/// Every T0 class file with its golden name (`jdk3/AllStructures$1.class` →
/// `jdk3.AllStructures$1`), skipping `upstream/`.
fn t0_classes(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path != base.join("upstream") {
                t0_classes(base, &path, out);
            }
            continue;
        }
        let relative = path
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(stem) = relative
            .strip_suffix(".class")
            .or_else(|| relative.strip_suffix(".clazz"))
        {
            out.push((stem.replace('/', "."), path.clone()));
        }
    }
}

#[test]
fn dump_equals_the_oracle_golden_for_every_t0_class() {
    let t0 = corpus_t0();
    let mut classes = Vec::new();
    t0_classes(&t0, &t0, &mut classes);
    assert!(classes.len() > 60, "T0 has {} classes", classes.len());
    let mut mismatches = Vec::new();
    for (name, path) in &classes {
        let bytes = fs::read(path).unwrap();
        let actual = match classfile_dump::dump(&bytes) {
            Ok(dump) => Golden::Ok(dump),
            Err(e) => Golden::Err(String::from_utf8(e.describe()).unwrap()),
        };
        if actual != golden::dump(name) {
            mismatches.push(name.clone());
        }
    }
    assert!(mismatches.is_empty(), "dump mismatches: {mismatches:?}");
}
