//! [`LocalVariablesSorter`]: ASM's `LocalVariablesSorter`.

// Java's narrowing casts are the specification here (ADR-17): local variable indices are ints in
// ASM, written as 16 bit values.
#![allow(clippy::cast_possible_truncation)]

use oxasm::constant::FrameValue;
use oxasm::opcodes::*;
use oxasm::{
    AnnotationVisitor, FrameKind, JavaStr, Label, MethodVisitor, Type, TypePath, TypeReference,
    TypeSort,
};

use crate::support::illegal_argument;

/// The type of the `java.lang.Object` class.
const OBJECT_TYPE: Type = Type::static_object_type(oxasm::jstr!("java/lang/Object"));

/// The hooks of a [`LocalVariablesSorter`]: ASM's protected methods that subclasses override.
/// Every method has a default implementation, which is ASM's.
pub trait LocalVariablesSorterHooks {
    /// Notifies that a new stack map frame is being visited. The array argument contains the
    /// stack map frame types corresponding to the local variables added with
    /// [`LocalVariablesSorter::new_local`]. This method can update these types in place for the
    /// stack map frame being visited. The default implementation of this method does nothing,
    /// i.e. a local variable added with `new_local` will have the same type in all stack map
    /// frames. But this behavior is not always the desired one, for instance if a local variable
    /// is added in the middle of a try/catch block: the frame for the exception handler should
    /// have a TOP type for this new local.
    ///
    /// - `new_locals`: the stack map frame types corresponding to the local variables added with
    ///   `new_local` (and `None` for the others). The format of this array is the same as in
    ///   [`MethodVisitor::visit_frame`], except that long and double types use two slots. The
    ///   types for the current stack map frame must be updated in place in this array.
    fn update_new_locals(&mut self, new_locals: &mut [Option<FrameValue<'static>>]) {
        let _ = new_locals;
    }

    /// Notifies that a local variable has been added or remapped. The default implementation of
    /// this method does nothing.
    ///
    /// - `local`: a local variable identifier, as returned by
    ///   [`LocalVariablesSorter::new_local`].
    /// - `type_`: the type of the value being stored in the local variable.
    fn set_local_type(&mut self, local: u32, type_: &Type) {
        let _ = (local, type_);
    }

    /// Returns the index of a new local variable of the given type, given the index of the next
    /// free local variable, which this method must update.
    fn new_local_mapping(&mut self, next_local: &mut u32, type_: &Type) -> u32 {
        let local = *next_local;
        *next_local += type_.get_size() as u32;
        local
    }
}

/// No hooks: ASM's `LocalVariablesSorter` itself.
impl LocalVariablesSorterHooks for () {}

/// A [`MethodVisitor`] that renumbers local variables in their order of appearance. This adapter
/// allows one to easily add new local variables to a method. The next visitor in the chain can
/// add new locals when needed by calling [`LocalVariablesSorter::new_local`] on this adapter.
///
/// ASM's protected methods (`updateNewLocals`, `setLocalType` and `newLocalMapping`) are the
/// [`LocalVariablesSorterHooks`] `H` (ADR-13).
///
/// # Panics
///
/// Its methods panic where ASM's throw: on an invalid opcode in `visit_var_insn`, and on a frame
/// that is not expanded (see [`oxasm::ParsingOptions`]'s `EXPAND_FRAMES`).
pub struct LocalVariablesSorter<M, H = ()> {
    /// The method visitor to which this adapter delegates calls.
    pub(crate) mv: M,
    /// The hooks of this sorter.
    pub hooks: H,
    /// The mapping from old to new local variable indices. A local variable at index i of size 1
    /// is remapped to 'mapping[2*i]', while a local variable at index i of size 2 is remapped to
    /// 'mapping[2*i+1]'. A value is the new index plus 1, or 0 if there is none yet.
    remapped_variable_indices: Vec<u32>,
    /// The local variable types after remapping. The format of this array is the same as in
    /// [`MethodVisitor::visit_frame`], except that long and double types use two slots.
    remapped_local_types: Vec<Option<FrameValue<'static>>>,
    /// The index of the first local variable, after formal parameters.
    first_local: u32,
    /// The index of the next local variable to be created by `new_local`.
    next_local: u32,
}

impl<M: MethodVisitor> LocalVariablesSorter<M> {
    /// Constructs a new [`LocalVariablesSorter`].
    ///
    /// - `access`: access flags of the adapted method.
    /// - `descriptor`: the method's descriptor (see [`Type`]).
    /// - `method_visitor`: the method visitor to which this adapter delegates calls.
    #[track_caller]
    pub fn new(access: u32, descriptor: &JavaStr, method_visitor: M) -> Self {
        LocalVariablesSorter::with_hooks(access, descriptor, method_visitor, ())
    }
}

impl<M: MethodVisitor, H: LocalVariablesSorterHooks> LocalVariablesSorter<M, H> {
    /// Constructs a new [`LocalVariablesSorter`] with the given hooks (ASM's protected
    /// constructor, for subclasses).
    #[track_caller]
    pub fn with_hooks(access: u32, descriptor: &JavaStr, method_visitor: M, hooks: H) -> Self {
        let mut next_local = u32::from(ACC_STATIC & access == 0);
        for argument_type in Type::get_argument_types_of(descriptor) {
            next_local += argument_type.get_size() as u32;
        }
        LocalVariablesSorter {
            mv: method_visitor,
            hooks,
            remapped_variable_indices: vec![0; 40],
            remapped_local_types: vec![None; 20],
            first_local: next_local,
            next_local,
        }
    }

