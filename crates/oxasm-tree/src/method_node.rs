//! [`MethodNode`]: ASM's `MethodNode`.

use std::sync::Arc;

use oxasm::constant::{Constant, FrameValue};
use oxasm::sym::InternMemo;
use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, FrameKind, Handle, Interner, JavaStr, Label,
    MethodVisitor, Sym, Type, TypePath, TypeReference,
};

use crate::annotation_node::{
    AnnotationNode, AnnotationNodeValue, AnnotationNodeVisitor, LocalVariableAnnotationNode,
    TypeAnnotationNode, Values, ValuesVisitor, accept_annotation,
};
use crate::field_node::{accept_annotations, add_annotation, add_type_annotation};
use crate::frame_node::FrameNode;
use crate::insn::{Insn, InvokeDynamicInsn};
use crate::insn_list::InsnList;
use crate::nodes::{LocalVariableNode, ParameterNode, TryCatchBlockNode, try_catch_type_ref};
use crate::strings::{Strings, resolve_all};
use crate::value::{SymConstant, SymHandle};

/// The label a method node stores for a visited label.
///
/// ASM's `MethodNode` maps each visited label to a `LabelNode`, which hands out a *fresh* label
/// when the node is visited, without the reader's `FLAG_DEBUG_ONLY`. Here the visited label is
/// the node's identity, so the flag is cleared instead, for a method writer to see what it sees
/// in ASM (ADR-03).
pub(crate) fn plain(label: Label) -> Label {
    label.with_debug_only(false)
}

/// A node that represents a method.
///
/// Unlike ASM, which hands out fresh labels each time a method node is visited (`resetLabels`),
/// the labels of a method node are always the same: accepting a method node twice into the
/// same method visitor gives it the same labels twice.
///
/// Its strings, and those of its instructions, local variables and the like, are [`Sym`]s of its
/// interner, which [`resolve`](Self::resolve) turns back into strings. `MethodNode::default()`
/// makes a new interner. Instructions moved from one method node to another keep their symbols,
/// so both nodes must share an interner (see [`with_interner`](Self::with_interner)).
#[derive(Clone, Debug, Default)]
pub struct MethodNode {
    /// The interner of this node's strings.
    interner: Arc<Interner>,
    /// The method's access flags (see [`oxasm::opcodes`]). This field also indicates if the
    /// method is synthetic and/or deprecated.
    pub access: u32,
    /// The method's name.
    pub name: Sym,
    /// The method's descriptor.
    pub desc: Sym,
    /// The method's signature, or `None`.
    pub signature: Option<Sym>,
    /// The internal names of the method's exception classes.
    pub exceptions: Vec<Sym>,
    /// The rarely used parts of this method, or `None` if it has none (see
    /// [`extras`](Self::extras) and [`extras_mut`](Self::extras_mut)).
    pub extras: Option<Box<MethodExtras>>,
    /// The instructions of this method.
    pub instructions: InsnList,
    /// The try catch blocks of this method.
    pub try_catch_blocks: Vec<TryCatchBlockNode>,
    /// The maximum stack size of this method.
    pub max_stack: u16,
    /// The maximum number of local variables of this method.
    pub max_locals: u16,
    /// The local variables of this method.
    pub local_variables: Vec<LocalVariableNode>,
    /// The code length this node was visited with, if any (see
    /// [`MethodVisitor::visit_code_length`]), which [`accept`](Self::accept) passes on.
    visited_code_length: Option<u32>,
}

// The rarely used parts are behind one box (T12).
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MethodNode>() == 208);

