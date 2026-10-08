//! The benchmark scenarios of oxasm against ASM 9.10.1 (oxasm-verification, "Benchmarks").
//!
//! Each [`Scenario`] mirrors one of ASM's JMH benchmarks (`AdapterBenchmark`,
//! `GeneratorBenchmark`, `TypeBenchmark`), and the oracle's `bench` command
//! (`oracle/Oracle.java`) runs ASM's side of it with the same calls. Both sides read their
//! classes from the same [pack](read_pack), written by `cargo xtask bench`, and return the same
//! check value per pass (an event count, the bytes written, ...), which xtask compares: a
//! scenario whose sides disagree on it did not do the same work.
//!
//! Three front ends use this crate:
//!
//! - `cargo xtask bench`, which runs this crate's `oxasm-bench` binary and the oracle in turn and
//!   prints the ratios;
//! - the `oxasm-bench` binary itself (a warm loop, as the oracle's);
//! - the criterion benches (`cargo bench -p oxasm-bench`), for regressions between oxasm
//!   versions.

mod visitors;

use std::path::Path;
use std::sync::Arc;

use oxasm::opcodes::{
    ACC_PUBLIC, ACC_STATIC, ALOAD, GETSTATIC, INVOKESPECIAL, INVOKEVIRTUAL, RETURN, V1_1,
};
use oxasm::{
    ClassHierarchy, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Constant, Interner,
    JavaStr, JavaString, MapHierarchy, ParsingOptions, Type, jstr,
};
use oxasm_tree::ClassNode;

use crate::visitors::{CollectTypes, Counter};

/// The number of classes `generate_hello_world` generates per pass.
pub const HELLO_WORLDS_PER_PASS: usize = 10_000;

/// One benchmark scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// `AdapterBenchmark.getClassInfo`: a class reader's header (access, name, super class and
    /// interfaces).
    ClassInfo,
    /// `AdapterBenchmark.read`: every event of the class, to a visitor that counts them.
    Read,
    /// `AdapterBenchmark.getClassObjectModel`: the class read into a `ClassNode`, with a new
    /// interner per class (as `new ClassNode()`).
    ReadTree,
    /// As `read_tree`, with one interner per pass (as sift shares one for a classpath). It has
    /// no ASM analogue of its own and compares with ASM's `read_tree`.
    ReadTreeShared,
    /// `AdapterBenchmark.readAndWrite(false)`: reader into `ClassWriter(0)`.
    ReadWrite,
    /// `AdapterBenchmark.readAndWriteWithCopyPool`: reader into `ClassWriter(reader, 0)`.
    ReadWriteCopyPool,
    /// `AdapterBenchmark.readAndWrite(true)`: reader into `ClassWriter(COMPUTE_MAXS)`.
    ReadWriteComputeMaxs,
    /// `AdapterBenchmark.readAndWriteWithComputeFrames`: reader into
    /// `ClassWriter(COMPUTE_FRAMES)`, whose common super classes come from the pack's hierarchy
    /// on both sides (ASM's benchmark loads classes instead).
    ReadWriteComputeFrames,
    /// `AdapterBenchmark.readAndWriteWithObjectModel`: reader into a `ClassNode`, then the node
    /// into `ClassWriter(0)`.
    ReadTreeWrite,
    /// `GeneratorBenchmark.asm`: [`HELLO_WORLDS_PER_PASS`] hello world classes with
    /// `COMPUTE_MAXS`.
    GenerateHelloWorld,
    /// `TypeBenchmark.getTypeFromDescriptor`: `Type::get_type` of every field and method
    /// descriptor the pack's classes use.
    TypeGet,
    /// `TypeBenchmark.getArgumentsAndReturnTypesFromDescriptor`: the argument and return types
    /// of every method descriptor, and the method type made of them.
    TypeArgumentsAndReturn,
    /// `TypeBenchmark.getArgumentsAndReturnSizeFromDescriptor`.
    TypeSizes,
}

