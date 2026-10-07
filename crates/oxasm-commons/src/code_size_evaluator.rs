//! [`CodeSizeEvaluator`]: ASM's `CodeSizeEvaluator`.

use oxasm::opcodes::*;
use oxasm::{Constant, Handle, JavaStr, Label, MethodVisitor};

/// A [`MethodVisitor`] that approximates the size of the methods it visits.
pub struct CodeSizeEvaluator<'a> {
    /// The method visitor to which this visitor delegates calls, if any.
    mv: Option<Box<dyn MethodVisitor + 'a>>,

    /// The minimum size in bytes of the visited method.
    min_size: usize,

    /// The maximum size in bytes of the visited method.
    max_size: usize,
}

impl std::fmt::Debug for CodeSizeEvaluator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeSizeEvaluator")
            .field("min_size", &self.min_size)
            .field("max_size", &self.max_size)
            .finish_non_exhaustive()
    }
}

impl<'a> CodeSizeEvaluator<'a> {
    /// Constructs a new `CodeSizeEvaluator`, which delegates its calls to `method_visitor`, if
    /// any.
    pub fn new(method_visitor: Option<Box<dyn MethodVisitor + 'a>>) -> Self {
        CodeSizeEvaluator {
            mv: method_visitor,
            min_size: 0,
            max_size: 0,
        }
    }

    /// The minimum size in bytes of the visited method.
    #[must_use]
    pub fn get_min_size(&self) -> usize {
        self.min_size
    }

    /// The maximum size in bytes of the visited method.
    #[must_use]
    pub fn get_max_size(&self) -> usize {
        self.max_size
    }

    fn add(&mut self, min_size: usize, max_size: usize) {
        self.min_size += min_size;
        self.max_size += max_size;
    }
}

impl MethodVisitor for CodeSizeEvaluator<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        match &mut self.mv {
            Some(mv) => Some(&mut **mv),
            None => None,
        }
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.add(1, 1);
        if let Some(mv) = self.delegate() {
            mv.visit_insn(opcode);
        }
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        if opcode == SIPUSH {
            self.add(3, 3);
        } else {
            self.add(2, 2);
        }
        if let Some(mv) = self.delegate() {
            mv.visit_int_insn(opcode, operand);
        }
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        if var_index < 4 && opcode != RET {
            self.add(1, 1);
        } else if var_index >= 256 {
            self.add(4, 4);
        } else {
            self.add(2, 2);
        }
        if let Some(mv) = self.delegate() {
            mv.visit_var_insn(opcode, var_index);
        }
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.add(3, 3);
        if let Some(mv) = self.delegate() {
            mv.visit_type_insn(opcode, type_);
        }
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.add(3, 3);
        if let Some(mv) = self.delegate() {
            mv.visit_field_insn(opcode, owner, name, descriptor);
        }
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        if opcode == INVOKEINTERFACE {
            self.add(5, 5);
        } else {
            self.add(3, 3);
        }
        if let Some(mv) = self.delegate() {
            mv.visit_method_insn(opcode, owner, name, descriptor, is_interface);
        }
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.add(5, 5);
        if let Some(mv) = self.delegate() {
            mv.visit_invoke_dynamic_insn(
                name,
                descriptor,
                bootstrap_method_handle,
                bootstrap_method_arguments,
            );
        }
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        if opcode == GOTO || opcode == JSR {
            self.add(3, 5);
        } else {
            self.add(3, 8);
        }
        if let Some(mv) = self.delegate() {
            mv.visit_jump_insn(opcode, label);
        }
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        let two_words = match value {
            Constant::Long(_) | Constant::Double(_) => true,
            Constant::Dynamic(constant_dynamic) => constant_dynamic.get_size() == 2,
            _ => false,
        };
        if two_words {
            self.add(3, 3);
        } else {
            self.add(2, 3);
        }
        if let Some(mv) = self.delegate() {
            mv.visit_ldc_insn(value);
        }
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        if var_index > 255 || !(-128..=127).contains(&increment) {
            self.add(6, 6);
        } else {
            self.add(3, 3);
        }
        if let Some(mv) = self.delegate() {
            mv.visit_iinc_insn(var_index, increment);
        }
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.add(13 + labels.len() * 4, 16 + labels.len() * 4);
        if let Some(mv) = self.delegate() {
            mv.visit_table_switch_insn(min, max, dflt, labels);
        }
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.add(9 + keys.len() * 8, 12 + keys.len() * 8);
        if let Some(mv) = self.delegate() {
            mv.visit_lookup_switch_insn(dflt, keys, labels);
        }
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.add(4, 4);
        if let Some(mv) = self.delegate() {
            mv.visit_multi_a_new_array_insn(descriptor, num_dimensions);
        }
    }
}