/// The rarely used parts of a [`MethodNode`]: its parameters, annotations, non standard
/// attributes and annotation default, which most methods don't have.
///
/// They are behind one box, which only the methods that have some allocate (where ASM leaves
/// their lists `null`), so that a method node takes 208 bytes instead of 584: sift keeps the
/// trees of whole class paths, and a smaller node is cheaper to make, move and drop (T12 in
/// oxasm-optimization.md).
#[derive(Clone, Debug, Default)]
pub struct MethodExtras {
    /// The method parameter info (access flags and name).
    pub parameters: Vec<ParameterNode>,
    /// The runtime visible annotations of this method.
    pub visible_annotations: Vec<AnnotationNode>,
    /// The runtime invisible annotations of this method.
    pub invisible_annotations: Vec<AnnotationNode>,
    /// The runtime visible type annotations of this method.
    pub visible_type_annotations: Vec<TypeAnnotationNode>,
    /// The runtime invisible type annotations of this method.
    pub invisible_type_annotations: Vec<TypeAnnotationNode>,
    /// The non standard attributes of this method (Code attributes included).
    pub attrs: Vec<Attribute>,
    /// The default value of this annotation interface method, or `None`.
    pub annotation_default: Option<AnnotationNodeValue>,
    /// The number of method parameters than can have runtime visible annotations. This number
    /// must be less or equal than the number of parameter types in the method descriptor (the
    /// default value 0 indicates that all the parameters described in the method descriptor can
    /// have annotations). It can be strictly less when a method has synthetic parameters and
    /// when these parameters are ignored when computing parameter indices for the purpose of
    /// parameter annotations (see JVMS 4.7.18).
    pub visible_annotable_parameter_count: u8,
    /// The runtime visible parameter annotations of this method, per parameter. Empty if the
    /// method has none (ASM's `null`); otherwise, as long as the method has parameters.
    pub visible_parameter_annotations: Vec<Vec<AnnotationNode>>,
    /// The number of method parameters than can have runtime invisible annotations; see
    /// [`visible_annotable_parameter_count`](Self::visible_annotable_parameter_count).
    pub invisible_annotable_parameter_count: u8,
    /// The runtime invisible parameter annotations of this method, per parameter; see
    /// [`visible_parameter_annotations`](Self::visible_parameter_annotations).
    pub invisible_parameter_annotations: Vec<Vec<AnnotationNode>>,
    /// The visible local variable annotations of this method.
    pub visible_local_variable_annotations: Vec<LocalVariableAnnotationNode>,
    /// The invisible local variable annotations of this method.
    pub invisible_local_variable_annotations: Vec<LocalVariableAnnotationNode>,
}

/// The extras of a method node that has none.
static NO_EXTRAS: MethodExtras = MethodExtras {
    parameters: Vec::new(),
    visible_annotations: Vec::new(),
    invisible_annotations: Vec::new(),
    visible_type_annotations: Vec::new(),
    invisible_type_annotations: Vec::new(),
    attrs: Vec::new(),
    annotation_default: None,
    visible_annotable_parameter_count: 0,
    visible_parameter_annotations: Vec::new(),
    invisible_annotable_parameter_count: 0,
    invisible_parameter_annotations: Vec::new(),
    visible_local_variable_annotations: Vec::new(),
    invisible_local_variable_annotations: Vec::new(),
};

impl MethodNode {
    /// Constructs a new method node, with no content, whose strings are symbols of the given
    /// interner.
    pub fn new(
        interner: Arc<Interner>,
        access: u32,
        name: Sym,
        descriptor: Sym,
        signature: Option<Sym>,
        exceptions: Vec<Sym>,
    ) -> MethodNode {
        MethodNode {
            interner,
            access,
            name,
            desc: descriptor,
            signature,
            exceptions,
            extras: None,
            instructions: InsnList::new(),
            try_catch_blocks: Vec::new(),
            max_stack: 0,
            max_locals: 0,
            local_variables: Vec::new(),
            visited_code_length: None,
        }
    }

    /// Constructs a new method node, with no content, whose strings are interned in the given
    /// interner (ASM's `MethodNode(access, name, descriptor, signature, exceptions)`).
    pub fn from_strings(
        interner: Arc<Interner>,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> MethodNode {
        let name = interner.intern(name);
        let descriptor = interner.intern(descriptor);
        let signature = signature.map(|s| interner.intern(s));
        let exceptions = exceptions.iter().map(|&e| interner.intern(e)).collect();
        MethodNode::new(interner, access, name, descriptor, signature, exceptions)
    }

    /// Constructs a method node with no content, not even a name (ASM's `MethodNode()`), whose
    /// strings are symbols of the given interner: a method visitor that stores what it visits.
    pub fn with_interner(interner: Arc<Interner>) -> MethodNode {
        MethodNode::new(
            interner,
            0,
            Sym::default(),
            Sym::default(),
            None,
            Vec::new(),
        )
    }

    /// The rarely used parts of this method: empty ones if it has none.
    #[inline]
    pub fn extras(&self) -> &MethodExtras {
        self.extras.as_deref().unwrap_or(&NO_EXTRAS)
    }

    /// The rarely used parts of this method, which are allocated (empty) if it has none.
    #[inline]
    pub fn extras_mut(&mut self) -> &mut MethodExtras {
        self.extras.get_or_insert_with(Box::default)
    }

    /// The interner of this node's strings.
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }

