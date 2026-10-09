//! oxasm's side of the analysis parity modes: the oracle's `analyze` output (see
//! `oracle/Oracle.java`), from `oxasm-analysis`.
//!
//! For each method: `name+descriptor`, then either `! <insn index> <message>` if the analysis
//! failed, or one `<insn index> <frame> | <insn text>` line per instruction, where `<frame>` is
//! ASM's `Frame.toString()` (`null` for unreachable instructions; source values print as
//! `<size>{<sorted insn indices>}`), and `<insn text>` the instruction printed by a Textifier
//! shared by the whole method, its lines trimmed and joined by a space.

use std::io::Write;

use oxasm::opcodes::ACC_INTERFACE;
use oxasm::{ClassHierarchy, ClassReader, JavaString, ParsingOptions, Type};
use oxasm_analysis::{
    Analyzer, BasicInterpreter, BasicValue, BasicVerifier, Interpreter, SimpleVerifier,
    SourceInterpreter, SourceValue,
};
use oxasm_tree::{ClassNode, InsnList, MethodNode};
use oxasm_util::printer::text_to_java_string;
use oxasm_util::{Printer, Textifier, TraceMethodVisitor};

/// A stock interpreter of the oracle's `analyze --interp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Basic,
    BasicVerifier,
    SimpleVerifier,
    Source,
}

/// The oracle's `analyze` output for a class, or the error that prevents it.
pub fn analyze(
    class_file: &[u8],
    interp: Interp,
    options: ParsingOptions,
    hierarchy: Option<&dyn ClassHierarchy>,
) -> Result<Vec<u8>, String> {
    let reader = ClassReader::new(class_file).map_err(|e| e.to_string())?;
    let mut class_node = crate::parity::class_node();
    reader
        .accept(&mut class_node, options)
        .map_err(|e| e.to_string())?;
    let mut text = Vec::new();
    for method in &class_node.methods {
        text.extend_from_slice(method.resolve(method.name).as_bytes());
        text.extend_from_slice(method.resolve(method.desc).as_bytes());
        text.push(b'\n');
        match interp {
            Interp::Basic => {
                analyze_method(&class_node, method, BasicInterpreter, basic, &mut text)?
            }
            Interp::BasicVerifier => {
                analyze_method(&class_node, method, BasicVerifier, basic, &mut text)?;
            }
            Interp::SimpleVerifier => {
                let hierarchy = hierarchy.ok_or("simpleverifier needs a hierarchy")?;
                let resolve = |sym| Type::get_object_type(class_node.resolve(sym));
                let verifier = SimpleVerifier::for_class(
                    resolve(class_node.name),
                    class_node.super_name.map(resolve),
                    class_node.interfaces.iter().copied().map(resolve).collect(),
                    class_node.access & ACC_INTERFACE != 0,
                    hierarchy,
                );
                analyze_method(&class_node, method, verifier, basic, &mut text)?;
            }
            Interp::Source => {
                let insns = &method.instructions;
                analyze_method(
                    &class_node,
                    method,
                    SourceInterpreter,
                    |value: &SourceValue, out: &mut Vec<u8>| source(insns, value, out),
                    &mut text,
                )?;
            }
        }
    }
    Ok(text)
}

fn basic(value: &BasicValue, out: &mut Vec<u8>) {
    out.extend_from_slice(value.to_java_string().as_bytes());
}

fn source(insns: &InsnList, value: &SourceValue, out: &mut Vec<u8>) {
    let mut indices: Vec<i64> = value
        .insns
        .iter()
        .map(|id| insns.index_of(id).map_or(-1, |i| i as i64))
        .collect();
    indices.sort_unstable();
    let indices: Vec<String> = indices.iter().map(i64::to_string).collect();
    write!(out, "{}{{{}}}", value.size, indices.join(",")).expect("writing to a vector");
}

fn analyze_method<I: Interpreter>(
    class_node: &ClassNode,
    method: &MethodNode,
    interpreter: I,
    value_format: impl Fn(&I::V, &mut Vec<u8>),
    text: &mut Vec<u8>,
) -> Result<(), String> {
    let mut analyzer = Analyzer::new(interpreter);
    let frames = match analyzer.analyze(class_node.resolve(class_node.name), method) {
        Ok(frames) => frames,
        // ASM's AssertionError is not a RuntimeException: it escapes its analyzer, and the
        // oracle fails for the whole class.
        Err(e)
            if e.message
                .as_bytes()
                .ends_with(b": java.lang.AssertionError") =>
        {
            return Err("java.lang.AssertionError: null".to_owned());
        }
        Err(e) => {
            let index = e
                .insn
                .and_then(|id| method.instructions.index_of(id))
                .map_or(-1, |i| i as i64);
            write!(text, "! {index} ").expect("writing to a vector");
            text.extend_from_slice(e.message.as_bytes());
            text.push(b'\n');
            return Ok(());
        }
    };
    let mut textifier = Textifier::new();
    let mut trace_method_visitor = TraceMethodVisitor::new(&mut textifier);
    for (i, id) in method.instructions.ids().enumerate() {
        let start = trace_method_visitor.p.get_text().len();
        method
            .instructions
            .accept_insn(id, method.interner(), &mut trace_method_visitor);
        let insn_text: JavaString =
            text_to_java_string(&trace_method_visitor.p.get_text()[start..]);
        write!(text, "{i} ").expect("writing to a vector");
        match frames.get(i) {
            None => text.extend_from_slice(b"null"),
            Some(frame) => {
                for value in frame.locals() {
                    value_format(value, text);
                }
                text.push(b' ');
                for value in frame.stack() {
                    value_format(value, text);
                }
            }
        }
        text.extend_from_slice(b" | ");
        let lines: Vec<&[u8]> = insn_text
            .as_bytes()
            .split(|&b| b == b'\n' || b == b'\r')
            .map(java_trim)
            .filter(|line| !line.is_empty())
            .collect();
        text.extend_from_slice(&lines.join(&b' '));
        text.push(b'\n');
    }
    Ok(())
}

/// Java's `String.trim()`: strips the characters up to U+0020 at both ends (in semi-UTF-8, these
/// are the bytes up to 0x20).
fn java_trim(line: &[u8]) -> &[u8] {
    let start = line.iter().position(|&b| b > b' ').unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|&b| b > b' ')
        .map_or(start, |i| i + 1);
    &line[start..end]
}
