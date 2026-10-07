//! Port of ASM's `GeneratorAdapterTest`.
//!
//! ASM's `Generator` helper class, which wraps each code generation method to return the
//! Textifier text of what it generates, is [`Generator::run`], which takes the code generation
//! as a closure. ASM's `GeneratorException` (an `IllegalArgumentException` caught by the
//! helper) is a panic with ASM's exception.
//!
//! Dropped: `testConstructor_illegalState` (a subclass calling the public constructor throws an
//! `IllegalStateException`): there is no subclassing (ADR-13).

use oxasm::constant::Constant;
use oxasm::opcodes::*;
use oxasm::{ConstantDynamic, Handle, HandleKind, JavaStr, Label, Type, jstr};
use oxasm_commons::generator_adapter::{
    ADD, AND, DIV, MUL, NEG, OR, REM, SHL, SHR, SUB, USHR, XOR,
};
use oxasm_commons::generator_adapter::{EQ, GE, GT, LE, LT, NE};
use oxasm_commons::{GeneratorAdapter, Method};
use oxasm_testkit::{panic_message, panics};
use oxasm_tree::{ClassNode, MethodNode};
use oxasm_util::printer::text_to_java_string;
use oxasm_util::{Textifier, TraceMethodVisitor};

const OBJECT_TYPE: Type = Type::static_object_type(jstr!("java/lang/Object"));

type Adapter = GeneratorAdapter<TraceMethodVisitor<'static, Textifier>>;

/// ASM's `Generator` test helper: a generator adapter that prints what it generates.
struct Generator {
    generator_adapter: Adapter,
}

impl Generator {
    fn new() -> Generator {
        Generator::with(ACC_PUBLIC, "m", "()V")
    }

    fn with(access: u32, name: &str, descriptor: &str) -> Generator {
        Generator {
            generator_adapter: GeneratorAdapter::new(
                TraceMethodVisitor::new(Textifier::new()),
                access,
                jstr!(name),
                jstr!(descriptor),
            ),
        }
    }