    /// The string of the given symbol of this node.
    pub fn resolve(&self, sym: Sym) -> &JavaStr {
        self.interner.resolve(sym)
    }

    /// The symbol of the given string in this node's interner.
    pub fn intern(&self, s: &(impl AsRef<JavaStr> + ?Sized)) -> Sym {
        self.interner.intern(s)
    }

    // ---------------------------------------------------------------------------------------------
    // Accept methods
    // ---------------------------------------------------------------------------------------------

    /// Makes the given class visitor visit this method.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let interner = &*self.interner;
        if let Some(mut method_visitor) = class_visitor.visit_method(
            self.access,
            interner.resolve(self.name),
            interner.resolve(self.desc),
            self.signature.map(|s| interner.resolve(s)),
            &resolve_all(interner, &self.exceptions),
        ) {
            self.accept_method_visitor(&mut *method_visitor);
        }
    }

    /// Makes the given method visitor visit this method (ASM's `accept(MethodVisitor)`).
    ///
    /// The type annotations of the try catch blocks are visited with the type reference of
    /// their block's current index, as ASM's `updateIndex` sets them (without changing the
    /// nodes, see [`TryCatchBlockNode::update_index`]).
    pub fn accept_method_visitor(&self, method_visitor: &mut dyn MethodVisitor) {
        let interner = &*self.interner;
        let extras = self.extras();
        // Visit the parameters.
        for parameter in &extras.parameters {
            parameter.accept(interner, method_visitor);
        }
        // Visit the annotations.
        if let Some(annotation_default) = &extras.annotation_default
            && let Some(mut annotation_visitor) = method_visitor.visit_annotation_default()
        {
            annotation_default.accept(interner, &mut *annotation_visitor, None);
            annotation_visitor.visit_end();
        }
        accept_annotations!(interner, extras, &mut *method_visitor);
        for (count, parameter_annotations, visible) in [
            (
                extras.visible_annotable_parameter_count,
                &extras.visible_parameter_annotations,
                true,
            ),
            (
                extras.invisible_annotable_parameter_count,
                &extras.invisible_parameter_annotations,
                false,
            ),
        ] {
            if count > 0 {
                method_visitor.visit_annotable_parameter_count(count, visible);
            }
            for (parameter, annotations) in parameter_annotations.iter().enumerate() {
                // A parameter index is a u8 (at most 255 parameters).
                let Ok(parameter) = u8::try_from(parameter) else {
                    break;
                };
                for annotation in annotations {
                    let av = method_visitor.visit_parameter_annotation(
                        parameter,
                        interner.resolve(annotation.desc),
                        visible,
                    );
                    accept_annotation(interner, annotation, av);
                }
            }
        }
        // Visit the non standard attributes.
        for attribute in &extras.attrs {
            method_visitor.visit_attribute(attribute);
        }
        // Visit the code.
        if !self.instructions.is_empty() {
            method_visitor.visit_code();
            method_visitor.visit_code_length(self.code_length_hint());
            // Visits the try catch blocks.
            for (index, try_catch_block) in self.try_catch_blocks.iter().enumerate() {
                try_catch_block.accept_with(
                    interner,
                    method_visitor,
                    Some(try_catch_type_ref(index)),
                );
            }
            // Visit the instructions.
            self.instructions.accept(interner, method_visitor);
            // Visits the local variables.
            for local_variable in &self.local_variables {
                local_variable.accept(interner, method_visitor);
            }
            // Visits the local variable annotations.
            for annotation in &extras.visible_local_variable_annotations {
                annotation.accept(interner, method_visitor, true);
            }
            for annotation in &extras.invisible_local_variable_annotations {
                annotation.accept(interner, method_visitor, false);
            }
            method_visitor.visit_maxs(self.max_stack, self.max_locals);
        }
        method_visitor.visit_end();
    }

    /// The length of this method's bytecode to send to [`MethodVisitor::visit_code_length`]:
    /// the one this node was visited with, if any, otherwise an estimate. It is only a hint, so
    /// a length that the node's changes since have made stale costs a visitor at most some
    /// growth or some unused capacity.
    ///
    /// The estimate is two bytes per node, plus 4. A method has about 1.1 bytes of bytecode per
    /// node at the median and 1.9 at p99, more in large methods, so this bound is short for 0.3%
    /// of a corpus' methods (1% of its bytecode).
    fn code_length_hint(&self) -> u32 {
        if let Some(code_length) = self.visited_code_length {
            return code_length;
        }
        u32::try_from(self.instructions.len())
            .map_or(u32::MAX, |n| n.saturating_mul(2).saturating_add(4))
    }
}

