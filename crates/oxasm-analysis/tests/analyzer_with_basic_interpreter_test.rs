//! Port of ASM's `AnalyzerWithBasicInterpreterTest`: unit tests for `Analyzer`, when used with a
//! `BasicInterpreter`.
//!
//! - `testConstructor` keeps its non-throwing half: there is no subclassing, so the
//!   `IllegalStateException` of `new BasicInterpreter() {}` cannot be written (ADR-09).
//! - `testAnalyze_basicInterpreter` subclasses `Analyzer` to use custom frames. There is no
//!   `newFrame` hook (a `Frame` subclass overrides `initJumpTarget`, which is
//!   `AnalyzerHooks::init_jump_target` here): the port checks that this hook is called instead of
//!   `frame instanceof CustomFrame`.
//! - The `Api` axis of the parameterized tests is dropped (ADR-09): they loop over
//!   `PrecompiledClass::ALL`.

mod common;

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use common::MethodNodeBuilder;
use oxasm::opcodes::*;
use oxasm::{ClassReader, Label, ParsingOptions, Type, jstr};
use oxasm_analysis::{
    Analyzer, AnalyzerError, AnalyzerHooks, BasicInterpreter, BasicValue, Frame, InsnRef,
    Interpreter,
};
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::{ClassNode, MethodNode, TryCatchBlockNode};

const CLASS_NAME: &str = "C";

fn read_class_node(bytes: &[u8]) -> ClassNode {
    let mut class_node = ClassNode::with_interner(Arc::default());
    ClassReader::new(bytes)
        .unwrap()
        .accept(&mut class_node, ParsingOptions::empty())
        .unwrap();
    class_node
}

#[test]
fn test_constructor() {
    let _ = BasicInterpreter::new();
    // ASM: assertThrows(IllegalStateException.class, () -> new BasicInterpreter() {}) - dropped,
    // there is no subclassing (ADR-09).
}

#[test]
fn test_analyze_invalid_new_array() {
    let method_node = MethodNodeBuilder::new()
        .iconst_0()
        .int_insn(NEWARRAY, -1)
        .vreturn()
        .build();

    let analyze = Analyzer::new(BasicInterpreter::new())
        .analyze(jstr!(CLASS_NAME), &method_node)
        .map(|_| ());

    let message = analyze.unwrap_err().message.to_string();
    assert!(message.contains("Invalid array type"), "{message}");
}

/// The hooks of ASM's analyzer subclass with custom frames: the behavior a `CustomFrame` would
/// override is `initJumpTarget`, whose calls are counted.
#[derive(Default)]
struct CustomFrameHooks {
    init_jump_target_calls: usize,
}

impl AnalyzerHooks<BasicValue> for CustomFrameHooks {
    fn init_jump_target(
        &mut self,
        _frame: &mut Frame<BasicValue>,
        _opcode: u8,
        _target: Option<Label>,
    ) {
        self.init_jump_target_calls += 1;
    }
}

/// Tests that the precompiled classes can be successfully analyzed with a BasicInterpreter, and
/// that Analyzer can be subclassed to use custom frames.
#[test]
fn test_analyze_basic_interpreter() {
    let mut init_jump_target_calls = 0;
    for &class_parameter in PrecompiledClass::ALL {
        let class_node = read_class_node(class_parameter.bytes());
        let mut analyzer =
            Analyzer::with_hooks(BasicInterpreter::new(), CustomFrameHooks::default());

        let mut method_frames: Vec<Vec<Option<Frame<BasicValue>>>> = Vec::new();
        for method_node in &class_node.methods {
            let frames = analyzer
                .analyze(class_node.resolve(class_node.name), method_node)
                .unwrap_or_else(|e| {
                    panic!(
                        "{class_parameter} {}: {e}",
                        method_node.resolve(method_node.name)
                    )
                })
                .to_vec();
            method_frames.push(frames);
        }

        // ASM: assertTrue(frame == null || frame instanceof CustomFrame) for each frame. There
        // are no frame subclasses here: the custom frame behavior is the init_jump_target hook.
        init_jump_target_calls += analyzer.hooks().init_jump_target_calls;
        assert_eq!(method_frames.len(), class_node.methods.len());
    }
    assert!(init_jump_target_calls > 0);
}