    /// Runs `generate`, and returns the text of what it generated (ASM's `toString()`: the
    /// trimmed Textifier strings, joined with spaces), which is then cleared.
    fn run(&mut self, generate: impl FnOnce(&mut Adapter)) -> String {
        generate(&mut self.generator_adapter);
        let text = &mut self.generator_adapter.mv().p.text;
        let result = text
            .iter()
            .map(|node| {
                text_to_java_string(std::slice::from_ref(node))
                    .to_string()
                    .trim()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join(" ");
        text.clear();
        result
    }

    /// ASM's `tableSwitch`: the generator generates `push(key)` for each case, and `push(-1)`
    /// for the default case.
    fn table_switch(&mut self, keys: &[i32], use_table: Option<bool>) -> String {
        let mut switch_generator = (
            |g: &mut Adapter, key: i32, _end: Label| g.push_int(key),
            |g: &mut Adapter| g.push_int(-1),
        );
        self.run(|g| match use_table {
            None => g.table_switch(keys, &mut switch_generator),
            Some(use_table) => g.table_switch_with(keys, &mut switch_generator, use_table),
        })
    }
}

fn gen_(generate: impl FnOnce(&mut Adapter)) -> String {
    Generator::new().run(generate)
}

/// ASM's `assertThrows(GeneratorException.class, ...)`: an `IllegalArgumentException`.
#[track_caller]
fn assert_generator_exception(generate: impl FnOnce(&mut Adapter)) {
    let message = panic_message(|| gen_(generate));
    assert!(
        message.starts_with("java.lang.IllegalArgumentException"),
        "{message}"
    );
}

#[test]
fn test_constructor_empty_descriptor() {
    let generator_adapter = GeneratorAdapter::new(
        MethodNode::default(),
        ACC_PUBLIC,
        jstr!("name"),
        jstr!("()V"),
    );

    assert_eq!(generator_adapter.get_access(), ACC_PUBLIC);
    assert_eq!(generator_adapter.get_name(), "name");
    assert_eq!(*generator_adapter.get_return_type(), Type::VOID);
    assert_eq!(generator_adapter.get_argument_types(), &[] as &[Type]);
}

#[test]
fn test_constructor_basic_descriptor() {
    let generator_adapter = GeneratorAdapter::new(
        MethodNode::default(),
        ACC_PRIVATE,
        jstr!("m"),
        jstr!("(I)F"),
    );

    assert_eq!(generator_adapter.get_access(), ACC_PRIVATE);
    assert_eq!(generator_adapter.get_name(), "m");
    assert_eq!(*generator_adapter.get_return_type(), Type::FLOAT);
    assert_eq!(generator_adapter.get_argument_types(), [Type::INT]);
}

#[test]
fn test_constructor_with_class_visitor_and_exceptions() {
    let mut class_node = ClassNode::new();

    let generator_adapter = GeneratorAdapter::with_class_visitor(
        ACC_PUBLIC,
        &Method::new("name", "()V"),
        Some(jstr!("()V")),
        &[Type::get_object_type("java/lang/Exception")],
        &mut class_node,
    )
    .unwrap();

    assert_eq!(generator_adapter.get_access(), ACC_PUBLIC);
    assert_eq!(generator_adapter.get_name(), "name");
    assert_eq!(*generator_adapter.get_return_type(), Type::VOID);
    assert_eq!(generator_adapter.get_argument_types(), &[] as &[Type]);
    drop(generator_adapter);
    let method_node = &class_node.methods[0];
    assert_eq!(method_node.access, ACC_PUBLIC);
    assert_eq!(&*method_node.name, "name");
    assert_eq!(&*method_node.desc, "()V");
    let exceptions: Vec<&JavaStr> = method_node.exceptions.iter().map(|e| &**e).collect();
    assert_eq!(exceptions, ["java/lang/Exception"]);
}

#[test]
fn test_constructor_with_class_visitor_and_no_exceptions() {
    let mut class_node = ClassNode::new();

    let generator_adapter = GeneratorAdapter::with_class_visitor(
        ACC_PUBLIC,
        &Method::new("name", "()V"),
        Some(jstr!("()V")),
        &[],
        &mut class_node,
    )
    .unwrap();

    assert_eq!(generator_adapter.get_access(), ACC_PUBLIC);
    assert_eq!(generator_adapter.get_name(), "name");
    assert_eq!(*generator_adapter.get_return_type(), Type::VOID);
    assert_eq!(generator_adapter.get_argument_types(), &[] as &[Type]);
    drop(generator_adapter);
    let method_node = &class_node.methods[0];
    assert_eq!(method_node.access, ACC_PUBLIC);
    assert_eq!(&*method_node.name, "name");
    assert_eq!(&*method_node.desc, "()V");
    assert!(method_node.exceptions.is_empty());
}

#[test]
fn test_push_boolean() {
    assert_eq!(gen_(|g| g.push_bool(false)), "ICONST_0");
    assert_eq!(gen_(|g| g.push_bool(true)), "ICONST_1");
}

#[test]
fn test_push_int() {
    assert_eq!(gen_(|g| g.push_int(-32769)), "LDC -32769");
    assert_eq!(gen_(|g| g.push_int(-32768)), "SIPUSH -32768");
    assert_eq!(gen_(|g| g.push_int(-128)), "BIPUSH -128");
    assert_eq!(gen_(|g| g.push_int(-1)), "ICONST_M1");
    assert_eq!(gen_(|g| g.push_int(0)), "ICONST_0");
    assert_eq!(gen_(|g| g.push_int(1)), "ICONST_1");
    assert_eq!(gen_(|g| g.push_int(2)), "ICONST_2");
    assert_eq!(gen_(|g| g.push_int(3)), "ICONST_3");
    assert_eq!(gen_(|g| g.push_int(4)), "ICONST_4");
    assert_eq!(gen_(|g| g.push_int(5)), "ICONST_5");
    assert_eq!(gen_(|g| g.push_int(6)), "BIPUSH 6");
    assert_eq!(gen_(|g| g.push_int(127)), "BIPUSH 127");
    assert_eq!(gen_(|g| g.push_int(128)), "SIPUSH 128");
    assert_eq!(gen_(|g| g.push_int(32767)), "SIPUSH 32767");
    assert_eq!(gen_(|g| g.push_int(32768)), "LDC 32768");
}

#[test]
fn test_push_long() {
    assert_eq!(gen_(|g| g.push_long(0)), "LCONST_0");
    assert_eq!(gen_(|g| g.push_long(1)), "LCONST_1");
    assert_eq!(gen_(|g| g.push_long(2)), "LDC 2L");
}

#[test]
fn test_push_float() {
    assert_eq!(gen_(|g| g.push_float(0.0)), "FCONST_0");
    assert_eq!(gen_(|g| g.push_float(1.0)), "FCONST_1");
    assert_eq!(gen_(|g| g.push_float(2.0)), "FCONST_2");
    assert_eq!(gen_(|g| g.push_float(3.0)), "LDC 3.0F");
}

#[test]
fn test_push_double() {
    assert_eq!(gen_(|g| g.push_double(0.0)), "DCONST_0");
    assert_eq!(gen_(|g| g.push_double(1.0)), "DCONST_1");
    assert_eq!(gen_(|g| g.push_double(2.0)), "LDC 2.0D");
}

#[test]
fn test_push_string() {
    assert_eq!(gen_(|g| g.push_string(None)), "ACONST_NULL");
    assert_eq!(
        gen_(|g| g.push_string(Some(jstr!("string")))),
        "LDC \"string\""
    );
}

#[test]
fn test_push_type() {
    assert_eq!(gen_(|g| g.push_type(None)), "ACONST_NULL");
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::VOID))),
        "GETSTATIC java/lang/Void.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::BOOLEAN))),
        "GETSTATIC java/lang/Boolean.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::CHAR))),
        "GETSTATIC java/lang/Character.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::BYTE))),
        "GETSTATIC java/lang/Byte.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::SHORT))),
        "GETSTATIC java/lang/Short.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::INT))),
        "GETSTATIC java/lang/Integer.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::FLOAT))),
        "GETSTATIC java/lang/Float.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::LONG))),
        "GETSTATIC java/lang/Long.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::DOUBLE))),
        "GETSTATIC java/lang/Double.TYPE : Ljava/lang/Class;"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&OBJECT_TYPE))),
        "LDC Ljava/lang/Object;.class"
    );
    assert_eq!(
        gen_(|g| g.push_type(Some(&Type::get_object_type("[I")))),
        "LDC [I.class"
    );
}

fn getstatic_handle() -> Handle {
    Handle::new(HandleKind::GetStatic, "pkg/Owner", "name", "I", false)
}

