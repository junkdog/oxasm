//! [`MethodVisitor`]: ASM's `MethodVisitor`.

use crate::attribute::Attribute;
use crate::constant::{Constant, FrameValue};
use crate::handle::Handle;
use crate::jstr::JavaStr;
use crate::label::Label;
use crate::opcodes::FrameKind;
use crate::type_path::TypePath;
use crate::type_reference::TypeReference;
use crate::visitor::AnnotationVisitor;

/// What a class reader offers a method writer for the method copy optimization (ADR-08): the raw
/// `method_info` of a method that can be copied as is, if nothing changes it on the way.
///
/// Internal: only the class reader creates these, and only the method writer uses them.
#[doc(hidden)]
#[derive(Debug)]
#[allow(dead_code)] // read by the reader (P2) and the writer (P5)
pub struct MethodCopySource<'r> {
    /// The id of the class reader the bytes come from.
    pub(crate) reader_id: u64,
    /// The `method_info` structure, attributes included.
    pub(crate) method_info: &'r [u8],
}

/// A visitor to visit a Java method. The methods of this trait must be called in the following
/// order: ( `visit_parameter` )* \[ `visit_annotation_default` \] ( `visit_annotation` |
/// `visit_annotable_parameter_count` | `visit_parameter_annotation` | `visit_type_annotation` |
/// `visit_attribute` )* \[ `visit_code` ( `visit_frame` | `visit_<x>_insn` | `visit_label` |
/// `visit_insn_annotation` | `visit_try_catch_block` | `visit_try_catch_annotation` |
/// `visit_local_variable` | `visit_local_variable_annotation` | `visit_line_number` |
/// `visit_attribute` )* `visit_maxs` \] `visit_end`.
///
/// In addition, the `visit_<x>_insn` and `visit_label` methods must be called in the sequential
/// order of the bytecode instructions of the visited code, `visit_insn_annotation` must be called
/// *after* the annotated instruction, `visit_try_catch_block` must be called *before* the labels
/// passed as arguments have been visited, `visit_try_catch_annotation` must be called *after* the
/// corresponding try catch block has been visited, and the `visit_local_variable`,
/// `visit_local_variable_annotation` and `visit_line_number` methods must be called *after* the
/// labels passed as arguments have been visited. Finally, the `visit_attribute` method must be
/// called before `visit_code` for non-code attributes, and after it for code attributes.
///
/// See the [module docs](crate::visitor) for the delegation and child visitor conventions.
pub trait MethodVisitor {
    /// The method visitor to which this visitor must delegate method calls; `None` (the default)
    /// ends the chain. Must return the same visitor every time.
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        None
    }

    /// The method copy optimization hook (ADR-08): a class reader offers the raw bytes of a
    /// method, and only a method writer that can copy them as is accepts (returning `true`, after
    /// which the reader does not visit the method's content).
    ///
    /// This method is deliberately **not** forwarded to [`MethodVisitor::delegate`]: an adapter
    /// that transforms methods must not be bypassed because its downstream is a writer. (The
    /// `&mut V` and `Box<V>` impls do forward it, since they are the same visitor.)
    #[doc(hidden)]
    fn try_copy_method(&mut self, _source: &MethodCopySource<'_>) -> bool {
        false
    }

    // ---------------------------------------------------------------------------------------------
    // Parameters, annotations and non standard attributes
    // ---------------------------------------------------------------------------------------------

    /// Visits a parameter of this method.
    ///
    /// - `name`: parameter name, or `None` if none is provided.
    /// - `access`: the parameter's access flags, only `ACC_FINAL`, `ACC_SYNTHETIC` or/and
    ///   `ACC_MANDATED` are allowed (see [`crate::opcodes`]).
    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        if let Some(d) = self.delegate() {
            d.visit_parameter(name, access);
        }
    }

    /// Visits the default value of this annotation interface method.
    ///
    /// Returns a visitor to the visit the actual default value of this annotation interface
    /// method, or `None` if this visitor is not interested in visiting this default value. The
    /// `name` parameters passed to the methods of this annotation visitor are ignored. Moreover,
    /// exactly one visit method must be called on this annotation visitor, followed by
    /// `visit_end`.
    fn visit_annotation_default<'a>(&'a mut self) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_annotation_default()
    }

    /// Visits an annotation of this method.
    ///
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_annotation(descriptor, visible)
    }

    /// Visits an annotation on a type in the method signature.
    ///
    /// - `type_ref`: a reference to the annotated type. The sort of this type reference must be
    ///   [`TypeReference::METHOD_TYPE_PARAMETER`],
    ///   [`TypeReference::METHOD_TYPE_PARAMETER_BOUND`], [`TypeReference::METHOD_RETURN`],
    ///   [`TypeReference::METHOD_RECEIVER`], [`TypeReference::METHOD_FORMAL_PARAMETER`] or
    ///   [`TypeReference::THROWS`].
    /// - `type_path`: the path to the annotated type argument, wildcard bound, array element
    ///   type, or static inner type within `type_ref`. `None` if the annotation targets
    ///   `type_ref` as a whole.
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?
            .visit_type_annotation(type_ref, type_path, descriptor, visible)
    }

    /// Visits the number of method parameters that can have annotations. By default (i.e. when
    /// this method is not called), all the method parameters defined by the method descriptor can
    /// have annotations.
    ///
    /// - `parameter_count`: the number of method parameters than can have annotations. This
    ///   number must be less or equal than the number of parameter types in the method
    ///   descriptor. It can be strictly less when a method has synthetic parameters and when these
    ///   parameters are ignored when computing parameter indices for the purpose of parameter
    ///   annotations (see JVMS 4.7.18).
    /// - `visible`: `true` to define the number of method parameters that can have annotations
    ///   visible at runtime, `false` to define the number of method parameters that can have
    ///   annotations invisible at runtime.
    fn visit_annotable_parameter_count(&mut self, parameter_count: u8, visible: bool) {
        if let Some(d) = self.delegate() {
            d.visit_annotable_parameter_count(parameter_count, visible);
        }
    }

    /// Visits an annotation of a parameter this method.
    ///
    /// - `parameter`: the parameter index. This index must be strictly smaller than the number of
    ///   parameters in the method descriptor, and strictly smaller than the parameter count
    ///   specified in `visit_annotable_parameter_count`. Important note: *a parameter index i is
    ///   not required to correspond to the i'th parameter descriptor in the method descriptor*, in
    ///   particular in case of synthetic parameters (see JVMS 4.7.18).
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    fn visit_parameter_annotation<'a>(
        &'a mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?
            .visit_parameter_annotation(parameter, descriptor, visible)
    }

    /// Visits a non standard attribute of this method.
    fn visit_attribute(&mut self, attribute: &Attribute) {
        if let Some(d) = self.delegate() {
            d.visit_attribute(attribute);
        }
    }

    /// Starts the visit of the method's code, if any (i.e. non abstract method).
    fn visit_code(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_code();
        }
    }

    /// Visits the current state of the local variables and operand stack elements. This method
    /// must(*) be called *just before* any instruction **i** that follows an unconditional branch
    /// instruction such as GOTO or THROW, that is the target of a jump instruction, or that starts
    /// an exception handler block. The visited types must describe the values of the local
    /// variables and of the operand stack elements *just before* **i** is executed.
    ///
    /// (*) this is mandatory only for classes whose version is greater than or equal to `V1_6`.
    ///
    /// The frames of a method must be given either in expanded form, or in compressed form (all
    /// frames must use the same format, i.e. you must not mix expanded and compressed frames
    /// within a single method):
    ///
    /// - In expanded form, all frames must have the [`FrameKind::New`] kind.
    /// - In compressed form, frames are basically "deltas" from the state of the previous frame:
    ///   - [`FrameKind::Same`]: exactly the same locals as the previous frame and an empty stack.
    ///   - [`FrameKind::Same1`]: exactly the same locals as the previous frame and a single value
    ///     on the stack (`stack` has one element).
    ///   - [`FrameKind::Append`]: the same locals as the previous frame, except that additional
    ///     locals are defined (`locals` holds the 1, 2 or 3 added types), and an empty stack.
    ///   - [`FrameKind::Chop`]: the same locals as the previous frame, except that the last 1-3
    ///     locals (the count carried by the kind) are absent, and an empty stack. `locals` is
    ///     empty.
    ///   - [`FrameKind::Full`]: complete frame data.
    ///
    /// In both cases the first frame, corresponding to the method's parameters and access flags,
    /// is implicit and must not be visited. Also, it is illegal to visit two or more frames for
    /// the same code location (i.e., at least one instruction must be visited between two calls
    /// to `visit_frame`).
    ///
    /// - `locals`: the local variable types in this frame. Long and double values are represented
    ///   by a single element.
    /// - `stack`: the operand stack types in this frame, in the same format.
    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        if let Some(d) = self.delegate() {
            d.visit_frame(kind, locals, stack);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Normal instructions
    // ---------------------------------------------------------------------------------------------

    /// Visits a zero operand instruction.
    ///
    /// - `opcode`: the opcode of the instruction to be visited. This opcode is either NOP,
    ///   ACONST_NULL, ICONST_M1, ICONST_0, ICONST_1, ICONST_2, ICONST_3, ICONST_4, ICONST_5,
    ///   LCONST_0, LCONST_1, FCONST_0, FCONST_1, FCONST_2, DCONST_0, DCONST_1, IALOAD, LALOAD,
    ///   FALOAD, DALOAD, AALOAD, BALOAD, CALOAD, SALOAD, IASTORE, LASTORE, FASTORE, DASTORE,
    ///   AASTORE, BASTORE, CASTORE, SASTORE, POP, POP2, DUP, DUP_X1, DUP_X2, DUP2, DUP2_X1,
    ///   DUP2_X2, SWAP, IADD, LADD, FADD, DADD, ISUB, LSUB, FSUB, DSUB, IMUL, LMUL, FMUL, DMUL,
    ///   IDIV, LDIV, FDIV, DDIV, IREM, LREM, FREM, DREM, INEG, LNEG, FNEG, DNEG, ISHL, LSHL, ISHR,
    ///   LSHR, IUSHR, LUSHR, IAND, LAND, IOR, LOR, IXOR, LXOR, I2L, I2F, I2D, L2I, L2F, L2D, F2I,
    ///   F2L, F2D, D2I, D2L, D2F, I2B, I2C, I2S, LCMP, FCMPL, FCMPG, DCMPL, DCMPG, IRETURN,
    ///   LRETURN, FRETURN, DRETURN, ARETURN, RETURN, ARRAYLENGTH, ATHROW, MONITORENTER, or
    ///   MONITOREXIT.
    fn visit_insn(&mut self, opcode: u8) {
        if let Some(d) = self.delegate() {
            d.visit_insn(opcode);
        }
    }

    /// Visits an instruction with a single int operand.
    ///
    /// - `opcode`: the opcode of the instruction to be visited. This opcode is either BIPUSH,
    ///   SIPUSH or NEWARRAY.
    /// - `operand`: the operand of the instruction to be visited. When opcode is BIPUSH, operand
    ///   value should be between `i8::MIN` and `i8::MAX`. When opcode is SIPUSH, operand value
    ///   should be between `i16::MIN` and `i16::MAX`. When opcode is NEWARRAY, operand value
    ///   should be one of `T_BOOLEAN`, `T_CHAR`, `T_FLOAT`, `T_DOUBLE`, `T_BYTE`, `T_SHORT`,
    ///   `T_INT` or `T_LONG`.
    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        if let Some(d) = self.delegate() {
            d.visit_int_insn(opcode, operand);
        }
    }

    /// Visits a local variable instruction. A local variable instruction is an instruction that
    /// loads or stores the value of a local variable.
    ///
    /// - `opcode`: the opcode of the local variable instruction to be visited. This opcode is
    ///   either ILOAD, LLOAD, FLOAD, DLOAD, ALOAD, ISTORE, LSTORE, FSTORE, DSTORE, ASTORE or RET.
    /// - `var_index`: the operand of the instruction to be visited. This operand is the index of
    ///   a local variable.
    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        if let Some(d) = self.delegate() {
            d.visit_var_insn(opcode, var_index);
        }
    }

    /// Visits a type instruction. A type instruction is an instruction that takes the internal
    /// name of a class as parameter.
    ///
    /// - `opcode`: the opcode of the type instruction to be visited. This opcode is either NEW,
    ///   ANEWARRAY, CHECKCAST or INSTANCEOF.
    /// - `type_`: the operand of the instruction to be visited. This operand must be the internal
    ///   name of an object or array class.
    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_type_insn(opcode, type_);
        }
    }

    /// Visits a field instruction. A field instruction is an instruction that loads or stores the
    /// value of a field of an object.
    ///
    /// - `opcode`: the opcode of the type instruction to be visited. This opcode is either
    ///   GETSTATIC, PUTSTATIC, GETFIELD or PUTFIELD.
    /// - `owner`: the internal name of the field's owner class.
    /// - `name`: the field's name.
    /// - `descriptor`: the field's descriptor (see [`Type`](crate::Type)).
    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        if let Some(d) = self.delegate() {
            d.visit_field_insn(opcode, owner, name, descriptor);
        }
    }

    /// Visits a method instruction. A method instruction is an instruction that invokes a method.
    ///
    /// - `opcode`: the opcode of the type instruction to be visited. This opcode is either
    ///   INVOKEVIRTUAL, INVOKESPECIAL, INVOKESTATIC or INVOKEINTERFACE.
    /// - `owner`: the internal name of the method's owner class.
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`](crate::Type)).
    /// - `is_interface`: if the method's owner class is an interface.
    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        if let Some(d) = self.delegate() {
            d.visit_method_insn(opcode, owner, name, descriptor, is_interface);
        }
    }

    /// Visits an invokedynamic instruction.
    ///
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`](crate::Type)).
    /// - `bootstrap_method_handle`: the bootstrap method.
    /// - `bootstrap_method_arguments`: the bootstrap method constant arguments.
    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        if let Some(d) = self.delegate() {
            d.visit_invoke_dynamic_insn(
                name,
                descriptor,
                bootstrap_method_handle,
                bootstrap_method_arguments,
            );
        }
    }

    /// Visits a jump instruction. A jump instruction is an instruction that may jump to another
    /// instruction.
    ///
    /// - `opcode`: the opcode of the type instruction to be visited. This opcode is either IFEQ,
    ///   IFNE, IFLT, IFGE, IFGT, IFLE, IF_ICMPEQ, IF_ICMPNE, IF_ICMPLT, IF_ICMPGE, IF_ICMPGT,
    ///   IF_ICMPLE, IF_ACMPEQ, IF_ACMPNE, GOTO, JSR, IFNULL or IFNONNULL.
    /// - `label`: the operand of the instruction to be visited. This operand is a label that
    ///   designates the instruction to which the jump instruction may jump.
    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        if let Some(d) = self.delegate() {
            d.visit_jump_insn(opcode, label);
        }
    }

    /// Visits a label. A label designates the instruction that will be visited just after it.
    fn visit_label(&mut self, label: Label) {
        if let Some(d) = self.delegate() {
            d.visit_label(label);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Special instructions
    // ---------------------------------------------------------------------------------------------

    /// Visits a LDC instruction.
    ///
    /// - `value`: the constant to be loaded on the stack: an `Integer`, a `Float`, a `Long`, a
    ///   `Double`, a `String`, a [`Type`](crate::Type) of `OBJECT` or `ARRAY` sort for `.class`
    ///   constants (for classes whose version is 49), a `Type` of `METHOD` sort for `MethodType`,
    ///   a [`Handle`] for `MethodHandle` constants (for classes whose version is 51), or a
    ///   [`ConstantDynamic`](crate::ConstantDynamic) for a constant dynamic (for classes whose
    ///   version is 55).
    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        if let Some(d) = self.delegate() {
            d.visit_ldc_insn(value);
        }
    }

    /// Visits an IINC instruction.
    ///
    /// - `var_index`: index of the local variable to be incremented.
    /// - `increment`: amount to increment the local variable by.
    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        if let Some(d) = self.delegate() {
            d.visit_iinc_insn(var_index, increment);
        }
    }

    /// Visits a TABLESWITCH instruction.
    ///
    /// - `min`: the minimum key value.
    /// - `max`: the maximum key value.
    /// - `dflt`: beginning of the default handler block.
    /// - `labels`: beginnings of the handler blocks. `labels[i]` is the beginning of the handler
    ///   block for the `min + i` key.
    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        if let Some(d) = self.delegate() {
            d.visit_table_switch_insn(min, max, dflt, labels);
        }
    }

    /// Visits a LOOKUPSWITCH instruction.
    ///
    /// - `dflt`: beginning of the default handler block.
    /// - `keys`: the values of the keys. Keys must be sorted in increasing order.
    /// - `labels`: beginnings of the handler blocks. `labels[i]` is the beginning of the handler
    ///   block for the `keys[i]` key.
    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        if let Some(d) = self.delegate() {
            d.visit_lookup_switch_insn(dflt, keys, labels);
        }
    }

    /// Visits a MULTIANEWARRAY instruction.
    ///
    /// - `descriptor`: an array type descriptor (see [`Type`](crate::Type)).
    /// - `num_dimensions`: the number of dimensions of the array to allocate.
    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        if let Some(d) = self.delegate() {
            d.visit_multi_a_new_array_insn(descriptor, num_dimensions);
        }
    }

    /// Visits an annotation on an instruction. This method must be called just *after* the
    /// annotated instruction. It can be called several times for the same instruction.
    ///
    /// - `type_ref`: a reference to the annotated type. The sort of this type reference must be
    ///   [`TypeReference::INSTANCEOF`], [`TypeReference::NEW`],
    ///   [`TypeReference::CONSTRUCTOR_REFERENCE`], [`TypeReference::METHOD_REFERENCE`],
    ///   [`TypeReference::CAST`], [`TypeReference::CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT`],
    ///   [`TypeReference::METHOD_INVOCATION_TYPE_ARGUMENT`],
    ///   [`TypeReference::CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT`], or
    ///   [`TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT`].
    /// - `type_path`: the path to the annotated type argument, wildcard bound, array element
    ///   type, or static inner type within `type_ref`. `None` if the annotation targets
    ///   `type_ref` as a whole.
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    fn visit_insn_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?
            .visit_insn_annotation(type_ref, type_path, descriptor, visible)
    }

    // ---------------------------------------------------------------------------------------------
    // Exceptions table entries, debug information, max stack and max locals
    // ---------------------------------------------------------------------------------------------

    /// Visits a try catch block.
    ///
    /// - `start`: the beginning of the exception handler's scope (inclusive).
    /// - `end`: the end of the exception handler's scope (exclusive).
    /// - `handler`: the beginning of the exception handler's code.
    /// - `type_`: the internal name of the type of exceptions handled by the handler, or `None`
    ///   to catch any exceptions (for "finally" blocks).
    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        if let Some(d) = self.delegate() {
            d.visit_try_catch_block(start, end, handler, type_);
        }
    }

    /// Visits an annotation on an exception handler type. This method must be called *after* the
    /// `visit_try_catch_block` for the annotated exception handler. It can be called several
    /// times for the same exception handler.
    ///
    /// - `type_ref`: a reference to the annotated type. The sort of this type reference must be
    ///   [`TypeReference::EXCEPTION_PARAMETER`].
    /// - `type_path`: the path to the annotated type argument, wildcard bound, array element
    ///   type, or static inner type within `type_ref`. `None` if the annotation targets
    ///   `type_ref` as a whole.
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    fn visit_try_catch_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?
            .visit_try_catch_annotation(type_ref, type_path, descriptor, visible)
    }

    /// Visits a local variable declaration.
    ///
    /// - `name`: the name of a local variable.
    /// - `descriptor`: the type descriptor of this local variable.
    /// - `signature`: the type signature of this local variable. `None` if the local variable
    ///   type does not use generic types.
    /// - `start`: the first instruction corresponding to the scope of this local variable
    ///   (inclusive).
    /// - `end`: the last instruction corresponding to the scope of this local variable
    ///   (exclusive).
    /// - `index`: the local variable's index.
    fn visit_local_variable(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        start: Label,
        end: Label,
        index: u16,
    ) {
        if let Some(d) = self.delegate() {
            d.visit_local_variable(name, descriptor, signature, start, end, index);
        }
    }

    /// Visits an annotation on a local variable type.
    ///
    /// - `type_ref`: a reference to the annotated type. The sort of this type reference must be
    ///   [`TypeReference::LOCAL_VARIABLE`] or [`TypeReference::RESOURCE_VARIABLE`].
    /// - `type_path`: the path to the annotated type argument, wildcard bound, array element
    ///   type, or static inner type within `type_ref`. `None` if the annotation targets
    ///   `type_ref` as a whole.
    /// - `start`: the fist instructions corresponding to the continuous ranges that make the
    ///   scope of this local variable (inclusive).
    /// - `end`: the last instructions corresponding to the continuous ranges that make the scope
    ///   of this local variable (exclusive). This slice must have the same length as `start`.
    /// - `index`: the local variable's index in each range. This slice must have the same length
    ///   as `start`.
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    #[allow(clippy::too_many_arguments)] // ASM's signature
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
        self.delegate()?.visit_local_variable_annotation(
            type_ref, type_path, start, end, index, descriptor, visible,
        )
    }

    /// Visits a line number declaration.
    ///
    /// - `line`: a line number. This number refers to the source file from which the class was
    ///   compiled.
    /// - `start`: the first instruction corresponding to this line number.
    fn visit_line_number(&mut self, line: u16, start: Label) {
        if let Some(d) = self.delegate() {
            d.visit_line_number(line, start);
        }
    }

    /// Visits the maximum stack size and the maximum number of local variables of the method.
    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        if let Some(d) = self.delegate() {
            d.visit_maxs(max_stack, max_locals);
        }
    }

    /// Visits the end of the method. This method, which is the last one to be called, is used to
    /// inform the visitor that all the annotations and attributes of the method have been
    /// visited.
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