/// Tests that the precompiled classes can be successfully analyzed with a BasicInterpreter, even
/// if the method node's max locals and max stack size are not set.
#[test]
fn test_analyze_and_compute_maxs_basic_interpreter() {
    for &class_parameter in PrecompiledClass::ALL {
        let mut class_node = read_class_node(class_parameter.bytes());
        let mut method_maxs: Vec<(u16, u16)> = Vec::new();
        for method_node in &mut class_node.methods {
            method_maxs.push((method_node.max_stack, method_node.max_locals));
            method_node.max_locals = 0;
            method_node.max_stack = 0;
        }
        let mut analyzer = Analyzer::new(BasicInterpreter::new());

        let mut analyzed_method_maxs: Vec<(u16, u16)> = Vec::new();
        let interner = class_node.interner().clone();
        let owner = interner.resolve(class_node.name);
        for method_node in &mut class_node.methods {
            analyzer
                .analyze_and_compute_maxs(owner, method_node)
                .unwrap_or_else(|e| {
                    panic!(
                        "{class_parameter} {}: {e}",
                        interner.resolve(method_node.name)
                    )
                });
            analyzed_method_maxs.push((method_node.max_stack, method_node.max_locals));
        }

        // jdk3.SubOptimalMaxStackAndLocals has non optimal max stack and max local values on
        // purpose.
        if class_parameter == PrecompiledClass::Jdk3SubOptimalMaxStackAndLocals {
            continue;
        }
        for (analyzed, original) in analyzed_method_maxs.iter().zip(&method_maxs) {
            assert!(analyzed.1 >= original.1, "{class_parameter}: max locals");
            assert!(analyzed.0 >= original.0, "{class_parameter}: max stack");
        }
    }
}

/// Tests that analyzeAndComputeMaxs computes the smallest possible maxLocals for static methods.
#[test]
fn test_analyze_and_compute_maxs_static_method() {
    let mut method_node = MethodNodeBuilder::with_descriptor(
        "(I)V", /* max_stack= */ 0, /* max_locals= */ 0,
    )
    .vreturn()
    .build();
    method_node.access |= ACC_STATIC;
    let mut analyzer = Analyzer::new(BasicInterpreter::new());

    analyzer
        .analyze_and_compute_maxs(jstr!("C"), &mut method_node)
        .unwrap();

    assert_eq!(1, method_node.max_locals);
    assert_eq!(0, method_node.max_stack);
}

/// A BasicInterpreter whose merge does not follow its required contract: it always returns a new
/// value, even when it is equal to the first argument (ASM's anonymous subclass, by delegation).
struct BadInterpreter(BasicInterpreter);

impl Interpreter for BadInterpreter {
    type V = BasicValue;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<BasicValue>, AnalyzerError> {
        self.0.new_value(ty)
    }
    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<BasicValue>, AnalyzerError> {
        self.0.new_operation(insn)
    }
    fn new_exception_value(
        &mut self,
        try_catch_block: &TryCatchBlockNode,
        handler_frame: &Frame<BasicValue>,
        exception_type: &Type,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        self.0
            .new_exception_value(try_catch_block, handler_frame, exception_type)
    }
    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<BasicValue, AnalyzerError> {
        self.0.copy_operation(insn, value)
    }
    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        self.0.unary_operation(insn, value)
    }
    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        self.0.binary_operation(insn, value1, value2)
    }
    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
        value3: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        self.0.ternary_operation(insn, value1, value2, value3)
    }
    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[BasicValue],
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        self.0.nary_operation(insn, values)
    }
    fn return_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
        expected: Option<&BasicValue>,
    ) -> Result<(), AnalyzerError> {
        self.0.return_operation(insn, value, expected)
    }
    fn merge(
        &mut self,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        // ASM: return new BasicValue(super.merge(value1, value2).getType()). A `None` merge is
        // value1: a new value is returned in its place, which breaks the contract.
        let merged = self.0.merge(value1, value2)?.unwrap_or(*value1);
        Ok(Some(BasicValue::new(merged.get_type().cloned())))
    }
}