#[test]
fn test_push_handle() {
    assert_eq!(gen_(|g| g.push_handle(None)), "ACONST_NULL");
    assert_eq!(
        gen_(|g| g.push_handle(Some(&getstatic_handle()))),
        "// handle kind 0x2 : GETSTATIC\n    LDC pkg/Owner.name(I)"
    );
}

#[test]
fn test_load_this() {
    assert_eq!(gen_(Adapter::load_this), "ALOAD 0");
}

#[test]
fn test_load_this_illegal_state() {
    let mut generator = Generator::with(ACC_STATIC, "m", "()V");

    let message = panic_message(|| generator.run(Adapter::load_this));

    assert!(message.starts_with("java.lang.IllegalStateException"));
}

#[test]
fn test_load_arg() {
    assert_eq!(
        Generator::with(ACC_PUBLIC, "m", "(I)V").run(|g| g.load_arg(0)),
        "ILOAD 1"
    );
    assert_eq!(
        Generator::with(ACC_STATIC, "m", "(J)V").run(|g| g.load_arg(0)),
        "LLOAD 0"
    );
    assert_eq!(
        Generator::with(ACC_STATIC, "m", "(JF)V").run(|g| g.load_arg(1)),
        "FLOAD 2"
    );
}

#[test]
fn test_load_args() {
    assert_eq!(
        Generator::with(ACC_PUBLIC, "m", "(IJFD)V").run(|g| g.load_args_range(1, 1)),
        "LLOAD 2"
    );
    assert_eq!(
        Generator::with(ACC_STATIC, "m", "(IJFD)V").run(Adapter::load_args),
        "ILOAD 0 LLOAD 1 FLOAD 3 DLOAD 4"
    );
}

#[test]
fn test_load_arg_array() {
    assert_eq!(
        Generator::with(ACC_PUBLIC, "m", "(ZBCSIJFDLjava/lang/Object;)V")
            .run(Adapter::load_arg_array),
        "BIPUSH 9 ANEWARRAY java/lang/Object \
         DUP ICONST_0 ILOAD 1 NEW java/lang/Boolean DUP_X1 SWAP \
         INVOKESPECIAL java/lang/Boolean.<init> (Z)V AASTORE \
         DUP ICONST_1 ILOAD 2 NEW java/lang/Byte DUP_X1 SWAP \
         INVOKESPECIAL java/lang/Byte.<init> (B)V AASTORE \
         DUP ICONST_2 ILOAD 3 NEW java/lang/Character DUP_X1 SWAP \
         INVOKESPECIAL java/lang/Character.<init> (C)V AASTORE \
         DUP ICONST_3 ILOAD 4 NEW java/lang/Short DUP_X1 SWAP \
         INVOKESPECIAL java/lang/Short.<init> (S)V AASTORE \
         DUP ICONST_4 ILOAD 5 NEW java/lang/Integer DUP_X1 SWAP \
         INVOKESPECIAL java/lang/Integer.<init> (I)V AASTORE \
         DUP ICONST_5 LLOAD 6 NEW java/lang/Long DUP_X2 DUP_X2 POP \
         INVOKESPECIAL java/lang/Long.<init> (J)V AASTORE \
         DUP BIPUSH 6 FLOAD 8 NEW java/lang/Float DUP_X1 SWAP \
         INVOKESPECIAL java/lang/Float.<init> (F)V AASTORE \
         DUP BIPUSH 7 DLOAD 9 NEW java/lang/Double DUP_X2 DUP_X2 POP \
         INVOKESPECIAL java/lang/Double.<init> (D)V AASTORE \
         DUP BIPUSH 8 ALOAD 11 AASTORE"
    );
}

#[test]
fn test_store_arg() {
    assert_eq!(
        Generator::with(ACC_PUBLIC, "m", "(I)V").run(|g| g.store_arg(0)),
        "ISTORE 1"
    );
    assert_eq!(
        Generator::with(ACC_STATIC, "m", "(J)V").run(|g| g.store_arg(0)),
        "LSTORE 0"
    );
    assert_eq!(
        Generator::with(ACC_STATIC, "m", "(JF)V").run(|g| g.store_arg(1)),
        "FSTORE 2"
    );
}

#[test]
fn test_new_local() {
    let mut generator = Generator::new();

    let local = generator.generator_adapter.new_local(&Type::FLOAT);

    assert_eq!(
        generator.generator_adapter.get_local_type(local),
        Some(Type::FLOAT)
    );
}

#[test]
fn test_load_local() {
    let mut generator = Generator::new();
    let local = generator.generator_adapter.new_local(&Type::FLOAT);

    let load_local = generator.run(|g| g.load_local(local));

    assert_eq!(load_local, "FLOAD 1");
}

#[test]
fn test_load_local_with_type() {
    let mut generator = Generator::new();
    let local = generator.generator_adapter.new_local(&Type::FLOAT);

    let load_local = generator.run(|g| g.load_local_with_type(local, &Type::INT));

    assert_eq!(load_local, "ILOAD 1");
    assert_eq!(
        generator.generator_adapter.get_local_type(local),
        Some(Type::INT)
    );
}

#[test]
fn test_store_local() {
    let mut generator = Generator::new();
    let local = generator.generator_adapter.new_local(&Type::FLOAT);

    let store_local = generator.run(|g| g.store_local(local));

    assert_eq!(store_local, "FSTORE 1");
}