// -------------------------------------------------------------------------------------------------
// Implementation of the MethodVisitor trait
// -------------------------------------------------------------------------------------------------

/// The visitor methods of a method node that store strings, interned directly or through the
/// memo of the class being visited.
impl MethodNode {
    /// The strings of this node's visit: through the given memo, if any.
    fn strings<'a>(interner: &'a Interner, memo: Option<&'a mut InternMemo>) -> Strings<'a> {
        match memo {
            Some(memo) => Strings::Memo(memo),
            None => Strings::Direct(interner),
        }
    }

    fn add_parameter(
        &mut self,
        memo: Option<&mut InternMemo>,
        name: Option<&JavaStr>,
        access: u32,
    ) {
        let name = Self::strings(&self.interner, memo).get_opt(name);
        self.extras_mut()
            .parameters
            .push(ParameterNode::new(name, access));
    }

    fn add_type_insn(&mut self, memo: Option<&mut InternMemo>, opcode: u8, type_: &JavaStr) {
        let desc = Self::strings(&self.interner, memo).get(type_);
        self.instructions.add(Insn::Type { opcode, desc });
    }

    fn add_field_insn(
        &mut self,
        memo: Option<&mut InternMemo>,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        let mut strings = Self::strings(&self.interner, memo);
        let insn = Insn::Field {
            opcode,
            owner: strings.get(owner),
            name: strings.get(name),
            desc: strings.get(descriptor),
        };
        self.instructions.add(insn);
    }

    fn add_method_insn(
        &mut self,
        memo: Option<&mut InternMemo>,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        let mut strings = Self::strings(&self.interner, memo);
        let insn = Insn::Method {
            opcode,
            owner: strings.get(owner),
            name: strings.get(name),
            desc: strings.get(descriptor),
            is_interface,
        };
        self.instructions.add(insn);
    }

    fn add_invoke_dynamic_insn(
        &mut self,
        memo: Option<&mut InternMemo>,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        let mut strings = Self::strings(&self.interner, memo);
        let insn = InvokeDynamicInsn {
            name: strings.get(name),
            desc: strings.get(descriptor),
            bsm: SymHandle::from_visited(bootstrap_method_handle, &mut strings),
            bsm_args: bootstrap_method_arguments
                .iter()
                .map(|arg| SymConstant::from_visited(arg, &mut strings))
                .collect(),
        };
        self.instructions.add(Insn::InvokeDynamic(Box::new(insn)));
    }

    fn add_ldc_insn(&mut self, memo: Option<&mut InternMemo>, value: &Constant<'_>) {
        let value = SymConstant::from_visited(value, &mut Self::strings(&self.interner, memo));
        self.instructions.add(Insn::Ldc(value));
    }

    fn add_multi_a_new_array_insn(
        &mut self,
        memo: Option<&mut InternMemo>,
        descriptor: &JavaStr,
        num_dimensions: u8,
    ) {
        let desc = Self::strings(&self.interner, memo).get(descriptor);
        self.instructions.add(Insn::MultiANewArray {
            desc,
            dims: num_dimensions,
        });
    }