impl Scenario {
    pub const ALL: [Scenario; 13] = [
        Scenario::ClassInfo,
        Scenario::Read,
        Scenario::ReadTree,
        Scenario::ReadTreeShared,
        Scenario::ReadWrite,
        Scenario::ReadWriteCopyPool,
        Scenario::ReadWriteComputeMaxs,
        Scenario::ReadWriteComputeFrames,
        Scenario::ReadTreeWrite,
        Scenario::GenerateHelloWorld,
        Scenario::TypeGet,
        Scenario::TypeArgumentsAndReturn,
        Scenario::TypeSizes,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Scenario::ClassInfo => "class_info",
            Scenario::Read => "read",
            Scenario::ReadTree => "read_tree",
            Scenario::ReadTreeShared => "read_tree_shared",
            Scenario::ReadWrite => "read_write",
            Scenario::ReadWriteCopyPool => "read_write_copy_pool",
            Scenario::ReadWriteComputeMaxs => "read_write_compute_maxs",
            Scenario::ReadWriteComputeFrames => "read_write_compute_frames",
            Scenario::ReadTreeWrite => "read_tree_write",
            Scenario::GenerateHelloWorld => "generate_hello_world",
            Scenario::TypeGet => "type_get",
            Scenario::TypeArgumentsAndReturn => "type_arguments_and_return",
            Scenario::TypeSizes => "type_sizes",
        }
    }

    pub fn from_name(name: &str) -> Option<Scenario> {
        Scenario::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The name of the oracle's scenario this one compares with.
    pub fn asm_name(self) -> &'static str {
        match self {
            Scenario::ReadTreeShared => Scenario::ReadTree.name(),
            other => other.name(),
        }
    }

    /// Whether the scenario computes frames, and so needs a class hierarchy.
    pub fn needs_hierarchy(self) -> bool {
        self == Scenario::ReadWriteComputeFrames
    }

    /// The number of operations of one pass over the fixture: classes, generated classes or
    /// descriptors. Times are reported per operation.
    pub fn ops(self, fixture: &Fixture) -> usize {
        match self {
            Scenario::GenerateHelloWorld => HELLO_WORLDS_PER_PASS,
            Scenario::TypeGet => fixture.descriptors.len() + fixture.method_descriptors.len(),
            Scenario::TypeArgumentsAndReturn | Scenario::TypeSizes => {
                fixture.method_descriptors.len()
            }
            _ => fixture.classes.len(),
        }
    }
}

/// What a scenario runs on: the classes of a pack, the hierarchy for computing frames, and the
/// descriptors of the `type_*` scenarios, collected from the classes as `TypeBenchmark` does.
#[derive(Debug)]
pub struct Fixture {
    pub classes: Vec<Vec<u8>>,
    pub hierarchy: Option<MapHierarchy>,
    descriptors: Vec<JavaString>,
    method_descriptors: Vec<JavaString>,
}

impl Fixture {
    /// The fixture of a pack, and of a hierarchy file in the format of the oracle's `hierarchy`
    /// command.
    ///
    /// # Errors
    ///
    /// If a file cannot be read, or the hierarchy or a class is malformed.
    pub fn load(pack: &Path, hierarchy: Option<&Path>) -> Result<Fixture, String> {
        let classes = read_pack(pack)?;
        let hierarchy = hierarchy.map(read_hierarchy).transpose()?;
        let mut fixture = Fixture {
            classes,
            hierarchy,
            descriptors: Vec::new(),
            method_descriptors: Vec::new(),
        };
        for (index, class) in fixture.classes.iter().enumerate() {
            let mut collect = CollectTypes {
                descriptors: &mut fixture.descriptors,
                method_descriptors: &mut fixture.method_descriptors,
            };
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut collect, ParsingOptions::empty()))
                .map_err(|e| format!("class {index} of {}: {e}", pack.display()))?;
        }
        Ok(fixture)
    }

    fn hierarchy(&self) -> Option<&dyn ClassHierarchy> {
        self.hierarchy.as_ref().map(|h| h as &dyn ClassHierarchy)
    }
}

/// Reads a hierarchy file, in the format of the oracle's `hierarchy` command.
///
/// # Errors
///
/// If the file cannot be read or is malformed.
pub fn read_hierarchy(path: &Path) -> Result<MapHierarchy, String> {
    let tsv = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    MapHierarchy::from_tsv(&tsv).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads a pack: the concatenation of `u32` big-endian length, class file bytes.
///
/// # Errors
///
/// If the file cannot be read or is truncated.
pub fn read_pack(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut classes = Vec::new();
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        let truncated = || format!("{} is truncated", path.display());
        let (length, tail) = rest.split_first_chunk::<4>().ok_or_else(truncated)?;
        let length = u32::from_be_bytes(*length) as usize;
        let class = tail.get(..length).ok_or_else(truncated)?;
        classes.push(class.to_vec());
        rest = &tail[length..];
    }
    Ok(classes)
}