#[test]
fn test_store_local_with_type() {
    let mut generator = Generator::new();
    let local = generator.generator_adapter.new_local(&Type::FLOAT);

    let store_local = generator.run(|g| g.store_local_with_type(local, &Type::INT));

    assert_eq!(store_local, "ISTORE 1");
    assert_eq!(
        generator.generator_adapter.get_local_type(local),
        Some(Type::INT)
    );
}

#[test]
fn test_array_load() {
    assert_eq!(gen_(|g| g.array_load(&Type::INT)), "IALOAD");
    assert_eq!(gen_(|g| g.array_load(&Type::LONG)), "LALOAD");
}

#[test]
fn test_array_store() {
    assert_eq!(gen_(|g| g.array_store(&Type::INT)), "IASTORE");
    assert_eq!(gen_(|g| g.array_store(&Type::LONG)), "LASTORE");
}

#[test]
fn test_pop() {
    assert_eq!(gen_(Adapter::pop), "POP");
}

#[test]
fn test_pop2() {
    assert_eq!(gen_(Adapter::pop2), "POP2");
}

#[test]
fn test_dup() {
    assert_eq!(gen_(Adapter::dup), "DUP");
}

#[test]
fn test_dup2() {
    assert_eq!(gen_(Adapter::dup2), "DUP2");
}

#[test]
fn test_dup_x1() {
    assert_eq!(gen_(Adapter::dup_x1), "DUP_X1");
}

#[test]
fn test_dup_x2() {
    assert_eq!(gen_(Adapter::dup_x2), "DUP_X2");
}

#[test]
fn test_dup2_x1() {
    assert_eq!(gen_(Adapter::dup2_x1), "DUP2_X1");
}

#[test]
fn test_dup2_x2() {
    assert_eq!(gen_(Adapter::dup2_x2), "DUP2_X2");
}

#[test]
fn test_swap() {
    assert_eq!(gen_(Adapter::swap), "SWAP");
    assert_eq!(gen_(|g| g.swap_types(&Type::INT, &Type::INT)), "SWAP");
    assert_eq!(
        gen_(|g| g.swap_types(&Type::LONG, &Type::INT)),
        "DUP_X2 POP"
    );
    assert_eq!(
        gen_(|g| g.swap_types(&Type::INT, &Type::LONG)),
        "DUP2_X1 POP2"
    );
    assert_eq!(
        gen_(|g| g.swap_types(&Type::LONG, &Type::LONG)),
        "DUP2_X2 POP2"
    );
}

#[test]
fn test_math() {
    assert_eq!(gen_(|g| g.math(ADD, &Type::INT)), "IADD");
    assert_eq!(gen_(|g| g.math(SUB, &Type::FLOAT)), "FSUB");
    assert_eq!(gen_(|g| g.math(MUL, &Type::LONG)), "LMUL");
    assert_eq!(gen_(|g| g.math(DIV, &Type::DOUBLE)), "DDIV");
    assert_eq!(gen_(|g| g.math(REM, &Type::INT)), "IREM");
    assert_eq!(gen_(|g| g.math(NEG, &Type::LONG)), "LNEG");
    assert_eq!(gen_(|g| g.math(SHL, &Type::INT)), "ISHL");
    assert_eq!(gen_(|g| g.math(SHR, &Type::LONG)), "LSHR");
    assert_eq!(gen_(|g| g.math(USHR, &Type::INT)), "IUSHR");
    assert_eq!(gen_(|g| g.math(AND, &Type::LONG)), "LAND");
    assert_eq!(gen_(|g| g.math(OR, &Type::INT)), "IOR");
    assert_eq!(gen_(|g| g.math(XOR, &Type::LONG)), "LXOR");
}

#[test]
fn test_not() {
    assert_eq!(gen_(Adapter::not), "ICONST_1 IXOR");
}

#[test]
fn test_iinc() {
    assert_eq!(gen_(|g| g.iinc(3, 5)), "IINC 3 5");
}

#[test]
fn test_cast() {
    let cast = |from: Type, to: Type| gen_(|g| g.cast(&from, &to));
    assert_eq!(cast(Type::DOUBLE, Type::DOUBLE), "");
    assert_eq!(cast(Type::DOUBLE, Type::FLOAT), "D2F");
    assert_eq!(cast(Type::DOUBLE, Type::LONG), "D2L");
    assert_eq!(cast(Type::DOUBLE, Type::INT), "D2I");
    assert_eq!(cast(Type::DOUBLE, Type::BYTE), "D2I I2B");
    assert_eq!(cast(Type::FLOAT, Type::DOUBLE), "F2D");
    assert_eq!(cast(Type::FLOAT, Type::FLOAT), "");
    assert_eq!(cast(Type::FLOAT, Type::LONG), "F2L");
    assert_eq!(cast(Type::FLOAT, Type::INT), "F2I");
    assert_eq!(cast(Type::FLOAT, Type::BYTE), "F2I I2B");
    assert_eq!(cast(Type::LONG, Type::DOUBLE), "L2D");
    assert_eq!(cast(Type::LONG, Type::FLOAT), "L2F");
    assert_eq!(cast(Type::LONG, Type::LONG), "");
    assert_eq!(cast(Type::LONG, Type::INT), "L2I");
    assert_eq!(cast(Type::LONG, Type::BYTE), "L2I I2B");
    assert_eq!(cast(Type::INT, Type::DOUBLE), "I2D");
    assert_eq!(cast(Type::INT, Type::FLOAT), "I2F");
    assert_eq!(cast(Type::INT, Type::LONG), "I2L");
    assert_eq!(cast(Type::INT, Type::INT), "");
    assert_eq!(cast(Type::INT, Type::BYTE), "I2B");
    assert_eq!(cast(Type::INT, Type::CHAR), "I2C");
    assert_eq!(cast(Type::INT, Type::SHORT), "I2S");
    assert_eq!(cast(Type::BYTE, Type::INT), "");
    assert_eq!(cast(Type::SHORT, Type::INT), "");
}

