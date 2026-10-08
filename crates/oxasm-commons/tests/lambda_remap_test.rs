//! Port of ASM's `LambdaRemapTest`.
//!
//! Its `LatestApi` tests run with both of ASM's remappers (an anonymous `Remapper` and a
//! `SimpleRemapper`). Its `DeprecatedApi` tests (`normalLambda`, `serializableLambda` and
//! `advancedLambda` with remappers created by ASM's deprecated constructors, which do not map
//! lambda names) are dropped: ADR-09, no `api` versions, so oxasm's remappers always behave as
//! ASM's latest api ones.

use std::borrow::Cow;

use oxasm::opcodes::*;
use oxasm::{ClassVisitor, Constant, Handle, HandleKind, JavaStr, MethodVisitor, Type, jstr};
use oxasm_commons::{ClassRemapper, Remapper, SimpleRemapper};
use oxasm_tree::{ClassNode, Insn};

/// The method signature of LambdaMetafactory.metafactory(...).
const LAMBDA_FACTORY_METAFACTORY: &str = "(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/invoke/MethodType;\
     Ljava/lang/invoke/MethodType;Ljava/lang/invoke/MethodHandle;\
     Ljava/lang/invoke/MethodType;)Ljava/lang/invoke/CallSite;";

/// The method signature of LambdaMetafactory.altMetafactory(...).
const LAMBDA_FACTORY_ALTMETAFACTORY: &str = "(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/invoke/MethodType;\
     [Ljava/lang/Object;)Ljava/lang/invoke/CallSite;";

// The 3 following functions correspond to the following Java code:
//
//  public class LambdaTest {
//    public interface TestInterface {
//      default void helloWorld() {}
//    }
//
//    public static void normalLambda() {
//      Runnable runnable = Thread::dumpStack;
//    }
//
//    public static void serializableLambda() {
//      Runnable runnable = (Runnable & Serializable) Thread::dumpStack;
//    }
//
//    public static void advancedLambda() {
//      Runnable runnable = (Runnable & Serializable & TestInterface) Thread::dumpStack;
//    }
//  }

fn dump_stack() -> Constant<'static> {
    Constant::Handle(Handle::new(
        HandleKind::InvokeStatic,
        "java/lang/Thread",
        "dumpStack",
        "()V",
        false,
    ))
}

fn func_normal_lambda(method_visitor: &mut dyn MethodVisitor) {
    method_visitor.visit_invoke_dynamic_insn(
        jstr!("run"),
        jstr!("()Ljava/lang/Runnable;"),
        &Handle::new(
            HandleKind::InvokeStatic,
            "java/lang/invoke/LambdaMetafactory",
            "metafactory",
            LAMBDA_FACTORY_METAFACTORY,
            false,
        ),
        &[
            Constant::Type(Type::get_type("()V")),
            dump_stack(),
            Constant::Type(Type::get_type("()V")),
        ],
    );
}

fn func_serializable_lambda(method_visitor: &mut dyn MethodVisitor) {
    method_visitor.visit_invoke_dynamic_insn(
        jstr!("run"),
        jstr!("()Ljava/lang/Runnable;"),
        &Handle::new(
            HandleKind::InvokeStatic,
            "java/lang/invoke/LambdaMetafactory",
            "altMetafactory",
            LAMBDA_FACTORY_ALTMETAFACTORY,
            false,
        ),
        &[
            Constant::Type(Type::get_type("()V")),
            dump_stack(),
            Constant::Type(Type::get_type("()V")),
            Constant::Integer(5),
            Constant::Integer(0),
        ],
    );
}

fn func_advanced_lambda(method_visitor: &mut dyn MethodVisitor) {
    method_visitor.visit_invoke_dynamic_insn(
        jstr!("run"),
        jstr!("()Ljava/lang/Runnable;"),
        &Handle::new(
            HandleKind::InvokeStatic,
            "java/lang/invoke/LambdaMetafactory",
            "altMetafactory",
            LAMBDA_FACTORY_ALTMETAFACTORY,
            false,
        ),
        &[
            Constant::Type(Type::get_type("()V")),
            dump_stack(),
            Constant::Type(Type::get_type("()V")),
            Constant::Integer(7),
            Constant::Integer(1),
            Constant::Type(Type::get_type("Lpkg/LambdaTest$TestInterface;")),
            Constant::Integer(0),
        ],
    );
}

/// ASM's anonymous `Remapper` of `remappersLatest()`.
struct RunnableRemapper;

impl Remapper for RunnableRemapper {
    fn map_method_name<'a>(
        &'a self,
        owner: &'a JavaStr,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        if owner == "java/lang/Runnable" && name == "run" {
            return Cow::Borrowed(jstr!("call"));
        }
        Cow::Borrowed(name)
    }

    fn map<'a>(&'a self, internal_name: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
        if internal_name == "java/lang/Runnable" {
            return Some(Cow::Borrowed(jstr!("me/MyRunnable")));
        }
        Some(Cow::Borrowed(internal_name))
    }
}

/// ASM's `remappersLatest()`.
fn remappers_latest() -> Vec<Box<dyn Remapper>> {
    vec![
        Box::new(RunnableRemapper),
        Box::new(SimpleRemapper::from_iter([
            ("java/lang/Runnable", "me/MyRunnable"),
            ("java/lang/Runnable.run()V", "call"),
        ])),
    ]
}

/// Visits a method `hello` with the given lambda through a class remapper, and checks the
/// remapped invokedynamic instruction.
fn assert_lambda_remapped(func: fn(&mut dyn MethodVisitor)) {
    for remapper in remappers_latest() {
        let mut class_node = ClassNode::new();
        let mut class_remapper = ClassRemapper::new(&mut class_node, &*remapper);
        class_remapper.visit(
            V11,
            ACC_PUBLIC,
            jstr!("C"),
            None,
            Some(jstr!("java/lang/Object")),
            &[],
        );
        let mut method_visitor = class_remapper
            .visit_method(ACC_PUBLIC, jstr!("hello"), jstr!("()V"), None, &[])
            .unwrap();
        method_visitor.visit_code();

        func(&mut *method_visitor);
        drop(method_visitor);
        drop(class_remapper);

        let Insn::InvokeDynamic(invoke_dynamic) =
            &class_node.methods[0].instructions.get(0).unwrap().insn
        else {
            panic!("an invokedynamic instruction");
        };
        assert_eq!(class_node.resolve(invoke_dynamic.desc), "()Lme/MyRunnable;");
        assert_eq!(class_node.resolve(invoke_dynamic.name), "call");
    }
}

#[test]
fn latest_api_normal_lambda() {
    assert_lambda_remapped(func_normal_lambda);
}

#[test]
fn latest_api_serializable_lambda() {
    assert_lambda_remapped(func_serializable_lambda);
}

#[test]
fn latest_api_advanced_lambda() {
    assert_lambda_remapped(func_advanced_lambda);
}
