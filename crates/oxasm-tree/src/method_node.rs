//! [`MethodNode`]: ASM's `MethodNode`.

use oxasm::constant::{Constant, FrameValue};
use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, FrameKind, Handle, JavaCompactStr, JavaStr, Label,
    MethodVisitor, Type, TypePath, TypeReference,
};

use crate::annotation_node::{
    AnnotationNode, AnnotationNodeValue, LocalVariableAnnotationNode, TypeAnnotationNode,
    ValuesVisitor, accept_annotation,
};
use crate::field_node::{accept_annotations, add_annotation, add_type_annotation};
use crate::frame_node::FrameNode;
use crate::insn::{Insn, InsnNode};
use crate::insn_list::InsnList;
use crate::nodes::{LocalVariableNode, ParameterNode, TryCatchBlockNode, try_catch_type_ref};
use crate::strings::{StringPool, Strings, Unshared};

/// The label a method node stores for a visited label.
///
/// ASM's `MethodNode` maps each visited label to a `LabelNode`, which hands out a *fresh* label
/// when the node is visited, without the reader's `FLAG_DEBUG_ONLY`. Here the visited label is
/// the node's identity, so the flag is cleared instead, for a method writer to see what it sees
/// in ASM (ADR-03).
fn plain(label: Label) -> Label {
    label.with_debug_only(false)
}

/// A node that represents a method.
///
/// Unlike ASM, which hands out fresh labels each time a method node is visited (`resetLabels`),
/// the labels of a method node are always the same: accepting a method node twice into the
/// same method visitor gives it the same labels twice.
#[derive(Clone, Debug, Default)]
pub struct MethodNode {
    /// The method's access flags (see [`oxasm::opcodes`]). This field also indicates if the
    /// method is synthetic and/or deprecated.
    pub access: u32,
    /// The method's name.
    pub name: JavaCompactStr,
    /// The method's descriptor.
    pub desc: JavaCompactStr,
    /// The method's signature, or `None`.
    pub signature: Option<JavaCompactStr>,
    /// The internal names of the method's exception classes.
    pub exceptions: Vec<JavaCompactStr>,
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
    /// The visible local variable annotations of this method.
    pub visible_local_variable_annotations: Vec<LocalVariableAnnotationNode>,
    /// The invisible local variable annotations of this method.
    pub invisible_local_variable_annotations: Vec<LocalVariableAnnotationNode>,
}

impl MethodNode {
    /// Constructs a new method node, with no content.
    pub fn new(
        access: u32,
        name: impl Into<JavaCompactStr>,
        descriptor: impl Into<JavaCompactStr>,
        signature: Option<JavaCompactStr>,
        exceptions: Vec<JavaCompactStr>,
    ) -> MethodNode {
        MethodNode {
            access,
            name: name.into(),
            desc: descriptor.into(),
            signature,
            exceptions,
            ..MethodNode::default()
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Accept methods
    // ---------------------------------------------------------------------------------------------

    /// Makes the given class visitor visit this method.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let exceptions: Vec<&JavaStr> = self.exceptions.iter().map(|e| &**e).collect();
        if let Some(mut method_visitor) = class_visitor.visit_method(
            self.access,
            &self.name,
            &self.desc,
            self.signature.as_deref(),
            &exceptions,
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
        // Visit the parameters.
        for parameter in &self.parameters {
            parameter.accept(method_visitor);
        }
        // Visit the annotations.
        if let Some(annotation_default) = &self.annotation_default
            && let Some(mut annotation_visitor) = method_visitor.visit_annotation_default()
        {
            annotation_default.accept(&mut *annotation_visitor, None);
            annotation_visitor.visit_end();
        }
        accept_annotations!(self, &mut *method_visitor);
        for (count, parameter_annotations, visible) in [
            (
                self.visible_annotable_parameter_count,
                &self.visible_parameter_annotations,
                true,
            ),
            (
                self.invisible_annotable_parameter_count,
                &self.invisible_parameter_annotations,
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
                        &annotation.desc,
                        visible,
                    );
                    accept_annotation(annotation, av);
                }
            }
        }
        // Visit the non standard attributes.
        for attribute in &self.attrs {
            method_visitor.visit_attribute(attribute);
        }
        // Visit the code.
        if !self.instructions.is_empty() {
            method_visitor.visit_code();
            // Visits the try catch blocks.
            for (index, try_catch_block) in self.try_catch_blocks.iter().enumerate() {
                try_catch_block.accept_with(method_visitor, Some(try_catch_type_ref(index)));
            }
            // Visit the instructions.
            self.instructions.accept(method_visitor);
            // Visits the local variables.
            for local_variable in &self.local_variables {
                local_variable.accept(method_visitor);
            }
            // Visits the local variable annotations.
            for annotation in &self.visible_local_variable_annotations {
                annotation.accept(method_visitor, true);
            }
            for annotation in &self.invisible_local_variable_annotations {
                annotation.accept(method_visitor, false);
            }
            method_visitor.visit_maxs(self.max_stack, self.max_locals);
        }
        method_visitor.visit_end();
    }
}