#[test]
fn test_cast_from_void() {
    assert_eq!(
        panic_message(|| gen_(|g| g.cast(&Type::VOID, &Type::INT))),
        "java.lang.IllegalArgumentException: Cannot cast from V to I"
    );
}

#[test]
fn test_cast_to_void() {
    assert_eq!(
        panic_message(|| gen_(|g| g.cast(&Type::INT, &Type::VOID))),
        "java.lang.IllegalArgumentException: Cannot cast from I to V"
    );
}

#[test]
fn test_box() {
    assert_eq!(gen_(|g| g.box_(&OBJECT_TYPE)), "");
    assert_eq!(gen_(|g| g.box_(&Type::get_object_type("[I"))), "");
    assert_eq!(gen_(|g| g.box_(&Type::VOID)), "ACONST_NULL");
    assert_eq!(
        gen_(|g| g.box_(&Type::BOOLEAN)),
        "NEW java/lang/Boolean DUP_X1 SWAP INVOKESPECIAL java/lang/Boolean.<init> (Z)V"
    );
    assert_eq!(
        gen_(|g| g.box_(&Type::BYTE)),
        "NEW java/lang/Byte DUP_X1 SWAP INVOKESPECIAL java/lang/Byte.<init> (B)V"
    );
    assert_eq!(
        gen_(|g| g.box_(&Type::CHAR)),
        "NEW java/lang/Character DUP_X1 SWAP INVOKESPECIAL java/lang/Character.<init> (C)V"
    );
    assert_eq!(
        gen_(|g| g.box_(&Type::SHORT)),
        "NEW java/lang/Short DUP_X1 SWAP INVOKESPECIAL java/lang/Short.<init> (S)V"
    );
    assert_eq!(
        gen_(|g| g.box_(&Type::INT)),
        "NEW java/lang/Integer DUP_X1 SWAP INVOKESPECIAL java/lang/Integer.<init> (I)V"
    );
    assert_eq!(
        gen_(|g| g.box_(&Type::LONG)),
        "NEW java/lang/Long DUP_X2 DUP_X2 POP INVOKESPECIAL java/lang/Long.<init> (J)V"
    );
    assert_eq!(
        gen_(|g| g.box_(&Type::FLOAT)),
        "NEW java/lang/Float DUP_X1 SWAP INVOKESPECIAL java/lang/Float.<init> (F)V"
    );
    assert_eq!(
        gen_(|g| g.box_(&Type::DOUBLE)),
        "NEW java/lang/Double DUP_X2 DUP_X2 POP INVOKESPECIAL java/lang/Double.<init> (D)V"
    );
}

#[test]
fn test_value_of() {
    assert_eq!(gen_(|g| g.value_of(&OBJECT_TYPE)), "");
    assert_eq!(gen_(|g| g.value_of(&Type::get_type("[I"))), "");
    assert_eq!(gen_(|g| g.value_of(&Type::VOID)), "ACONST_NULL");
    assert_eq!(
        gen_(|g| g.value_of(&Type::BOOLEAN)),
        "INVOKESTATIC java/lang/Boolean.valueOf (Z)Ljava/lang/Boolean;"
    );
    assert_eq!(
        gen_(|g| g.value_of(&Type::BYTE)),
        "INVOKESTATIC java/lang/Byte.valueOf (B)Ljava/lang/Byte;"
    );
    assert_eq!(
        gen_(|g| g.value_of(&Type::CHAR)),
        "INVOKESTATIC java/lang/Character.valueOf (C)Ljava/lang/Character;"
    );
    assert_eq!(
        gen_(|g| g.value_of(&Type::SHORT)),
        "INVOKESTATIC java/lang/Short.valueOf (S)Ljava/lang/Short;"
    );
    assert_eq!(
        gen_(|g| g.value_of(&Type::INT)),
        "INVOKESTATIC java/lang/Integer.valueOf (I)Ljava/lang/Integer;"
    );
    assert_eq!(
        gen_(|g| g.value_of(&Type::LONG)),
        "INVOKESTATIC java/lang/Long.valueOf (J)Ljava/lang/Long;"
    );
    assert_eq!(
        gen_(|g| g.value_of(&Type::FLOAT)),
        "INVOKESTATIC java/lang/Float.valueOf (F)Ljava/lang/Float;"
    );
    assert_eq!(
        gen_(|g| g.value_of(&Type::DOUBLE)),
        "INVOKESTATIC java/lang/Double.valueOf (D)Ljava/lang/Double;"
    );
}