    fn add_frame(
        &mut self,
        memo: Option<&mut InternMemo>,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        let mut strings = Self::strings(&self.interner, memo);
        let frame = FrameNode::from_visited(kind, locals, stack, &mut strings);
        self.instructions.add(frame);
    }

    fn add_try_catch_block(
        &mut self,
        memo: Option<&mut InternMemo>,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        let type_ = Self::strings(&self.interner, memo).get_opt(type_);
        self.try_catch_blocks.push(TryCatchBlockNode::new(
            plain(start),
            plain(end),
            plain(handler),
            type_,
        ));
    }

    #[allow(clippy::too_many_arguments)] // ASM's signature, and the strings
    fn add_local_variable(
        &mut self,
        memo: Option<&mut InternMemo>,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        start: Label,
        end: Label,
        index: u16,
    ) {
        let mut strings = Self::strings(&self.interner, memo);
        let local_variable = LocalVariableNode::new(
            strings.get(name),
            strings.get(descriptor),
            strings.get_opt(signature),
            plain(start),
            plain(end),
            index,
        );
        self.local_variables.push(local_variable);
    }

    fn add_annotation<'a>(
        &'a mut self,
        memo: Option<&'a mut InternMemo>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let extras = self.extras.get_or_insert_with(Box::default);
        add_annotation(
            &mut extras.visible_annotations,
            &mut extras.invisible_annotations,
            Self::strings(&self.interner, memo),
            descriptor,
            visible,
        )
    }

    fn add_type_annotation<'a>(
        &'a mut self,
        memo: Option<&'a mut InternMemo>,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let extras = self.extras.get_or_insert_with(Box::default);
        add_type_annotation(
            &mut extras.visible_type_annotations,
            &mut extras.invisible_type_annotations,
            Self::strings(&self.interner, memo),
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn add_annotation_default<'a>(
        &'a mut self,
        memo: Option<&'a mut InternMemo>,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let extras = self.extras.get_or_insert_with(Box::default);
        Some(Box::new(ValuesVisitor {
            values: Values::Default(&mut extras.annotation_default),
            strings: Self::strings(&self.interner, memo),
        }))
    }

    /// Like ASM, the parameter annotation lists are first sized by the number of parameters of
    /// the method descriptor. Where ASM throws (`ArrayIndexOutOfBoundsException` for a parameter
    /// index past them, `StringIndexOutOfBoundsException` for a malformed descriptor), which only
    /// malformed class files cause, they grow to the parameter index instead.
    fn add_parameter_annotation<'a>(
        &'a mut self,
        memo: Option<&'a mut InternMemo>,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let extras = self.extras.get_or_insert_with(Box::default);
        let parameter_annotations = if visible {
            &mut extras.visible_parameter_annotations
        } else {
            &mut extras.invisible_parameter_annotations
        };
        if parameter_annotations.is_empty() {
            let params =
                Type::try_get_argument_count_of(self.interner.resolve(self.desc)).unwrap_or(0);
            parameter_annotations.resize_with(params, Vec::new);
        }
        let parameter = usize::from(parameter);
        if parameter >= parameter_annotations.len() {
            parameter_annotations.resize_with(parameter + 1, Vec::new);
        }
        let mut strings = Self::strings(&self.interner, memo);
        let annotations = &mut parameter_annotations[parameter];
        annotations.push(AnnotationNode::new(strings.get(descriptor)));
        let annotation = annotations.last_mut().expect("just added");
        Some(Box::new(AnnotationNodeVisitor::new(annotation, strings)))
    }

    /// The annotation goes to the last real instruction (not a label, frame or line number).
    /// If there is none, which only a malformed class file causes, it is not stored (ASM throws
    /// a `NullPointerException`).
    fn add_insn_annotation<'a>(
        &'a mut self,
        memo: Option<&'a mut InternMemo>,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        // Find the last real instruction, i.e. the instruction targeted by this annotation.
        let mut current_insn = self.instructions.last();
        while let Some(id) = current_insn
            && self.instructions[id].opcode().is_none()
        {
            current_insn = self.instructions.previous(id);
        }
        // Add the annotation to this instruction.
        let annotations = self.instructions.type_annotations_mut(current_insn?);
        add_type_annotation(
            &mut annotations.visible,
            &mut annotations.invisible,
            Self::strings(&self.interner, memo),
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    /// The annotation goes to the try catch block whose index is in the type reference. If
    /// there is no such block, which only a malformed class file causes, it is not stored (ASM
    /// throws an `IndexOutOfBoundsException`).
    fn add_try_catch_annotation<'a>(
        &'a mut self,
        memo: Option<&'a mut InternMemo>,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let index = (type_ref.get_value() & 0x00FF_FF00) >> 8;
        let try_catch_block = self.try_catch_blocks.get_mut(index as usize)?;
        add_type_annotation(
            &mut try_catch_block.visible_type_annotations,
            &mut try_catch_block.invisible_type_annotations,
            Self::strings(&self.interner, memo),
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    #[allow(clippy::too_many_arguments)] // ASM's signature, and the strings
    fn add_local_variable_annotation<'a>(
        &'a mut self,
        memo: Option<&'a mut InternMemo>,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        start: &[Label],
        end: &[Label],
        index: &[u16],
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let extras = self.extras.get_or_insert_with(Box::default);
        let annotations = if visible {
            &mut extras.visible_local_variable_annotations
        } else {
            &mut extras.invisible_local_variable_annotations
        };
        let mut strings = Self::strings(&self.interner, memo);
        annotations.push(LocalVariableAnnotationNode::new(
            type_ref,
            type_path.cloned(),
            start.iter().copied().map(plain).collect(),
            end.iter().copied().map(plain).collect(),
            index.to_vec(),
            strings.get(descriptor),
        ));
        let annotation = annotations.last_mut().expect("just added");
        Some(Box::new(AnnotationNodeVisitor::new(
            &mut annotation.annotation,
            strings,
        )))
    }
}

