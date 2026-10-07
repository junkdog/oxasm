//! ASM's `CodeSizeEvaluatorTest`.

mod common;

use oxasm::{
    ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, JavaStr, Label, MethodVisitor,
    ParsingOptions,
};
use oxasm_commons::CodeSizeEvaluator;
use oxasm_testkit::PrecompiledClass;

/// Tests that the size estimations of CodeSizeEvaluator are correct, and that classes are
/// unchanged with a ClassReader->CodeSizeEvaluator->ClassWriter transform.
///
/// (The `api` dimension of ASM's test disappears, ADR-09.)
#[test]
fn test_all_methods_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
        let mut code_sizes_evaluator = CodeSizesEvaluator {
            cv: &mut class_writer,
            evaluations: Vec::new(),
        };

        class_reader
            .accept_with_codecs(
                &mut code_sizes_evaluator,
                &common::attributes(),
                ParsingOptions::empty(),
            )
            .unwrap();

        let evaluations = code_sizes_evaluator.evaluations;
        let output = class_writer.to_bytes().unwrap();
        // ASM reads the actual size from the offset of a label visited at the end of the code;
        // here it is the code_length of each Code attribute of the written class, in order.
        let actual_sizes = code_lengths(&output);
        assert_eq!(evaluations.len(), actual_sizes.len(), "{class_parameter}");
        for (&(min_size, max_size), &actual_size) in evaluations.iter().zip(&actual_sizes) {
            assert!(actual_size >= min_size, "{class_parameter}");
            assert!(actual_size <= max_size, "{class_parameter}");
        }
        common::assert_same_class_file(class_file, &output, &class_parameter);
    }
}

/// Collects the minimum and maximum sizes the [`CodeSizeEvaluator`] computes for each method.
struct CodeSizesEvaluator<V> {
    cv: V,
    evaluations: Vec<(usize, usize)>,
}

impl<V: ClassVisitor> ClassVisitor for CodeSizesEvaluator<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.cv)
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
            .cv
            .visit_method(access, name, descriptor, signature, exceptions);
        Some(Box::new(EvaluatingMethodVisitor {
            code_size_evaluator: CodeSizeEvaluator::new(method_visitor),
            evaluations: &mut self.evaluations,
        }))
    }
}

/// ASM's anonymous `CodeSizeEvaluator` subclass. (ASM's `visitLdcInsn` override, which checks
/// the type of the constant, is not needed: `Constant` is an enum.)
struct EvaluatingMethodVisitor<'a> {
    code_size_evaluator: CodeSizeEvaluator<'a>,
    evaluations: &'a mut Vec<(usize, usize)>,
}

impl MethodVisitor for EvaluatingMethodVisitor<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.code_size_evaluator)
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        let end = Label::new();
        self.code_size_evaluator.visit_label(end);
        self.code_size_evaluator.visit_maxs(max_stack, max_locals);

        self.evaluations.push((
            self.code_size_evaluator.get_min_size(),
            self.code_size_evaluator.get_max_size(),
        ));
    }
}

/// The `code_length` of the Code attribute of each method of a class file, in method order (for
/// the methods that have one).
fn code_lengths(class_file: &[u8]) -> Vec<usize> {
    let u1 = |offset: usize| usize::from(class_file[offset]);
    let u2 = |offset: usize| {
        usize::from(u16::from_be_bytes([
            class_file[offset],
            class_file[offset + 1],
        ]))
    };
    let u4 = |offset: usize| {
        u32::from_be_bytes(class_file[offset..offset + 4].try_into().unwrap()) as usize
    };
    // The constant pool: the offsets of the CONSTANT_Utf8 entries.
    let constant_pool_count = u2(8);
    let mut utf8_offsets = vec![None; constant_pool_count];
    let mut offset = 10;
    let mut index = 1;
    while index < constant_pool_count {
        let tag = u1(offset);
        let mut entries = 1;
        offset += match tag {
            1 => {
                utf8_offsets[index] = Some(offset);
                3 + u2(offset + 1)
            }
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => 5,
            5 | 6 => {
                entries = 2;
                9
            }
            7 | 8 | 16 | 19 | 20 => 3,
            15 => 4,
            _ => panic!("unknown constant pool tag {tag}"),
        };
        index += entries;
    }
    let is_code = |name_index: usize| {
        utf8_offsets[name_index].is_some_and(|o| &class_file[o + 3..o + 3 + u2(o + 1)] == b"Code")
    };
    // access_flags, this_class, super_class, interfaces.
    offset += 6;
    offset += 2 + 2 * u2(offset);
    let mut code_lengths = Vec::new();
    for is_method in [false, true] {
        let count = u2(offset);
        offset += 2;
        for _ in 0..count {
            let attributes_count = u2(offset + 6);
            offset += 8;
            for _ in 0..attributes_count {
                if is_method && is_code(u2(offset)) {
                    // max_stack, max_locals, code_length.
                    code_lengths.push(u4(offset + 6 + 4));
                }
                offset += 6 + u4(offset + 2);
            }
        }
    }
    code_lengths
}