#[test]
fn test_unbox() {
    assert_eq!(gen_(|g| g.unbox(&Type::VOID)), "");
    assert_eq!(
        gen_(|g| g.unbox(&Type::BOOLEAN)),
        "CHECKCAST java/lang/Boolean INVOKEVIRTUAL java/lang/Boolean.booleanValue ()Z"
    );
    assert_eq!(
        gen_(|g| g.unbox(&Type::BYTE)),
        "CHECKCAST java/lang/Number INVOKEVIRTUAL java/lang/Number.intValue ()I"
    );
    assert_eq!(
        gen_(|g| g.unbox(&Type::CHAR)),
        "CHECKCAST java/lang/Character INVOKEVIRTUAL java/lang/Character.charValue ()C"
    );
    assert_eq!(
        gen_(|g| g.unbox(&Type::SHORT)),
        "CHECKCAST java/lang/Number INVOKEVIRTUAL java/lang/Number.intValue ()I"
    );
    assert_eq!(
        gen_(|g| g.unbox(&Type::INT)),
        "CHECKCAST java/lang/Number INVOKEVIRTUAL java/lang/Number.intValue ()I"
    );
    assert_eq!(
        gen_(|g| g.unbox(&Type::LONG)),
        "CHECKCAST java/lang/Number INVOKEVIRTUAL java/lang/Number.longValue ()J"
    );
    assert_eq!(
        gen_(|g| g.unbox(&Type::FLOAT)),
        "CHECKCAST java/lang/Number INVOKEVIRTUAL java/lang/Number.floatValue ()F"
    );
    assert_eq!(
        gen_(|g| g.unbox(&Type::DOUBLE)),
        "CHECKCAST java/lang/Number INVOKEVIRTUAL java/lang/Number.doubleValue ()D"
    );
    assert_eq!(gen_(|g| g.unbox(&OBJECT_TYPE)), "");
    assert_eq!(
        gen_(|g| g.unbox(&Type::get_object_type("java/lang/Number"))),
        "CHECKCAST java/lang/Number"
    );
    assert_eq!(gen_(|g| g.unbox(&Type::get_type("[I"))), "CHECKCAST [I");
}

#[test]
fn test_if_cmp() {
    let if_cmp = |type_: Type, mode: u8| gen_(|g| g.if_cmp(&type_, mode, Label::new()));
    assert_eq!(if_cmp(Type::INT, EQ), "IF_ICMPEQ L0");
    assert_eq!(if_cmp(Type::INT, NE), "IF_ICMPNE L0");
    assert_eq!(if_cmp(Type::INT, GE), "IF_ICMPGE L0");
    assert_eq!(if_cmp(Type::INT, GT), "IF_ICMPGT L0");
    assert_eq!(if_cmp(Type::INT, LE), "IF_ICMPLE L0");
    assert_eq!(if_cmp(Type::INT, LT), "IF_ICMPLT L0");
    assert_eq!(if_cmp(Type::LONG, GE), "LCMP IFGE L0");
    assert_eq!(if_cmp(Type::FLOAT, GE), "FCMPL IFGE L0");
    assert_eq!(if_cmp(Type::FLOAT, GT), "FCMPL IFGT L0");
    assert_eq!(if_cmp(Type::FLOAT, LE), "FCMPG IFLE L0");
    assert_eq!(if_cmp(Type::FLOAT, LT), "FCMPG IFLT L0");
    assert_eq!(if_cmp(Type::DOUBLE, GE), "DCMPL IFGE L0");
    assert_eq!(if_cmp(Type::DOUBLE, GT), "DCMPL IFGT L0");
    assert_eq!(if_cmp(Type::DOUBLE, LE), "DCMPG IFLE L0");
    assert_eq!(if_cmp(Type::DOUBLE, LT), "DCMPG IFLT L0");
    assert_eq!(if_cmp(OBJECT_TYPE, EQ), "IF_ACMPEQ L0");
    assert_eq!(if_cmp(OBJECT_TYPE, NE), "IF_ACMPNE L0");
    assert_eq!(if_cmp(Type::get_type("[I"), EQ), "IF_ACMPEQ L0");
    assert_eq!(if_cmp(Type::get_type("[I"), NE), "IF_ACMPNE L0");
    assert_generator_exception(|g| g.if_cmp(&OBJECT_TYPE, GE, Label::new()));
    assert_generator_exception(|g| g.if_cmp(&Type::get_type("[I"), GE, Label::new()));
    assert_generator_exception(|g| g.if_cmp(&Type::INT, 0, Label::new()));
}

#[test]
fn test_mark() {
    assert_eq!(gen_(|g| g.mark(Label::new())), "L0");
}

#[test]
fn test_if_i_cmp() {
    let if_i_cmp = |mode: u8| gen_(|g| g.if_i_cmp(mode, Label::new()));
    assert_eq!(if_i_cmp(EQ), "IF_ICMPEQ L0");
    assert_eq!(if_i_cmp(NE), "IF_ICMPNE L0");
    assert_eq!(if_i_cmp(GE), "IF_ICMPGE L0");
    assert_eq!(if_i_cmp(GT), "IF_ICMPGT L0");
    assert_eq!(if_i_cmp(LE), "IF_ICMPLE L0");
    assert_eq!(if_i_cmp(LT), "IF_ICMPLT L0");
    assert_generator_exception(|g| g.if_i_cmp(0, Label::new()));
}

#[test]
fn test_if_z_cmp() {
    let if_z_cmp = |mode: u8| gen_(|g| g.if_z_cmp(mode, Label::new()));
    assert_eq!(if_z_cmp(EQ), "IFEQ L0");
    assert_eq!(if_z_cmp(NE), "IFNE L0");
    assert_eq!(if_z_cmp(GE), "IFGE L0");
    assert_eq!(if_z_cmp(GT), "IFGT L0");
    assert_eq!(if_z_cmp(LE), "IFLE L0");
    assert_eq!(if_z_cmp(LT), "IFLT L0");
}