/// Writes a pack (see [`read_pack`]).
///
/// # Errors
///
/// If the file cannot be written.
///
/// # Panics
///
/// If a class is 4 GiB or larger.
pub fn write_pack<'c>(
    path: &Path,
    classes: impl IntoIterator<Item = &'c [u8]>,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    for class in classes {
        let length = u32::try_from(class.len()).expect("a class file is smaller than 4 GiB");
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(class);
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Runs one pass of a scenario over the fixture, and returns its check value.
///
/// # Panics
///
/// If a class fails (packs are filtered with [`run_class`] beforehand).
pub fn run(scenario: Scenario, fixture: &Fixture) -> u64 {
    match scenario {
        Scenario::GenerateHelloWorld => (0..HELLO_WORLDS_PER_PASS)
            .map(|_| generate_hello_world().len() as u64)
            .sum(),
        Scenario::TypeGet => type_get(&fixture.descriptors, &fixture.method_descriptors),
        Scenario::TypeArgumentsAndReturn => type_arguments_and_return(&fixture.method_descriptors),
        Scenario::TypeSizes => type_sizes(&fixture.method_descriptors),
        Scenario::ReadTreeShared => {
            let interner = Arc::new(Interner::new());
            sum_classes(fixture, |class| read_tree(class, Some(&interner)))
        }
        _ => sum_classes(fixture, |class| {
            run_class(scenario, class, fixture.hierarchy())
        }),
    }
}

fn sum_classes(fixture: &Fixture, mut f: impl FnMut(&[u8]) -> Result<u64, String>) -> u64 {
    let mut check = 0u64;
    for (index, class) in fixture.classes.iter().enumerate() {
        match f(class) {
            Ok(value) => check = check.wrapping_add(value),
            Err(e) => panic!("class {index}: {e}"),
        }
    }
    check
}

/// Runs a scenario on one class, and returns the class's share of the check value. For the
/// `type_*` scenarios, the class's descriptors go through the scenario; `generate_hello_world`
/// ignores the class.
///
/// # Errors
///
/// If the class fails: the error, or the message of a panic of the `type_*` scenarios.
pub fn run_class(
    scenario: Scenario,
    class: &[u8],
    hierarchy: Option<&dyn ClassHierarchy>,
) -> Result<u64, String> {
    let read_error = |e: oxasm::ReadError| e.to_string();
    let write_error = |e: oxasm::WriteError| e.to_string();
    match scenario {
        Scenario::ClassInfo => {
            let reader = ClassReader::new(class).map_err(read_error)?;
            let access = reader.get_access().map_err(read_error)?;
            std::hint::black_box(reader.get_class_name().map_err(read_error)?);
            std::hint::black_box(reader.get_super_name().map_err(read_error)?);
            let interfaces = reader.get_interfaces().map_err(read_error)?;
            Ok(u64::from(access) + interfaces.len() as u64)
        }
        Scenario::Read => {
            let mut counter = Counter::default();
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut counter, ParsingOptions::empty()))
                .map_err(read_error)?;
            Ok(counter.count)
        }
        Scenario::ReadTree => read_tree(class, None),
        Scenario::ReadTreeShared => read_tree(class, Some(&Arc::new(Interner::new()))),
        Scenario::ReadWrite
        | Scenario::ReadWriteCopyPool
        | Scenario::ReadWriteComputeMaxs
        | Scenario::ReadWriteComputeFrames => {
            let reader = ClassReader::new(class).map_err(read_error)?;
            let mut class_writer = match scenario {
                Scenario::ReadWrite => ClassWriter::new(ClassWriterFlags::empty()),
                Scenario::ReadWriteCopyPool => {
                    ClassWriter::from_reader(&reader, ClassWriterFlags::empty())
                        .map_err(read_error)?
                }
                Scenario::ReadWriteComputeMaxs => ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS),
                _ => {
                    let class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_FRAMES);
                    match hierarchy {
                        Some(hierarchy) => class_writer.with_hierarchy(hierarchy),
                        None => class_writer,
                    }
                }
            };
            reader
                .accept(&mut class_writer, ParsingOptions::empty())
                .map_err(read_error)?;
            Ok(class_writer.to_bytes().map_err(write_error)?.len() as u64)
        }
        Scenario::ReadTreeWrite => {
            let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
            let mut class_node = ClassNode::with_interner(Arc::default());
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut class_node, ParsingOptions::empty()))
                .map_err(read_error)?;
            class_node.accept(&mut class_writer as &mut dyn ClassVisitor);
            Ok(class_writer.to_bytes().map_err(write_error)?.len() as u64)
        }
        Scenario::GenerateHelloWorld => Ok(generate_hello_world().len() as u64),
        Scenario::TypeGet | Scenario::TypeArgumentsAndReturn | Scenario::TypeSizes => {
            let mut descriptors = Vec::new();
            let mut method_descriptors = Vec::new();
            let mut collect = CollectTypes {
                descriptors: &mut descriptors,
                method_descriptors: &mut method_descriptors,
            };
            ClassReader::new(class)
                .and_then(|r| r.accept(&mut collect, ParsingOptions::empty()))
                .map_err(read_error)?;
            std::panic::catch_unwind(|| match scenario {
                Scenario::TypeGet => type_get(&descriptors, &method_descriptors),
                Scenario::TypeArgumentsAndReturn => type_arguments_and_return(&method_descriptors),
                _ => type_sizes(&method_descriptors),
            })
            .map_err(|panic| {
                panic
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_else(|| "panic".to_owned())
            })
        }
    }
}

