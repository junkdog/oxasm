//! The `ClassFile` dumper never panics on malformed input, and bounds its recursion (see the
//! module docs of `classfile_dump` for where it reports a `StackOverflowError` instead).

use oxasm_testkit::classfile_dump::{self, ClassFile};
use oxasm_testkit::{InvalidClass, PrecompiledClass};

/// A deterministic xorshift generator (no dependency needed).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }
}

#[test]
fn truncated_and_mutated_classes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let classes = PrecompiledClass::ALL
        .iter()
        .map(|c| c.bytes())
        .chain(InvalidClass::ALL.iter().map(|c| c.bytes()));
    for bytes in classes {
        for i in 0..64 {
            let _ = classfile_dump::dump(&bytes[..bytes.len() * i / 64]);
        }
        for _ in 0..64 {
            let mut mutant = bytes.to_vec();
            for _ in 0..1 + rng.below(4) {
                let position = rng.below(mutant.len());
                mutant[position] =
                    [0, 1, 0x7F, 0x80, 0xFF, rng.next().to_le_bytes()[0]][rng.below(6)];
            }
            let _ = classfile_dump::dump(&mutant);
        }
    }
}

/// A minimal class file: version 52, the given constant pool entries (each already encoded, and
/// counting one slot), `this_class` #2 (`ConstantClassInfo` of #1), no super class, and the given
/// methods and class attributes (encoded with their counts).
fn class_file(constant_pool: &[Vec<u8>], methods: &[u8], attributes: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
    bytes.extend((u16::try_from(constant_pool.len()).unwrap() + 1).to_be_bytes());
    for entry in constant_pool {
        bytes.extend(entry);
    }
    bytes.extend([0, 0x21, 0, 2, 0, 0, 0, 0, 0, 0]);
    bytes.extend(methods);
    bytes.extend(attributes);
    bytes
}

fn utf8(s: &str) -> Vec<u8> {
    let mut entry = vec![1];
    entry.extend(u16::try_from(s.len()).unwrap().to_be_bytes());
    entry.extend(s.as_bytes());
    entry
}

#[test]
fn method_handle_cycle_is_a_stack_overflow() {
    // #3 is a method handle to itself.
    let bytes = class_file(
        &[utf8("A"), vec![7, 0, 1], vec![15, 1, 0, 3]],
        &[0, 0],
        &[0, 0],
    );

    let error = ClassFile::new(&bytes).dump().unwrap_err();

    assert_eq!(error.java_exception(), "java.lang.StackOverflowError");
}

#[test]
fn deeply_nested_element_values_do_not_overflow() {
    const DEPTH: usize = 100_000;
    let mut annotation = vec![0, 1, 0, 1, 0, 1, 0, 1]; // num_annotations, type, pairs, name
    for _ in 0..DEPTH {
        annotation.extend([b'[', 0, 1]);
    }
    annotation.extend([b's', 0, 1]);
    let mut attributes = vec![0, 1, 0, 3];
    attributes.extend(u32::try_from(annotation.len()).unwrap().to_be_bytes());
    attributes.extend(&annotation);
    let bytes = class_file(
        &[utf8("A"), vec![7, 0, 1], utf8("RuntimeVisibleAnnotations")],
        &[0, 0],
        &attributes,
    );

    let dump = ClassFile::new(&bytes).dump_lossy().unwrap();

    assert_eq!(dump.matches("[: 1\n").count(), DEPTH);
}

#[test]
fn deeply_nested_code_attributes_are_a_stack_overflow() {
    // One method whose Code attribute contains a Code attribute, and so on.
    let depth = classfile_dump::MAX_NESTING as usize + 10;
    let mut methods = vec![0, 1, 0, 0, 0, 1, 0, 1];
    for _ in 0..depth {
        // attributes_count, then a Code attribute without code nor exception handlers.
        methods.extend([0, 1, 0, 3, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0]);
    }
    methods.extend([0, 0]);
    let bytes = class_file(&[utf8("A"), vec![7, 0, 1], utf8("Code")], &methods, &[0, 0]);

    let error = ClassFile::new(&bytes).dump().unwrap_err();

    assert_eq!(error.java_exception(), "java.lang.StackOverflowError");
}