#[test]
fn test_if_null() {
    assert_eq!(gen_(|g| g.if_null(Label::new())), "IFNULL L0");
}

#[test]
fn test_if_non_null() {
    assert_eq!(gen_(|g| g.if_non_null(Label::new())), "IFNONNULL L0");
}

#[test]
fn test_goto() {
    let mut generator = Generator::new();
    let label = generator.generator_adapter.new_label();

    let go_to = generator.run(|g| g.go_to(label));

    assert_eq!(go_to, "GOTO L0");
}

#[test]
fn test_table_switch() {
    assert_eq!(Generator::new().table_switch(&[], None), "L0 ICONST_M1 L1");
    assert_eq!(
        Generator::new().table_switch(&[0, 1], None),
        "TABLESWITCH\n      0: L0\n      1: L1\n      default: L2 L0 ICONST_0 L1 ICONST_1 L2 \
         ICONST_M1 L3"
    );
    assert_eq!(
        Generator::new().table_switch(&[0, 1], Some(false)),
        "LOOKUPSWITCH\n      0: L0\n      1: L1\n      default: L2 L0 ICONST_0 L1 ICONST_1 L2 \
         ICONST_M1 L3"
    );
    assert_eq!(
        Generator::new().table_switch(&[0, 4], None),
        "LOOKUPSWITCH\n      0: L0\n      4: L1\n      default: L2 L0 ICONST_0 L1 ICONST_4 L2 \
         ICONST_M1 L3"
    );
    assert_eq!(
        Generator::new().table_switch(&[0, 4], Some(true)),
        "TABLESWITCH\n      0: L0\n      1: L1\n      2: L1\n      3: L1\n      4: L2\n      \
         default: L1 L0 ICONST_0 L2 ICONST_4 L1 ICONST_M1 L3"
    );
    let message = panic_message(|| Generator::new().table_switch(&[1, 0], None));
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: keys must be sorted in ascending order"
    );
}

#[test]
fn test_ret() {
    assert_eq!(gen_(|g| g.ret(5)), "RET 5");
}

#[test]
fn test_return_value() {
    let return_value =
        |descriptor: &str| Generator::with(ACC_PUBLIC, "m", descriptor).run(Adapter::return_value);
    assert_eq!(return_value("()V"), "RETURN");
    assert_eq!(return_value("()Z"), "IRETURN");
    assert_eq!(return_value("()B"), "IRETURN");
    assert_eq!(return_value("()C"), "IRETURN");
    assert_eq!(return_value("()S"), "IRETURN");
    assert_eq!(return_value("()I"), "IRETURN");
    assert_eq!(return_value("()J"), "LRETURN");
    assert_eq!(return_value("()F"), "FRETURN");
    assert_eq!(return_value("()D"), "DRETURN");
    assert_eq!(return_value("()[I"), "ARETURN");
    assert_eq!(return_value("()Lpkg/Class"), "ARETURN");
}

fn pkg_class() -> Type {
    Type::get_object_type("pkg/Class")
}

#[test]
fn test_get_static() {
    assert_eq!(
        gen_(|g| g.get_static(&pkg_class(), jstr!("f"), &Type::INT)),
        "GETSTATIC pkg/Class.f : I"
    );
}

#[test]
fn test_put_static() {
    assert_eq!(
        gen_(|g| g.put_static(&pkg_class(), jstr!("f"), &Type::INT)),
        "PUTSTATIC pkg/Class.f : I"
    );
}

#[test]
fn test_get_field() {
    assert_eq!(
        gen_(|g| g.get_field(&pkg_class(), jstr!("f"), &Type::INT)),
        "GETFIELD pkg/Class.f : I"
    );
}

#[test]
fn test_put_field() {
    assert_eq!(
        gen_(|g| g.put_field(&pkg_class(), jstr!("f"), &Type::INT)),
        "PUTFIELD pkg/Class.f : I"
    );
}

#[test]
fn test_invoke_virtual() {
    assert_eq!(
        gen_(|g| g.invoke_virtual(&pkg_class(), &Method::new("m", "(I)J"))),
        "INVOKEVIRTUAL pkg/Class.m (I)J"
    );
}

#[test]
fn test_invoke_constructor() {
    assert_eq!(
        gen_(|g| g.invoke_constructor(&pkg_class(), &Method::new("<init>", "(I)J"))),
        "INVOKESPECIAL pkg/Class.<init> (I)J"
    );
}

#[test]
fn test_invoke_static() {
    assert_eq!(
        gen_(|g| g.invoke_static(&pkg_class(), &Method::new("m", "(I)J"))),
        "INVOKESTATIC pkg/Class.m (I)J"
    );
}

#[test]
fn test_invoke_interface() {
    assert_eq!(
        gen_(|g| g.invoke_interface(&pkg_class(), &Method::new("m", "(I)J"))),
        "INVOKEINTERFACE pkg/Class.m (I)J (itf)"
    );
}