/// A method node visited on its own interns each visited string directly in its interner (a
/// [`ClassNode`](crate::ClassNode) interns them through its memo).
impl MethodVisitor for MethodNode {
    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        self.add_parameter(None, name, access);
    }

    fn visit_annotation_default<'a>(&'a mut self) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.add_annotation_default(None)
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.add_annotation(None, descriptor, visible)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.add_type_annotation(None, type_ref, type_path, descriptor, visible)
    }

    fn visit_annotable_parameter_count(&mut self, parameter_count: u8, visible: bool) {
        if visible {
            self.extras_mut().visible_annotable_parameter_count = parameter_count;
        } else {
            self.extras_mut().invisible_annotable_parameter_count = parameter_count;
        }
    }

    fn visit_parameter_annotation<'a>(
        &'a mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.add_parameter_annotation(None, parameter, descriptor, visible)
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.extras_mut().attrs.push(attribute.clone());
    }

    fn visit_code(&mut self) {
        // Nothing to do.
    }

    fn visit_code_length(&mut self, code_length: u32) {
        self.visited_code_length = Some(code_length);
        // A method has about 0.7 nodes (instructions, labels, frames and line numbers) per
        // byte of bytecode, small methods up to about 1: one node per byte, and a few for the
        // smallest, leaves most lists without a single reallocation.
        self.instructions
            .reserve(usize::try_from(code_length).map_or(0, |n| n.saturating_add(4)));
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        self.add_frame(None, kind, locals, stack);
    }

    #[inline]
    fn visit_insn(&mut self, opcode: u8) {
        self.instructions.add(Insn::Simple { opcode });
    }

    #[inline]
    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.instructions.add(Insn::Int { opcode, operand });
    }

    #[inline]
    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.instructions.add(Insn::Var {
            opcode,
            var: var_index,
        });
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.add_type_insn(None, opcode, type_);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.add_field_insn(None, opcode, owner, name, descriptor);
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.add_method_insn(None, opcode, owner, name, descriptor, is_interface);
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.add_invoke_dynamic_insn(
            None,
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
    }

    #[inline]
    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.instructions.add(Insn::Jump {
            opcode,
            label: plain(label),
        });
    }

    #[inline]
    fn visit_label(&mut self, label: Label) {
        self.instructions.add(Insn::Label(plain(label)));
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.add_ldc_insn(None, value);
    }

    #[inline]
    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.instructions.add(Insn::Iinc {
            var: var_index,
            incr: increment,
        });
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.instructions.add(Insn::table_switch_insn(
            min,
            max,
            plain(dflt),
            labels.iter().copied().map(plain).collect(),
        ));
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.instructions.add(Insn::lookup_switch_insn(
            plain(dflt),
            keys.to_vec(),
            labels.iter().copied().map(plain).collect(),
        ));
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.add_multi_a_new_array_insn(None, descriptor, num_dimensions);
    }

    fn visit_insn_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.add_insn_annotation(None, type_ref, type_path, descriptor, visible)
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.add_try_catch_block(None, start, end, handler, type_);
    }

    fn visit_try_catch_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.add_try_catch_annotation(None, type_ref, type_path, descriptor, visible)
    }

    fn visit_local_variable_count(&mut self, count: u16) {
        self.local_variables.reserve(usize::from(count));
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
        self.add_local_variable(None, name, descriptor, signature, start, end, index);
    }

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
        self.add_local_variable_annotation(
            None, type_ref, type_path, start, end, index, descriptor, visible,
        )
    }

    #[inline]
    fn visit_line_number(&mut self, line: u16, start: Label) {
        self.instructions.add(Insn::Line {
            line,
            start: plain(start),
        });
    }

    fn visit_maxs(&mut self, max_stack: u16, max_locals: u16) {
        self.max_stack = max_stack;
        self.max_locals = max_locals;
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// The method visitor of a method node that a class node visits: it interns the strings of the
/// method through the class node's memo, and otherwise delegates to the method node.
pub(crate) struct PooledMethodNode<'a> {
    pub(crate) node: &'a mut MethodNode,
    pub(crate) memo: &'a mut InternMemo,
}

