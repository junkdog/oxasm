//! The committed goldens are complete and consistent, offline (no Java needed).
//! `cargo xtask golden --check` proves that they match the oracle.

use std::fs;
use std::path::Path;

use oxasm_testkit::golden::{self, Golden, ReaderOptions};
use oxasm_testkit::{InvalidClass, PrecompiledClass, corpus_t0};

/// Every class file under `dir`, as golden names (`jdk3/AllStructures$1.class` →
/// `jdk3.AllStructures$1`), skipping `upstream/`.
fn class_names(base: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path != base.join("upstream") {
                class_names(base, &path, out);
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
            out.push(stem.replace('/', "."));
        }
    }
}

#[test]
fn every_t0_class_has_every_golden() {
    let t0 = corpus_t0();
    let mut names = Vec::new();
    class_names(&t0, &t0, &mut names);
    assert!(names.len() > 60, "T0 has {} classes", names.len());
    for name in &names {
        for options in ReaderOptions::ALL {
            golden::textifier(name, options);
        }
        golden::dump(name);
    }
}

#[test]
fn precompiled_classes_textify_and_invalid_classes_fail() {
    for class in PrecompiledClass::ALL {
        for options in ReaderOptions::ALL {
            assert!(
                matches!(golden::textifier(class.name(), options), Golden::Ok(_)),
                "{class} {options:?}"
            );
        }
    }
    let failing: Vec<_> = InvalidClass::ALL
        .iter()
        .filter(|c| {
            matches!(
                golden::textifier(c.name(), ReaderOptions::Default),
                Golden::Err(_)
            )
        })
        .collect();
    // ASM rejects every invalid class when reading everything.
    assert_eq!(failing.len(), InvalidClass::ALL.len());
}

/// The offline half of the P0 gate: the default-options Textifier goldens include ASM's own.
#[test]
fn default_textifier_goldens_equal_upstream() {
    let upstream = corpus_t0().join("upstream/textifier");
    let mut count = 0;
    for entry in fs::read_dir(&upstream).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let expected = fs::read(&path).unwrap();
        let actual = golden::textifier(&name, ReaderOptions::Default).unwrap();
        assert!(expected == actual, "{name} differs from upstream");
        count += 1;
    }
    // Every precompiled class except the two LargeMethods, which TextifierTest skips.
    assert_eq!(count, PrecompiledClass::ALL.len() - 2);
}
