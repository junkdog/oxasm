//! Port of ASM's `InstructionAdapterTest`.
//!
//! Dropped (each with its reason):
//!
//! - `testConstructor`: ASM checks that a subclass does not use the constructor without an
//!   `api`; there are no `api` versions (ADR-09) and no subclasses (ADR-13: a subclass is an
//!   `InstructionVisitor` in an `InstructionDispatcher`).
//! - `testDeprecatedVisitMethodInsn_illegalArgument`: the deprecated `visitMethodInsn` without
//!   `isInterface` does not exist (ADR-09).
//! - `testVisitLdcInsn_illegalArgument`: `visitLdcInsn(new Object())` cannot be written, as an
//!   `ldc` constant is a `Constant` (ADR-05).
//! - `testInvokeSpecial_unsupportedOperation`, `testInvokeVirtual_unsupportedOperation` and
//!   `testInvokeStatic_unsupportedOperation`: ASM4 api versions (ADR-09).
//!
//! `testDeprecatedInvokeSpecial`, `testDeprecatedInvokeVirtual` and `testDeprecatedInvokeStatic`
//! call the deprecated overloads without `isInterface`, which ASM's latest api turns into calls
//! with `isInterface = false`: here they call the methods with `false`.

mod common;

use common::{CodeComment, Comment, assert_same_class_file, to_text};
use oxasm::opcodes::*;
use oxasm::{
    AttributeCodec, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Constant, Handle,
    HandleKind, JavaStr, Label, MethodVisitor, ParsingOptions, Type, jstr,
};
use oxasm_commons::{InstructionAdapter, InstructionDispatcher, InstructionVisitor};
use oxasm_testkit::{PrecompiledClass, panic_message};
use oxasm_tree::MethodNode;
use oxasm_util::{Textifier, TraceMethodVisitor};

const ILLEGAL_ARGUMENT: &str = "java.lang.IllegalArgumentException";

fn new_adapter(method_node: &mut MethodNode) -> InstructionAdapter<&mut MethodNode> {
    InstructionAdapter::new(method_node)
}

#[test]
fn test_visit_insn_illegal_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| instruction_adapter.visit_insn(GOTO));

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

#[test]
fn test_visit_int_insn_illegal_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| instruction_adapter.visit_int_insn(GOTO, 0));

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

#[test]
fn test_visit_int_insn_illegal_new_array_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| instruction_adapter.visit_int_insn(NEWARRAY, 0));

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

#[test]
fn test_visit_var_insn_illegal_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| instruction_adapter.visit_var_insn(GOTO, 0));

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

#[test]
fn test_visit_type_insn_illegal_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| instruction_adapter.visit_type_insn(GOTO, jstr!("pkg/Class")));

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

#[test]
fn test_visit_field_insn_illegal_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| {
        instruction_adapter.visit_field_insn(
            INVOKEVIRTUAL,
            jstr!("pkg/Class"),
            jstr!("name"),
            jstr!("I"),
        );
    });

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

#[test]
fn test_visit_method_insn_illegal_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| {
        instruction_adapter.visit_method_insn(
            GETFIELD,
            jstr!("pkg/Class"),
            jstr!("name"),
            jstr!("I"),
            false,
        );
    });

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

#[test]
fn test_visit_jump_insn_illegal_argument() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    let message = panic_message(|| instruction_adapter.visit_jump_insn(NOP, Label::new()));

    assert_eq!(message, ILLEGAL_ARGUMENT);
}