/// The class read into a `ClassNode` with the given interner, or a new one. The check value
/// counts the fields, the methods and their instructions.
fn read_tree(class: &[u8], interner: Option<&Arc<Interner>>) -> Result<u64, String> {
    let mut class_node = ClassNode::with_interner(interner.cloned().unwrap_or_default());
    ClassReader::new(class)
        .and_then(|r| r.accept(&mut class_node, ParsingOptions::empty()))
        .map_err(|e| e.to_string())?;
    let instructions: usize = class_node
        .methods
        .iter()
        .map(|m| m.instructions.len())
        .sum();
    Ok((class_node.fields.len() + class_node.methods.len() + instructions) as u64)
}

/// `AsmGenerator.generateClass`.
fn generate_hello_world() -> Vec<u8> {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::COMPUTE_MAXS);
    class_writer.visit(
        V1_1,
        ACC_PUBLIC,
        jstr!("HelloWorld"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    class_writer.visit_source(Some(jstr!("HelloWorld.java")), None);
    {
        let mut mv = class_writer
            .visit_method(ACC_PUBLIC, jstr!("<init>"), jstr!("()V"), None, &[])
            .expect("a class writer visits methods");
        mv.visit_var_insn(ALOAD, 0);
        mv.visit_method_insn(
            INVOKESPECIAL,
            jstr!("java/lang/Object"),
            jstr!("<init>"),
            jstr!("()V"),
            false,
        );
        mv.visit_insn(RETURN);
        mv.visit_maxs(0, 0);
        mv.visit_end();
    }
    {
        let mut mv = class_writer
            .visit_method(
                ACC_PUBLIC | ACC_STATIC,
                jstr!("main"),
                jstr!("([Ljava/lang/String;)V"),
                None,
                &[],
            )
            .expect("a class writer visits methods");
        mv.visit_field_insn(
            GETSTATIC,
            jstr!("java/lang/System"),
            jstr!("out"),
            jstr!("Ljava/io/PrintStream;"),
        );
        mv.visit_ldc_insn(&Constant::String(jstr!("Hello world!").into()));
        mv.visit_method_insn(
            INVOKEVIRTUAL,
            jstr!("java/io/PrintStream"),
            jstr!("println"),
            jstr!("(Ljava/lang/String;)V"),
            false,
        );
        mv.visit_insn(RETURN);
        mv.visit_maxs(0, 0);
        mv.visit_end();
    }
    class_writer
        .to_bytes()
        .expect("the hello world class is valid")
}

/// The check value is the sum of the types' sorts.
fn type_get(descriptors: &[JavaString], method_descriptors: &[JavaString]) -> u64 {
    descriptors
        .iter()
        .chain(method_descriptors)
        .map(|d| Type::get_type(d.as_ref() as &JavaStr).get_sort() as u64)
        .sum()
}

/// The check value is the sum of the argument counts and return sorts.
fn type_arguments_and_return(method_descriptors: &[JavaString]) -> u64 {
    let mut check = 0;
    for descriptor in method_descriptors {
        let descriptor: &JavaStr = descriptor.as_ref();
        let argument_types = Type::get_argument_types_of(descriptor);
        let return_type = Type::get_return_type_of(descriptor);
        check += argument_types.len() as u64 + return_type.get_sort() as u64;
        std::hint::black_box(Type::get_method_type_of(&return_type, &argument_types));
    }
    check
}

/// The check value is the sum of the packed sizes.
fn type_sizes(method_descriptors: &[JavaString]) -> u64 {
    method_descriptors
        .iter()
        .map(|d| {
            u64::from(Type::get_arguments_and_return_sizes_of(
                d.as_ref() as &JavaStr
            ))
        })
        .sum()
}
