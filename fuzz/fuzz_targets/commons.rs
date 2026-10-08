//! Running a class through each oxasm-commons adapter into a `ClassWriter` never panics, hangs or
//! runs out of memory, except with the panics that stand for ASM's exceptions (`java.lang.*`,
//! ADR-06): `ClassRemapper` (with a remapper that renames every internal name), the
//! `SerialVersionUidAdder`, the `StaticInitMerger`, and every method in a `TryCatchBlockSorter`,
//! a `JsrInlinerAdapter`, a `LocalVariablesSorter`, an `AdviceAdapter`, an `InstructionAdapter`,
//! an `AnalyzerAdapter` and a `CodeSizeEvaluator` (oxasm-verification, "Fuzzing").
//!
//! Like ASM's, the JSR inliner duplicates each subroutine for every path that calls it, which is
//! exponential in the nesting of subroutines: classes with many `JSR`s skip it.

#![no_main]

use std::borrow::Cow;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Once};

use libfuzzer_sys::fuzz_target;
use oxasm::opcodes::{ACC_ABSTRACT, ACC_NATIVE, JSR};
use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, JavaString, MethodVisitor,
    ParsingOptions, Type, jstr,
};
use oxasm_commons::{
    Advice, AdviceAdapter, AnalyzerAdapter, ClassRemapper, CodeSizeEvaluator, InstructionAdapter,
    JsrInlinerAdapter, LocalVariablesSorter, Remapper, SerialVersionUidAdder, StaticInitMerger,
    TryCatchBlockSorter,
};
use oxasm_tree::{ClassNode, Insn};

/// The most `JSR` instructions in a class run through the JSR inliner.
const MAX_JSRS: usize = 8;

/// The message of a panic, if it is a string.
fn message<'a>(payload: &'a (dyn std::any::Any + Send)) -> Option<&'a str> {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
}

/// Runs `f`, and fails on any panic other than one that stands for ASM's exception.
fn adapting(f: impl FnOnce()) {
    if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(f)) {
        if !message(&*payload).is_some_and(|m| m.starts_with("java.lang.")) {
            panic::resume_unwind(payload);
        }
    }
}

/// Renames every internal name.
struct Rename;

impl Remapper for Rename {
    fn map<'a>(&'a self, internal_name: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
        let mut name = internal_name.to_owned();
        name.push_java_str(jstr!("$R"));
        Some(Cow::Owned(name))
    }
}

/// An advice that generates code on enter and on exit.
struct PushPop;

impl<M: MethodVisitor> Advice<M> for PushPop {
    fn on_method_enter(&mut self, adapter: &mut AdviceAdapter<M, Self>) {
        let local = adapter.new_local(&Type::INT);
        adapter.push_int(1);
        adapter.store_local(local);
    }

    fn on_method_exit(&mut self, adapter: &mut AdviceAdapter<M, Self>, opcode: u8) {
        adapter.push_int(i32::from(opcode));
        adapter.pop();
    }
}

/// The method adapters.
#[derive(Clone, Copy)]
enum Wrap {
    TcbSort,
    Jsr,
    Lvs,
    Advice,
    InstructionAdapter,
    AnalyzerAdapter,
    CodeSize,
}

/// A class visitor that wraps the method visitors of its delegate.
struct MethodAdapters<V> {
    class_visitor: V,
    wrap: Wrap,
    owner: JavaString,
}