/// ASM's `Boolean`, `Byte`, `Character` and `Short` constants are `Integer` constants here
/// (ADR-05), with the same `iconst` result.
#[test]
fn test_visit_ldc_insn() {
    let mut textifier = Textifier::new();
    {
        let mut instruction_adapter =
            InstructionAdapter::new(TraceMethodVisitor::new(&mut textifier));

        instruction_adapter.visit_ldc_insn(&Constant::Integer(0)); // Boolean.FALSE
        instruction_adapter.visit_ldc_insn(&Constant::Integer(1)); // Boolean.TRUE
        instruction_adapter.visit_ldc_insn(&Constant::Integer(2)); // Byte 2
        instruction_adapter.visit_ldc_insn(&Constant::Integer(i32::from(b'3'))); // Character '3'
        instruction_adapter.visit_ldc_insn(&Constant::Integer(4)); // Short 4
        instruction_adapter.visit_ldc_insn(&Constant::Integer(5));
        instruction_adapter.visit_ldc_insn(&Constant::Long(6));
        instruction_adapter.visit_ldc_insn(&Constant::Float(7.0));
        instruction_adapter.visit_ldc_insn(&Constant::Double(8.0));
        instruction_adapter.visit_ldc_insn(&Constant::String(jstr!("9").into()));
        instruction_adapter.visit_ldc_insn(&Constant::Type(Type::get_object_type("pkg/Class")));
        instruction_adapter.visit_ldc_insn(&Constant::Handle(Handle::new(
            HandleKind::GetField,
            "pkg/Class",
            "name",
            "I",
            /* is_interface= */ false,
        )));
    }

    let text: Vec<String> = textifier
        .text
        .iter()
        .map(|text| text.as_str().unwrap().to_string().trim().to_owned())
        .collect();
    assert_eq!(
        text.join(" "),
        "ICONST_0 ICONST_1 ICONST_2 BIPUSH 51 ICONST_4 ICONST_5 LDC 6L LDC 7.0F LDC 8.0D LDC \"9\" \
         LDC Lpkg/Class;.class // handle kind 0x1 : GETFIELD\n    LDC pkg/Class.name(I)"
    );
}

#[test]
fn test_deprecated_invoke_special() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    instruction_adapter.invokespecial(jstr!("pkg/Class"), jstr!("name"), jstr!("()V"), false);

    assert!(
        to_text(&method_node)
            .trim()
            .starts_with("INVOKESPECIAL pkg/Class.name ()V")
    );
}

#[test]
fn test_deprecated_invoke_virtual() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    instruction_adapter.invokevirtual(jstr!("pkg/Class"), jstr!("name"), jstr!("()V"), false);

    assert!(
        to_text(&method_node)
            .trim()
            .starts_with("INVOKEVIRTUAL pkg/Class.name ()V")
    );
}

#[test]
fn test_deprecated_invoke_static() {
    let mut method_node = MethodNode::default();
    let mut instruction_adapter = new_adapter(&mut method_node);

    instruction_adapter.invokestatic(jstr!("pkg/Class"), jstr!("name"), jstr!("()V"), false);

    assert!(
        to_text(&method_node)
            .trim()
            .starts_with("INVOKESTATIC pkg/Class.name ()V")
    );
}

fn attributes() -> [&'static dyn AttributeCodec; 2] {
    [&Comment, &CodeComment]
}

/// ASM's `InstructionClassAdapter`: wraps every method in an `InstructionAdapter` subclass
/// that overrides nothing (here, an `InstructionVisitor` in an `InstructionDispatcher`).
struct InstructionClassAdapter<V>(V);

/// ASM's anonymous `InstructionAdapter` subclass, which overrides nothing.
struct NoOverride<M>(M);

impl<M: MethodVisitor> InstructionVisitor for NoOverride<M> {
    fn mv(&mut self) -> &mut dyn MethodVisitor {
        &mut self.0
    }
}

impl<V: ClassVisitor> ClassVisitor for InstructionClassAdapter<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.0)
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let mv = self
            .0
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(Box::new(InstructionDispatcher::new(NoOverride(mv))))
    }
}

/// Tests that classes transformed with an InstructionAdapter are unchanged.
#[test]
fn test_all_methods_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        class_reader
            .accept_with_codecs(
                &mut InstructionClassAdapter(&mut class_writer),
                &attributes(),
                ParsingOptions::empty(),
            )
            .unwrap();

        assert_same_class_file(
            class_file,
            &class_writer.to_bytes().unwrap(),
            &class_parameter,
        );
    }
}