    /// The method visitor to which this adapter delegates calls (ASM's `mv`).
    pub fn mv(&mut self) -> &mut M {
        &mut self.mv
    }

    /// Returns the method visitor to which this adapter delegates calls.
    pub fn into_inner(self) -> M {
        self.mv
    }

    /// The index of the first local variable, after formal parameters.
    #[must_use]
    pub fn first_local(&self) -> u32 {
        self.first_local
    }

    /// The index of the next local variable to be created by [`LocalVariablesSorter::new_local`].
    #[must_use]
    pub fn next_local(&self) -> u32 {
        self.next_local
    }

    /// Constructs a new local variable of the given type, and returns its identifier.
    ///
    /// # Panics
    ///
    /// If `type_` is `void` or a method type (ASM's `AssertionError`).
    #[track_caller]
    pub fn new_local(&mut self, type_: &Type) -> u16 {
        let local_type = match type_.get_sort() {
            TypeSort::Boolean
            | TypeSort::Char
            | TypeSort::Byte
            | TypeSort::Short
            | TypeSort::Int => FrameValue::Integer,
            TypeSort::Float => FrameValue::Float,
            TypeSort::Long => FrameValue::Long,
            TypeSort::Double => FrameValue::Double,
            TypeSort::Array => FrameValue::Object(type_.get_descriptor().into_owned().into()),
            TypeSort::Object => FrameValue::Object(type_.get_internal_name().to_owned().into()),
            TypeSort::Void | TypeSort::Method => panic!("java.lang.AssertionError"),
        };
        let local = self.new_local_mapping(type_);
        self.hooks.set_local_type(local, type_);
        self.set_frame_local(local, local_type);
        local as u16
    }

    fn set_frame_local(&mut self, local: u32, type_: FrameValue<'static>) {
        let local = local as usize;
        let num_locals = self.remapped_local_types.len();
        if local >= num_locals {
            self.remapped_local_types
                .resize((2 * num_locals).max(local + 1), None);
        }
        self.remapped_local_types[local] = Some(type_);
    }

    fn remap(&mut self, var_index: u32, type_: &Type) -> u32 {
        let size = type_.get_size() as u32;
        if var_index + size <= self.first_local {
            return var_index;
        }
        // Not negative: a void type (of size 0) past the arguments has a positive index.
        let key = (2 * var_index + size - 1) as usize;
        let indices_size = self.remapped_variable_indices.len();
        if key >= indices_size {
            self.remapped_variable_indices
                .resize((2 * indices_size).max(key + 1), 0);
        }
        let mut value = self.remapped_variable_indices[key];
        if value == 0 {
            value = self.new_local_mapping(type_);
            self.hooks.set_local_type(value, type_);
            self.remapped_variable_indices[key] = value + 1;
        } else {
            value -= 1;
        }
        value
    }

    fn new_local_mapping(&mut self, type_: &Type) -> u32 {
        self.hooks.new_local_mapping(&mut self.next_local, type_)
    }
}

impl<M, H> std::fmt::Debug for LocalVariablesSorter<M, H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalVariablesSorter")
            .field("first_local", &self.first_local)
            .field("next_local", &self.next_local)
            .finish_non_exhaustive()
    }
}