impl<V: ClassVisitor> ClassVisitor for MethodAdapters<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.class_visitor)
    }

    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.owner = name.to_owned();
        self.class_visitor
            .visit(version, access, name, signature, super_name, interfaces);
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
            .class_visitor
            .visit_method(access, name, descriptor, signature, exceptions)?;
        Some(match self.wrap {
            Wrap::TcbSort => Box::new(TryCatchBlockSorter::new(
                Some(mv),
                access,
                name,
                descriptor,
                signature,
                exceptions,
            )),
            Wrap::Jsr => Box::new(JsrInlinerAdapter::new(
                Some(mv),
                access,
                name,
                descriptor,
                signature,
                exceptions,
            )),
            Wrap::Lvs => Box::new(LocalVariablesSorter::new(access, descriptor, mv)),
            Wrap::Advice if access & (ACC_ABSTRACT | ACC_NATIVE) == 0 => {
                Box::new(AdviceAdapter::new(mv, access, name, descriptor, PushPop))
            }
            Wrap::Advice => mv,
            Wrap::InstructionAdapter => Box::new(InstructionAdapter::new(mv)),
            Wrap::AnalyzerAdapter => Box::new(AnalyzerAdapter::new(
                &self.owner,
                access,
                name,
                descriptor,
                Some(mv),
            )),
            Wrap::CodeSize => Box::new(CodeSizeEvaluator::new(Some(mv))),
        })
    }
}

/// The number of `JSR` instructions in the class.
fn jsr_count(reader: &ClassReader<'_>) -> usize {
    let mut class_node = ClassNode::with_interner(Arc::default());
    let _ = reader.accept(&mut class_node, ParsingOptions::SKIP_DEBUG);
    class_node
        .methods
        .iter()
        .flat_map(|method| method.instructions.iter())
        .filter(|insn| matches!(&insn.insn, Insn::Jump { opcode: JSR, .. }))
        .count()
}

fn method_adapters<V: ClassVisitor>(class_visitor: V, wrap: Wrap) -> MethodAdapters<V> {
    MethodAdapters {
        class_visitor,
        wrap,
        owner: JavaString::new(),
    }
}

/// Writes the class that `accept` reads into a class writer with the given flags, if it reads
/// one.
fn write(flags: ClassWriterFlags, accept: impl FnOnce(&mut ClassWriter<'static>) -> bool) {
    let mut class_writer = ClassWriter::new(flags);
    if accept(&mut class_writer) {
        let _ = class_writer.to_bytes();
    }
}

static QUIET_ADAPTER_PANICS: Once = Once::new();

fuzz_target!(|data: &[u8]| {
    // libfuzzer's panic hook aborts: let the adapters' panics unwind (to `adapting`) quietly.
    QUIET_ADAPTER_PANICS.call_once(|| {
        let hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !message(info.payload()).is_some_and(|m| m.starts_with("java.lang.")) {
                hook(info);
            }
        }));
    });

    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    let none = ParsingOptions::empty();
    let flags = ClassWriterFlags::empty();
    adapting(|| {
        write(flags, |cw| {
            reader.accept(&mut ClassRemapper::new(cw, Rename), none).is_ok()
        })
    });
    adapting(|| {
        write(flags, |cw| {
            reader.accept(&mut SerialVersionUidAdder::new(cw), none).is_ok()
        })
    });
    adapting(|| {
        write(flags, |cw| {
            reader
                .accept(&mut StaticInitMerger::new(jstr!("clinit$"), cw), none)
                .is_ok()
        })
    });
    let wraps = [
        (Wrap::TcbSort, none),
        (Wrap::Lvs, ParsingOptions::EXPAND_FRAMES),
        (Wrap::Advice, ParsingOptions::EXPAND_FRAMES),
        (Wrap::InstructionAdapter, none),
        (Wrap::AnalyzerAdapter, ParsingOptions::EXPAND_FRAMES),
        (Wrap::CodeSize, none),
    ];
    for (wrap, options) in wraps {
        adapting(|| {
            write(flags, |cw| {
                reader.accept(&mut method_adapters(cw, wrap), options).is_ok()
            })
        });
    }
    if jsr_count(&reader) <= MAX_JSRS {
        adapting(|| {
            write(ClassWriterFlags::COMPUTE_MAXS, |cw| {
                reader.accept(&mut method_adapters(cw, Wrap::Jsr), none).is_ok()
            })
        });
    }
});