// -------------------------------------------------------------------------------------------------
// Implementation of the MethodVisitor trait
// -------------------------------------------------------------------------------------------------

/// The visitor methods of a method node that store strings, with the strings from the given
/// source.
impl MethodNode {
    fn add_parameter(&mut self, strings: &mut impl Strings, name: Option<&JavaStr>, access: u32) {
        self.parameters
            .push(ParameterNode::new(strings.get_opt(name), access));
    }

    fn add_type_insn(&mut self, strings: &mut impl Strings, opcode: u8, type_: &JavaStr) {
        self.instructions.add(Insn::Type {
            opcode,
            desc: strings.get(type_),
        });
    }

    fn add_field_insn(
        &mut self,
        strings: &mut impl Strings,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.instructions.add(Insn::Field {
            opcode,
            owner: strings.get(owner),
            name: strings.get(name),
            desc: strings.get(descriptor),
        });
    }

    fn add_method_insn(
        &mut self,
        strings: &mut impl Strings,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.instructions.add(Insn::Method {
            opcode,
            owner: strings.get(owner),
            name: strings.get(name),
            desc: strings.get(descriptor),
            is_interface,
        });
    }

    fn add_invoke_dynamic_insn(
        &mut self,
        strings: &mut impl Strings,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.instructions.add(Insn::invoke_dynamic_insn(
            strings.get(name),
            strings.get(descriptor),
            bootstrap_method_handle.clone(),
            bootstrap_method_arguments
                .iter()
                .map(Constant::to_owned)
                .collect(),
        ));
    }

    fn add_multi_a_new_array_insn(
        &mut self,
        strings: &mut impl Strings,
        descriptor: &JavaStr,
        num_dimensions: u8,
    ) {
        self.instructions.add(Insn::MultiANewArray {
            desc: strings.get(descriptor),
            dims: num_dimensions,
        });
    }

    fn add_try_catch_block(
        &mut self,
        strings: &mut impl Strings,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.try_catch_blocks.push(TryCatchBlockNode::new(
            plain(start),
            plain(end),
            plain(handler),
            strings.get_opt(type_),
        ));
    }

    #[allow(clippy::too_many_arguments)] // ASM's signature, and the strings
    fn add_local_variable(
        &mut self,
        strings: &mut impl Strings,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        start: Label,
        end: Label,
        index: u16,
    ) {
        self.local_variables.push(LocalVariableNode::new(
            strings.get(name),
            strings.get(descriptor),
            strings.get_opt(signature),
            plain(start),
            plain(end),
            index,
        ));
    }
}

