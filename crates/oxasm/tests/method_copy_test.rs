//! The method copy optimization (ADR-08): a class writer built `from_reader` copies the methods
//! that reach it unchanged, and only those. `MethodVisitor::try_copy_method` is not forwarded
//! to `delegate()`, so a method adapter in front of the writer is never bypassed, even if it
//! overrides nothing. (oxasm-specific; ASM checks `methodVisitor instanceof MethodWriter`.)

use std::cell::Cell;
use std::rc::Rc;

use oxasm::opcodes::INVOKEVIRTUAL;
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, JavaString, MethodVisitor,
    ParsingOptions,
};
use oxasm_testkit::PrecompiledClass;

type Wrap = dyn for<'a> Fn(Box<dyn MethodVisitor + 'a>) -> Box<dyn MethodVisitor + 'a>;

/// A class adapter that wraps every method visitor of its downstream with `wrap`.
struct WrapMethods<'w, V> {
    next: V,
    wrap: &'w Wrap,
}

impl<V: ClassVisitor> ClassVisitor for WrapMethods<'_, V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.next)
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let method_visitor = self
            .next
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some((self.wrap)(method_visitor))
    }
}

/// A method adapter that overrides nothing but `visit_code` (to count the methods whose code
/// it sees): every event goes to its delegate, except the non-forwarding `try_copy_method`.
struct Transparent<'a> {
    next: Box<dyn MethodVisitor + 'a>,
    codes: Rc<Cell<usize>>,
}

impl MethodVisitor for Transparent<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.next)
    }

    fn visit_code(&mut self) {
        self.codes.set(self.codes.get() + 1);
        self.next.visit_code();
    }
}

/// A method adapter that renames the methods invoked with INVOKEVIRTUAL.
struct RenameInvocations<'a>(Box<dyn MethodVisitor + 'a>);

impl MethodVisitor for RenameInvocations<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.0)
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        let name = if opcode == INVOKEVIRTUAL {
            JavaStr::from_str("renamedByTheAdapter")
        } else {
            name
        };
        self.0
            .visit_method_insn(opcode, owner, name, descriptor, is_interface);
    }
}

/// Collects the names of the methods invoked with INVOKEVIRTUAL.
#[derive(Default)]
struct InvokedNames(Vec<JavaString>);

impl ClassVisitor for InvokedNames {
    fn visit_method<'a>(
        &'a mut self,
        _access: u32,
        _name: &JavaStr,
        _descriptor: &JavaStr,
        _signature: Option<&JavaStr>,
        _exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(&mut *self))
    }
}

impl MethodVisitor for InvokedNames {
    fn visit_method_insn(
        &mut self,
        opcode: u8,
        _owner: &JavaStr,
        name: &JavaStr,
        _descriptor: &JavaStr,
        _is_interface: bool,
    ) {
        if opcode == INVOKEVIRTUAL {
            self.0.push(name.to_owned());
        }
    }
}

/// The names of the methods invoked with INVOKEVIRTUAL in a class file.
fn invoked_names(class_file: &[u8]) -> Vec<JavaString> {
    let mut names = InvokedNames::default();
    ClassReader::new(class_file)
        .unwrap()
        .accept(&mut names, ParsingOptions::empty())
        .unwrap();
    names.0
}

/// The class file of `reader`, written by a `from_reader` class writer through `wrap`.
fn write_through(reader: &ClassReader<'_>, wrap: &Wrap) -> Vec<u8> {
    let class_writer = ClassWriter::from_reader(reader, ClassWriterFlags::empty()).unwrap();
    let mut adapter = WrapMethods {
        next: class_writer,
        wrap,
    };
    reader
        .accept(&mut adapter, ParsingOptions::empty())
        .unwrap();
    adapter.next.to_bytes().unwrap()
}

#[test]
fn transformations_are_never_bypassed() {
    let class_file = PrecompiledClass::Jdk8AllInstructions.bytes();
    let reader = ClassReader::new(class_file).unwrap();
    let original = invoked_names(class_file);
    assert!(!original.is_empty());
    // The adapter in front of the writer sees (and transforms) every method, although the
    // writer could copy them as is.
    let transformed = write_through(&reader, &|mv| Box::new(RenameInvocations(mv)));
    let names = invoked_names(&transformed);
    assert_eq!(names.len(), original.len());
    assert!(names.iter().all(|name| name == "renamedByTheAdapter"));
}

#[test]
fn an_adapter_that_overrides_nothing_disables_the_copy() {
    for &precompiled_class in PrecompiledClass::ALL {
        let class_file = precompiled_class.bytes();
        let Ok(reader) = ClassReader::new(class_file) else {
            continue;
        };
        // How many methods have code, as seen by a visitor in front of a new class writer.
        let expected = Rc::new(Cell::new(0));
        {
            let mut adapter = WrapMethods {
                next: ClassWriter::new(ClassWriterFlags::empty()),
                wrap: &{
                    let codes = expected.clone();
                    move |mv| {
                        Box::new(Transparent {
                            next: mv,
                            codes: codes.clone(),
                        })
                    }
                },
            };
            let _ = reader.accept(&mut adapter, ParsingOptions::empty());
        }
        // In front of a class writer that could copy the methods, the adapter still sees all
        // their code.
        let codes = Rc::new(Cell::new(0));
        {
            let class_writer =
                ClassWriter::from_reader(&reader, ClassWriterFlags::empty()).unwrap();
            let mut adapter = WrapMethods {
                next: class_writer,
                wrap: &{
                    let codes = codes.clone();
                    move |mv| {
                        Box::new(Transparent {
                            next: mv,
                            codes: codes.clone(),
                        })
                    }
                },
            };
            let _ = reader.accept(&mut adapter, ParsingOptions::empty());
        }
        assert_eq!(codes.get(), expected.get(), "{precompiled_class:?}");
    }
}