macro_rules! forward_method_visitor {
    ($($ty:ty),*) => {$(
        impl<T: MethodVisitor + ?Sized> MethodVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
                (**self).delegate()
            }
            fn try_copy_method(&mut self, source: &MethodCopySource<'_>) -> bool {
                (**self).try_copy_method(source)
            }
            fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
                (**self).visit_parameter(name, access);
            }
            fn visit_annotation_default<'a>(&'a mut self) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_annotation_default()
            }
            fn visit_annotation<'a>(
                &'a mut self,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_annotation(descriptor, visible)
            }
            fn visit_type_annotation<'a>(
                &'a mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_type_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_annotable_parameter_count(&mut self, parameter_count: u8, visible: bool) {
                (**self).visit_annotable_parameter_count(parameter_count, visible);
            }
            fn visit_parameter_annotation<'a>(
                &'a mut self,
                parameter: u8,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_parameter_annotation(parameter, descriptor, visible)
            }
            fn visit_attribute(&mut self, attribute: &Attribute) {
                (**self).visit_attribute(attribute);
            }
            fn visit_code(&mut self) {
                (**self).visit_code();
            }
            fn visit_frame(
                &mut self,
                kind: FrameKind,
                locals: &[FrameValue<'_>],
                stack: &[FrameValue<'_>],
            ) {
                (**self).visit_frame(kind, locals, stack);
            }
            fn visit_insn(&mut self, opcode: u8) {
                (**self).visit_insn(opcode);
            }
            fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
                (**self).visit_int_insn(opcode, operand);
            }
            fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
                (**self).visit_var_insn(opcode, var_index);
            }
            fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
                (**self).visit_type_insn(opcode, type_);
            }
            fn visit_field_insn(
                &mut self,
                opcode: u8,
                owner: &JavaStr,
                name: &JavaStr,
                descriptor: &JavaStr,
            ) {
                (**self).visit_field_insn(opcode, owner, name, descriptor);
            }
            fn visit_method_insn(
                &mut self,
                opcode: u8,
                owner: &JavaStr,
                name: &JavaStr,
                descriptor: &JavaStr,
                is_interface: bool,
            ) {
                (**self).visit_method_insn(opcode, owner, name, descriptor, is_interface);
            }
            fn visit_invoke_dynamic_insn(
                &mut self,
                name: &JavaStr,
                descriptor: &JavaStr,
                bootstrap_method_handle: &Handle,
                bootstrap_method_arguments: &[Constant<'_>],
            ) {
                (**self).visit_invoke_dynamic_insn(
                    name,
                    descriptor,
                    bootstrap_method_handle,
                    bootstrap_method_arguments,
                );
            }
            fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
                (**self).visit_jump_insn(opcode, label);
            }
            fn visit_label(&mut self, label: Label) {
                (**self).visit_label(label);
            }
            fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
                (**self).visit_ldc_insn(value);
            }
            fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
                (**self).visit_iinc_insn(var_index, increment);
            }
            fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
                (**self).visit_table_switch_insn(min, max, dflt, labels);
            }
            fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
                (**self).visit_lookup_switch_insn(dflt, keys, labels);
            }
            fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
                (**self).visit_multi_a_new_array_insn(descriptor, num_dimensions);
            }
            fn visit_insn_annotation<'a>(
                &'a mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_insn_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_try_catch_block(
                &mut self,
                start: Label,
                end: Label,
                handler: Label,
                type_: Option<&JavaStr>,
            ) {
                (**self).visit_try_catch_block(start, end, handler, type_);
            }
            fn visit_try_catch_annotation<'a>(
                &'a mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_try_catch_annotation(type_ref, type_path, descriptor, visible)
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
                (**self).visit_local_variable(name, descriptor, signature, start, end, index);
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
                (**self).visit_local_variable_annotation(
                    type_ref, type_path, start, end, index, descriptor, visible,
                )
            }
            fn visit_line_number(&mut self, line: u16, start: Label) {
                (**self).visit_line_number(line, start);
            }
            fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
                (**self).visit_maxs(max_stack, max_locals);
            }
            fn visit_end(&mut self) {
                (**self).visit_end();
            }
        }
    )*};
}

forward_method_visitor!(&mut T, Box<T>);