/// oxasm-specific: the same, with a plain `InstructionAdapter` (ASM's `InstructionAdapter`
/// constructed without subclassing).
#[test]
fn test_all_methods_precompiled_class_plain_adapter() {
    struct PlainAdapter<V>(V);

    impl<V: ClassVisitor> ClassVisitor for PlainAdapter<V> {
        fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
            Some(&mut self.0)
        }

        fn visit_method<'a>(
            &'a mut self,
            access: u32,
            name: &JavaStr,
            descriptor: &JavaStr,
            signature: Option<&JavaStr>,
            exceptions: &[&JavaStr],
        ) -> Option<Box<dyn MethodVisitor + 'a>> {
            let mv = self
                .0
                .visit_method(access, name, descriptor, signature, exceptions)?;
            Some(Box::new(InstructionAdapter::new(mv)))
        }
    }

    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());

        class_reader
            .accept_with_codecs(
                &mut PlainAdapter(&mut class_writer),
                &attributes(),
                ParsingOptions::empty(),
            )
            .unwrap();

        assert_same_class_file(
            class_file,
            &class_writer.to_bytes().unwrap(),
            &class_parameter,
        );
    }
}

/// oxasm-specific: an overridden typed method transforms the instructions it stands for, and
/// only those (ASM's subclassing of `InstructionAdapter`).
#[test]
fn test_overridden_typed_method() {
    struct AddToSub<M>(M);

    impl<M: MethodVisitor> InstructionVisitor for AddToSub<M> {
        fn mv(&mut self) -> &mut dyn MethodVisitor {
            &mut self.0
        }

        fn add(&mut self, type_: &Type) {
            self.sub(type_);
        }
    }

    let mut method_node = MethodNode::default();
    {
        let mut dispatcher = InstructionDispatcher::new(AddToSub(&mut method_node));
        dispatcher.visit_insn(IADD);
        dispatcher.visit_insn(LADD);
        dispatcher.visit_insn(IMUL);
        dispatcher.visit_ldc_insn(&Constant::Integer(1000));
        dispatcher.visit_ldc_insn(&Constant::Integer(100_000));
        dispatcher.visit_ldc_insn(&Constant::Float(2.0));
        dispatcher.visit_ldc_insn(&Constant::Double(-0.0));
    }

    assert_eq!(
        to_text(&method_node),
        "    ISUB\n    LSUB\n    IMUL\n    SIPUSH 1000\n    LDC 100000\n    FCONST_2\n    \
         LDC -0.0D\n    MAXSTACK = 0\n    MAXLOCALS = 0\n"
    );
}

/// oxasm-specific: the typed emitter, including `cast` and `newarray`.
#[test]
fn test_typed_methods() {
    let mut method_node = MethodNode::default();
    {
        let mut ia = new_adapter(&mut method_node);
        ia.iconst(-1);
        ia.iconst(-129);
        ia.lconst(1);
        ia.fconst(1.0);
        ia.dconst(1.0);
        ia.aconst(None);
        ia.load(3, &Type::DOUBLE);
        ia.store(4, &Type::get_object_type("pkg/Class"));
        ia.aload(&Type::BOOLEAN);
        ia.areturn(&Type::VOID);
        ia.cast(&Type::DOUBLE, &Type::BYTE);
        ia.cast(&Type::INT, &Type::INT);
        ia.newarray(&Type::LONG);
        ia.newarray(&Type::get_object_type("pkg/Class"));
        ia.cmpl(&Type::DOUBLE);
        ia.cmpg(&Type::FLOAT);
    }

    assert_eq!(
        to_text(&method_node),
        "    ICONST_M1\n    SIPUSH -129\n    LCONST_1\n    FCONST_1\n    DCONST_1\n    \
         ACONST_NULL\n    DLOAD 3\n    ASTORE 4\n    BALOAD\n    RETURN\n    D2I\n    I2B\n    \
         NEWARRAY T_LONG\n    ANEWARRAY pkg/Class\n    DCMPL\n    FCMPG\n    MAXSTACK = 0\n    MAXLOCALS = 0\n"
    );
}