#[test]
fn test_invoke_dynamic() {
    assert_eq!(
        gen_(|g| g.invoke_dynamic(
            jstr!("m"),
            jstr!("(I)J"),
            &getstatic_handle(),
            &[
                Constant::Integer(1),
                Constant::Integer(2),
                Constant::Integer(3)
            ],
        )),
        "INVOKEDYNAMIC m(I)J [\n      // handle kind 0x2 : GETSTATIC\n      pkg/Owner.name(I)\n      \
         // arguments:\n      1, \n      2, \n      3\n    ]"
    );
}

#[test]
fn test_constant_dynamic() {
    let constant_dynamic = ConstantDynamic::new(
        "name",
        "Ljava/lang/Object;",
        getstatic_handle(),
        vec![
            Constant::Integer(1),
            Constant::Integer(2),
            Constant::Integer(3),
        ],
    );
    assert_eq!(
        gen_(|g| g.push_constant_dynamic(Some(&constant_dynamic))),
        "LDC name : Ljava/lang/Object; [\n      // handle kind 0x2 : GETSTATIC\n      \
         pkg/Owner.name(I)\n      // arguments:\n      1, \n      2, \n      3\n    ]"
    );
}

#[test]
fn test_new_instance() {
    assert_eq!(gen_(|g| g.new_instance(&pkg_class())), "NEW pkg/Class");
}

#[test]
fn test_new_array() {
    assert_eq!(gen_(|g| g.new_array(&Type::BOOLEAN)), "NEWARRAY T_BOOLEAN");
    assert_eq!(gen_(|g| g.new_array(&Type::BYTE)), "NEWARRAY T_BYTE");
    assert_eq!(gen_(|g| g.new_array(&Type::CHAR)), "NEWARRAY T_CHAR");
    assert_eq!(gen_(|g| g.new_array(&Type::SHORT)), "NEWARRAY T_SHORT");
    assert_eq!(gen_(|g| g.new_array(&Type::INT)), "NEWARRAY T_INT");
    assert_eq!(gen_(|g| g.new_array(&Type::FLOAT)), "NEWARRAY T_FLOAT");
    assert_eq!(gen_(|g| g.new_array(&Type::LONG)), "NEWARRAY T_LONG");
    assert_eq!(gen_(|g| g.new_array(&Type::DOUBLE)), "NEWARRAY T_DOUBLE");
    assert_eq!(gen_(|g| g.new_array(&pkg_class())), "ANEWARRAY pkg/Class");
    assert_eq!(gen_(|g| g.new_array(&Type::get_type("[I"))), "ANEWARRAY [I");
}

#[test]
fn test_array_length() {
    assert_eq!(gen_(Adapter::array_length), "ARRAYLENGTH");
}

#[test]
fn test_throw_exception() {
    assert_eq!(gen_(Adapter::throw_exception), "ATHROW");
    assert_eq!(
        gen_(|g| g.throw_new_exception(&Type::get_object_type("pkg/Exception"), Some(jstr!("msg")))),
        "NEW pkg/Exception DUP LDC \"msg\" \
         INVOKESPECIAL pkg/Exception.<init> (Ljava/lang/String;)V ATHROW"
    );
}

#[test]
fn test_checkcast() {
    assert_eq!(gen_(|g| g.check_cast(&OBJECT_TYPE)), "");
    assert_eq!(gen_(|g| g.check_cast(&pkg_class())), "CHECKCAST pkg/Class");
}

#[test]
fn test_instance_of() {
    assert_eq!(
        gen_(|g| g.instance_of(&Type::get_type("Lpkg/Class;"))),
        "INSTANCEOF pkg/Class"
    );
}

#[test]
fn test_monitor_enter() {
    assert_eq!(gen_(Adapter::monitor_enter), "MONITORENTER");
}

#[test]
fn test_monitor_exit() {
    assert_eq!(gen_(Adapter::monitor_exit), "MONITOREXIT");
}

#[test]
fn test_end_method() {
    assert_eq!(gen_(Adapter::end_method), "MAXSTACK = 0 MAXLOCALS = 0");
    assert_eq!(
        Generator::with(ACC_ABSTRACT, "m", "()V").run(Adapter::end_method),
        ""
    );
}

#[test]
fn test_catch_exception() {
    assert_eq!(
        gen_(|g| g.catch_exception(Label::new(), Label::new(), None)),
        "TRYCATCHBLOCK L0 L1 L2 null L2"
    );
    assert_eq!(
        gen_(|g| g.catch_exception(
            Label::new(),
            Label::new(),
            Some(&Type::get_object_type("pkg/Exception"))
        )),
        "TRYCATCHBLOCK L0 L1 L2 pkg/Exception L2"
    );
}

/// oxasm-specific: the incoming visit events go through the local variables sorter, while the
/// generated code is emitted as is.
#[test]
fn test_visited_locals_are_remapped() {
    let mut generator = Generator::with(ACC_STATIC, "m", "(I)V");
    let local = generator.generator_adapter.new_local(&Type::LONG);
    assert_eq!(local, 1);
    assert!(panics(|| {
        Generator::with(ACC_STATIC, "m", "()V")
            .generator_adapter
            .get_local_type(5)
    }));

    let text = generator.run(|g| {
        use oxasm::MethodVisitor;
        g.visit_var_insn(ILOAD, 1);
        g.visit_iinc_insn(1, 1);
        g.load_local(local);
        g.visit_maxs(0, 0);
    });

    assert_eq!(text, "ILOAD 3 IINC 3 1 LLOAD 1 MAXSTACK = 0 MAXLOCALS = 4");
}