/// Tests that the analyzer does not loop infinitely, even if the {@link Interpreter#merge} method
/// does not follow its required contract (namely that if the merge result is equal to the first
/// argument, the first argument should be returned - see #316326).
#[test]
fn test_analyze_bad_interpreter() {
    let class_node = read_class_node(PrecompiledClass::Jdk8AllFrames.bytes());
    let methods: Vec<MethodNode> = class_node.methods.clone();
    let method_count = methods.len();

    // ASM: assertTimeoutPreemptively(ofSeconds(1), analysis) for each method. The analyses run
    // on another thread, which reports each one; the timeout is larger than ASM's for
    // unoptimized builds.
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut analyzer = Analyzer::new(BadInterpreter(BasicInterpreter::new()));
        for method_node in &methods {
            let result = analyzer.analyze(jstr!(CLASS_NAME), method_node).map(|_| ());
            if sender.send(result).is_err() {
                return;
            }
        }
    });

    for _ in 0..method_count {
        let result = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the analysis does not terminate");
        result.unwrap();
    }
}

/// Tests that stack map frames are correctly merged when a JSR instruction can be reached from two
/// different control flow paths, with different local variable types (#316204).
#[test]
fn test_analyze_merge_with_jsr_reachable_from_two_different_paths() {
    let class_node = read_class_node(&oxasm_testkit::t0_resource(
        "asm-analysis",
        "Issue316204.class",
    ));
    let mut analyzer = Analyzer::new(BasicInterpreter::new());

    analyzer
        .analyze(
            class_node.resolve(class_node.name),
            get_method(&class_node, "basicStopBundles"),
        )
        .unwrap();

    assert_eq!(
        "RIR..... ",
        analyzer.get_frames()[104].as_ref().unwrap().to_string()
    );
}

fn get_method<'a>(class_node: &'a ClassNode, name: &str) -> &'a MethodNode {
    class_node
        .methods
        .iter()
        .find(|method_node| method_node.resolve(method_node.name) == name)
        .unwrap()
}

// -------------------------------------------------------------------------------------------------
// Not in ASM: the frames of an analysis share one allocation (optimization lead A1), which a frame
// changed after the analysis must not change for the others.
// -------------------------------------------------------------------------------------------------

/// The printed frames, `None` for the instructions without one.
fn printed(frames: &[Option<Frame<BasicValue>>]) -> Vec<Option<String>> {
    frames
        .iter()
        .map(|frame| frame.as_ref().map(ToString::to_string))
        .collect()
}

#[test]
fn frames_changed_after_the_analysis_keep_the_others_unchanged() {
    let method_node = MethodNodeBuilder::with_maxs(3, 3)
        .iconst_0()
        .istore(1)
        .iconst_0()
        .iconst_0()
        .pop()
        .pop()
        .vreturn()
        .build();
    let mut analyzer = Analyzer::new(BasicInterpreter::new());
    analyzer.analyze(jstr!(CLASS_NAME), &method_node).unwrap();
    let frames = analyzer.take_frames();
    let expected = printed(&frames);
    assert_eq!(expected[4].as_deref(), Some("RI. II"), "{expected:?}");

    let changes: [fn(&mut Frame<BasicValue>); 6] = [
        |frame| frame.set_local(0, BasicValue::INT_VALUE),
        |frame| frame.push(BasicValue::FLOAT_VALUE),
        |frame| {
            frame.pop();
        },
        |frame| frame.clear_stack(),
        |frame| frame.set_stack(0, BasicValue::FLOAT_VALUE),
        |frame| {
            let mut other = frame.clone();
            other.set_local(0, BasicValue::INT_VALUE);
            assert!(frame.merge(&other, &mut BasicInterpreter::new()).unwrap());
        },
    ];
    for (index, change) in changes.iter().enumerate() {
        // A frame of the analysis, and clones of it made in each way, changed one at a time.
        let mut changed_frames = frames.clone();
        let mut clone = frames[4].clone().unwrap();
        let mut cloned_from = Frame::new(Vec::new(), None);
        cloned_from.clone_from(frames[4].as_ref().unwrap());
        let mut initialized = Frame::new(Vec::new(), None);
        initialized.init(frames[4].as_ref().unwrap());
        for frame in [
            changed_frames[4].as_mut().unwrap(),
            &mut clone,
            &mut cloned_from,
            &mut initialized,
        ] {
            change(frame);
            assert_ne!(Some(frame.to_string()), expected[4], "change {index}");
        }
        let mut unchanged = printed(&changed_frames);
        unchanged[4] = expected[4].clone();
        assert_eq!(unchanged, expected, "change {index}");
        assert_eq!(printed(&frames), expected, "change {index}");
    }
}
