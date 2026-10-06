//! The signature corpus of ASM's `SignaturesProviders`: every class, field and method signature
//! of the precompiled classes that ASM7 can read, plus 48 deeply nested field signatures.
//!
//! ASM collects the signatures with a `ClassReader`. Until oxasm has one (P2), they are taken
//! from the committed Textifier goldens, which print each signature as a `// signature` comment
//! right before the declaration it belongs to: a class at column 0, a field or method at column 2
//! (a method's declaration has a `(`), and a local variable at column 4, which ASM's provider
//! does not collect.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::golden::{self, ReaderOptions};
use crate::precompiled::{JdkVersion, PrecompiledClass};

/// The class, field and method signatures of [`signatures`].
#[derive(Debug, Default)]
pub struct Signatures {
    pub class: Vec<String>,
    pub field: Vec<String>,
    pub method: Vec<String>,
}

/// The corpus, built on first use.
pub fn signatures() -> &'static Signatures {
    static SIGNATURES: OnceLock<Signatures> = OnceLock::new();
    SIGNATURES.get_or_init(collect)
}

fn collect() -> Signatures {
    let (mut class, mut field, mut method) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    // `!isMoreRecentThan(Api.ASM7)`: the JDK 14 and 15 classes need ASM8 and ASM9.
    for precompiled in PrecompiledClass::ALL
        .iter()
        .filter(|c| c.jdk_version() <= JdkVersion::Jdk11)
    {
        let text = golden::textifier(precompiled.name(), ReaderOptions::Default).unwrap_text();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let indent = line.len() - line.trim_start().len();
            let Some(signature) = line.trim_start().strip_prefix("// signature ") else {
                continue;
            };
            // The declaration is the next line that is not a comment.
            let declaration = lines[i + 1..]
                .iter()
                .find(|l| !l.trim_start().starts_with("//"))
                .expect("a signature comment precedes a declaration");
            match indent {
                0 => class.insert(signature.to_owned()),
                2 if declaration.contains('(') => method.insert(signature.to_owned()),
                2 => field.insert(signature.to_owned()),
                _ => false,
            };
        }
    }
    assert!(!class.is_empty() && !field.is_empty() && !method.is_empty());
    let mut field: Vec<String> = field.into_iter().collect();
    for depth in 0..48 {
        let mut signature = String::new();
        build_deep_signature(&mut signature, depth);
        field.push(signature);
    }
    Signatures {
        class: class.into_iter().collect(),
        field,
        method: method.into_iter().collect(),
    }
}

fn build_deep_signature(signature: &mut String, depth: usize) {
    signature.push_str("LGeneric");
    if depth == 0 {
        signature.push(';');
    } else {
        signature.push_str("<LOpen;");
        build_deep_signature(signature, depth - 1);
        signature.push_str("LClose;>;");
    }
}