/// The type of a local variable in a stack map frame, for its remapping.
fn frame_value_type<'a>(local_type: &'a FrameValue<'_>) -> Type<'a> {
    match local_type {
        FrameValue::Integer => Type::INT,
        FrameValue::Float => Type::FLOAT,
        FrameValue::Long => Type::LONG,
        FrameValue::Double => Type::DOUBLE,
        FrameValue::Object(internal_name) => crate::support::object_type(internal_name),
        _ => OBJECT_TYPE,
    }
}

fn is_long_or_double(local_type: Option<&FrameValue<'_>>) -> bool {
    matches!(local_type, Some(FrameValue::Long | FrameValue::Double))
}

impl<M: MethodVisitor, H: LocalVariablesSorterHooks> MethodVisitor for LocalVariablesSorter<M, H> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.mv)
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        let var_type = match opcode {
            LLOAD | LSTORE => Type::LONG,
            DLOAD | DSTORE => Type::DOUBLE,
            FLOAD | FSTORE => Type::FLOAT,
            ILOAD | ISTORE => Type::INT,
            ALOAD | ASTORE | RET => OBJECT_TYPE,
            _ => illegal_argument(format_args!("Invalid opcode {opcode}")),
        };
        let remapped = self.remap(u32::from(var_index), &var_type);
        self.mv.visit_var_insn(opcode, remapped as u16);
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        let remapped = self.remap(u32::from(var_index), &Type::INT);
        self.mv.visit_iinc_insn(remapped as u16, increment);
    }

    fn visit_maxs(&mut self, max_stack: u16, _max_locals: u16) {
        self.mv.visit_maxs(max_stack, self.next_local as u16);
    }

    fn visit_local_variable(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        start: Label,
        end: Label,
        index: u16,
    ) {
        let remapped_index = self.remap(u32::from(index), &Type::get_type(descriptor)) as u16;
        self.mv
            .visit_local_variable(name, descriptor, signature, start, end, remapped_index);
    }

    fn visit_local_variable_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        start: &[Label],
        end: &[Label],
        index: &[u16],
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let type_ = Type::get_type(descriptor);
        let remapped_index: Vec<u16> = index
            .iter()
            .map(|&i| self.remap(u32::from(i), &type_) as u16)
            .collect();
        self.mv.visit_local_variable_annotation(
            type_ref,
            type_path,
            start,
            end,
            &remapped_index,
            descriptor,
            visible,
        )
    }

    fn visit_frame(&mut self, kind: FrameKind, local: &[FrameValue<'_>], stack: &[FrameValue<'_>]) {
        if kind != FrameKind::New {
            // Uncompressed frame.
            illegal_argument(
                "LocalVariablesSorter only accepts expanded frames (see ClassReader.EXPAND_FRAMES)",
            );
        }

        // Create a copy of remappedLocals.
        let old_remapped_locals = self.remapped_local_types.clone();

        self.hooks.update_new_locals(&mut self.remapped_local_types);

        // Copy the types from 'local' to 'remappedLocals'. 'remappedLocals' already contains the
        // variables added with 'newLocal'.
        let mut old_var: u32 = 0; // Old local variable index.
        for local_type in local {
            if *local_type != FrameValue::Top {
                let var_type = frame_value_type(local_type);
                let remapped = self.remap(old_var, &var_type);
                self.set_frame_local(remapped, local_type.to_owned());
            }
            old_var += if is_long_or_double(Some(local_type)) {
                2
            } else {
                1
            };
        }

        // Remove TOP after long and double types as well as trailing TOPs.
        let mut old_var = 0;
        let mut new_var = 0;
        let mut remapped_num_local = 0;
        while old_var < self.remapped_local_types.len() {
            let local_type = self.remapped_local_types[old_var].take();
            old_var += if is_long_or_double(local_type.as_ref()) {
                2
            } else {
                1
            };
            match local_type {
                Some(local_type) if local_type != FrameValue::Top => {
                    self.remapped_local_types[new_var] = Some(local_type);
                    new_var += 1;
                    remapped_num_local = new_var;
                }
                _ => {
                    self.remapped_local_types[new_var] = Some(FrameValue::Top);
                    new_var += 1;
                }
            }
        }

        // Visit the remapped frame.
        let remapped_locals: Vec<FrameValue<'static>> = self.remapped_local_types
            [..remapped_num_local]
            .iter()
            .map(|t| t.clone().unwrap_or(FrameValue::Top))
            .collect();
        self.mv.visit_frame(kind, &remapped_locals, stack);

        // Restore the original value of 'remappedLocals'.
        self.remapped_local_types = old_remapped_locals;
    }
}