impl MethodVisitor for PooledMethodNode<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(self.node)
    }

    // The instructions without strings call the method node's methods directly: through the
    // default methods, each would take a second dyn call, by `delegate` (T11).

    fn visit_insn(&mut self, opcode: u8) {
        self.node.visit_insn(opcode);
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.node.visit_int_insn(opcode, operand);
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.node.visit_var_insn(opcode, var_index);
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.node.visit_jump_insn(opcode, label);
    }

    fn visit_label(&mut self, label: Label) {
        self.node.visit_label(label);
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.node.visit_iinc_insn(var_index, increment);
    }

    fn visit_line_number(&mut self, line: u16, start: Label) {
        self.node.visit_line_number(line, start);
    }

    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        self.node.add_parameter(Some(self.memo), name, access);
    }

    fn visit_annotation_default<'a>(&'a mut self) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.node.add_annotation_default(Some(self.memo))
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.node
            .add_annotation(Some(self.memo), descriptor, visible)
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.node
            .add_type_annotation(Some(self.memo), type_ref, type_path, descriptor, visible)
    }

    fn visit_parameter_annotation<'a>(
        &'a mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.node
            .add_parameter_annotation(Some(self.memo), parameter, descriptor, visible)
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        self.node.add_frame(Some(self.memo), kind, locals, stack);
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.node.add_type_insn(Some(self.memo), opcode, type_);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.node
            .add_field_insn(Some(self.memo), opcode, owner, name, descriptor);
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.node.add_method_insn(
            Some(self.memo),
            opcode,
            owner,
            name,
            descriptor,
            is_interface,
        );
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.node.add_invoke_dynamic_insn(
            Some(self.memo),
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.node.add_ldc_insn(Some(self.memo), value);
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.node
            .add_multi_a_new_array_insn(Some(self.memo), descriptor, num_dimensions);
    }

    fn visit_insn_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.node
            .add_insn_annotation(Some(self.memo), type_ref, type_path, descriptor, visible)
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.node
            .add_try_catch_block(Some(self.memo), start, end, handler, type_);
    }

    fn visit_try_catch_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.node.add_try_catch_annotation(
            Some(self.memo),
            type_ref,
            type_path,
            descriptor,
            visible,
        )
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
        self.node.add_local_variable(
            Some(self.memo),
            name,
            descriptor,
            signature,
            start,
            end,
            index,
        );
    }

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
        self.node.add_local_variable_annotation(
            Some(self.memo),
            type_ref,
            type_path,
            start,
            end,
            index,
            descriptor,
            visible,
        )
    }
}