/// A method node visited on its own stores a new allocation for each visited string longer
/// than 23 bytes (a [`ClassNode`](crate::ClassNode) shares them).
impl MethodVisitor for MethodNode {
    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        self.add_parameter(&mut Unshared, name, access);
    }

    fn visit_annotation_default<'a>(&'a mut self) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        Some(Box::new(ValuesVisitor::Default(
            &mut self.annotation_default,
        )))
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        add_annotation(
            &mut self.visible_annotations,
            &mut self.invisible_annotations,
            descriptor,
            visible,
        )
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        add_type_annotation(
            &mut self.visible_type_annotations,
            &mut self.invisible_type_annotations,
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_annotable_parameter_count(&mut self, parameter_count: u8, visible: bool) {
        if visible {
            self.visible_annotable_parameter_count = parameter_count;
        } else {
            self.invisible_annotable_parameter_count = parameter_count;
        }
    }

    /// Like ASM, the parameter annotation lists are first sized by the number of parameters of
    /// the method descriptor. Where ASM throws (`ArrayIndexOutOfBoundsException` for a parameter
    /// index past them, `StringIndexOutOfBoundsException` for a malformed descriptor), which only
    /// malformed class files cause, they grow to the parameter index instead.
    fn visit_parameter_annotation<'a>(
        &'a mut self,
        parameter: u8,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let parameter_annotations = if visible {
            &mut self.visible_parameter_annotations
        } else {
            &mut self.invisible_parameter_annotations
        };
        if parameter_annotations.is_empty() {
            let params = Type::try_get_argument_count_of(&self.desc).unwrap_or(0);
            parameter_annotations.resize_with(params, Vec::new);
        }
        let parameter = usize::from(parameter);
        if parameter >= parameter_annotations.len() {
            parameter_annotations.resize_with(parameter + 1, Vec::new);
        }
        let annotations = &mut parameter_annotations[parameter];
        annotations.push(AnnotationNode::new(descriptor));
        Some(Box::new(annotations.last_mut().expect("just added")))
    }

    fn visit_attribute(&mut self, attribute: &Attribute) {
        self.attrs.push(attribute.clone());
    }

    fn visit_code(&mut self) {
        // Nothing to do.
    }

    fn visit_frame(
        &mut self,
        kind: FrameKind,
        locals: &[FrameValue<'_>],
        stack: &[FrameValue<'_>],
    ) {
        let mut frame = FrameNode::new(kind, locals, stack);
        for value in frame.local.iter_mut().chain(&mut frame.stack) {
            if let FrameValue::Uninitialized(label) = value {
                *label = plain(*label);
            }
        }
        self.instructions.add(frame);
    }

    fn visit_insn(&mut self, opcode: u8) {
        self.instructions.add(Insn::Simple { opcode });
    }

    fn visit_int_insn(&mut self, opcode: u8, operand: i32) {
        self.instructions.add(Insn::Int { opcode, operand });
    }

    fn visit_var_insn(&mut self, opcode: u8, var_index: u16) {
        self.instructions.add(Insn::Var {
            opcode,
            var: var_index,
        });
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.add_type_insn(&mut Unshared, opcode, type_);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.add_field_insn(&mut Unshared, opcode, owner, name, descriptor);
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.add_method_insn(&mut Unshared, opcode, owner, name, descriptor, is_interface);
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.add_invoke_dynamic_insn(
            &mut Unshared,
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.instructions.add(Insn::Jump {
            opcode,
            label: plain(label),
        });
    }

    fn visit_label(&mut self, label: Label) {
        self.instructions.add(Insn::Label(plain(label)));
    }

    fn visit_ldc_insn(&mut self, value: &Constant<'_>) {
        self.instructions.add(Insn::Ldc(value.to_owned()));
    }

    fn visit_iinc_insn(&mut self, var_index: u16, increment: i16) {
        self.instructions.add(Insn::Iinc {
            var: var_index,
            incr: increment,
        });
    }

    fn visit_table_switch_insn(&mut self, min: i32, max: i32, dflt: Label, labels: &[Label]) {
        self.instructions.add(Insn::TableSwitch {
            min,
            max,
            dflt: plain(dflt),
            labels: labels.iter().copied().map(plain).collect(),
        });
    }

    fn visit_lookup_switch_insn(&mut self, dflt: Label, keys: &[i32], labels: &[Label]) {
        self.instructions.add(Insn::LookupSwitch {
            dflt: plain(dflt),
            keys: keys.to_vec(),
            labels: labels.iter().copied().map(plain).collect(),
        });
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.add_multi_a_new_array_insn(&mut Unshared, descriptor, num_dimensions);
    }

    /// The annotation goes to the last real instruction (not a label, frame or line number).
    /// If there is none, which only a malformed class file causes, it is not stored (ASM throws
    /// a `NullPointerException`).
    fn visit_insn_annotation<'a>(
        &'a mut self,
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
        let insn: &mut InsnNode = &mut self.instructions[current_insn?];
        // Add the annotation to this instruction.
        let annotations = insn.type_annotations_mut();
        add_type_annotation(
            &mut annotations.visible,
            &mut annotations.invisible,
            type_ref,
            type_path,
            descriptor,
            visible,
        )
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.add_try_catch_block(&mut Unshared, start, end, handler, type_);
    }

    /// The annotation goes to the try catch block whose index is in the type reference. If
    /// there is no such block, which only a malformed class file causes, it is not stored (ASM
    /// throws an `IndexOutOfBoundsException`).
    fn visit_try_catch_annotation<'a>(
        &'a mut self,
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
        self.add_local_variable(
            &mut Unshared,
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
        let annotations = if visible {
            &mut self.visible_local_variable_annotations
        } else {
            &mut self.invisible_local_variable_annotations
        };
        annotations.push(LocalVariableAnnotationNode::new(
            type_ref,
            type_path.cloned(),
            start.iter().copied().map(plain).collect(),
            end.iter().copied().map(plain).collect(),
            index.to_vec(),
            descriptor,
        ));
        let annotation = annotations.last_mut().expect("just added");
        Some(Box::new(&mut annotation.annotation))
    }

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

/// The method visitor of a method node that a class node visits: it shares the strings of the
/// method with those of the class, through the class node's pool, and otherwise delegates to the
/// method node.
pub(crate) struct PooledMethodNode<'a> {
    pub(crate) node: &'a mut MethodNode,
    pub(crate) strings: &'a mut StringPool,
}

impl MethodVisitor for PooledMethodNode<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(self.node)
    }

    fn visit_parameter(&mut self, name: Option<&JavaStr>, access: u32) {
        self.node.add_parameter(self.strings, name, access);
    }

    fn visit_type_insn(&mut self, opcode: u8, type_: &JavaStr) {
        self.node.add_type_insn(self.strings, opcode, type_);
    }

    fn visit_field_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.node
            .add_field_insn(self.strings, opcode, owner, name, descriptor);
    }

    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
        is_interface: bool,
    ) {
        self.node
            .add_method_insn(self.strings, opcode, owner, name, descriptor, is_interface);
    }

    fn visit_invoke_dynamic_insn(
        &mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        bootstrap_method_handle: &Handle,
        bootstrap_method_arguments: &[Constant<'_>],
    ) {
        self.node.add_invoke_dynamic_insn(
            self.strings,
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
    }

    fn visit_multi_a_new_array_insn(&mut self, descriptor: &JavaStr, num_dimensions: u8) {
        self.node
            .add_multi_a_new_array_insn(self.strings, descriptor, num_dimensions);
    }

    fn visit_try_catch_block(
        &mut self,
        start: Label,
        end: Label,
        handler: Label,
        type_: Option<&JavaStr>,
    ) {
        self.node
            .add_try_catch_block(self.strings, start, end, handler, type_);
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
        self.node
            .add_local_variable(self.strings, name, descriptor, signature, start, end, index);
    }
}
