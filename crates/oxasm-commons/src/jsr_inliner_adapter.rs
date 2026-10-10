//! [`JsrInlinerAdapter`]: ASM's `JSRInlinerAdapter`.

// Java's int instruction indices are the specification here (ADR-17).
#![allow(clippy::cast_possible_truncation)]

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use oxasm::opcodes::*;
use oxasm::{JavaStr, Label, MethodVisitor};
use oxasm_tree::{
    Insn, InsnKind, InsnList, InsnNode, LocalVariableNode, MethodNode, TryCatchBlockNode,
};
use rustc_hash::FxHashMap;

/// A [`MethodVisitor`] that removes JSR instructions and inlines the referenced subroutines.
///
/// ASM's `JSRInlinerAdapter` *is* a `MethodNode` (ADR-13): this adapter holds one, which stores
/// everything it visits. At [`visit_end`](MethodVisitor::visit_end), if the code contains at
/// least one JSR instruction, the subroutines are inlined in the node, which then makes the
/// downstream visitor, if any, visit the result.
///
/// # Panics
///
/// `visit_end` panics where ASM throws (with the `toString()` of ASM's exception, ADR-06): on a
/// recursive subroutine invocation, on a RET instruction owned by no subroutine, and on code that
/// refers to labels it does not contain.
pub struct JsrInlinerAdapter<'a> {
    /// The method node that stores what this adapter visits, and holds the inlined code after
    /// `visit_end` (ASM's `JSRInlinerAdapter` itself).
    method_node: MethodNode,
    /// The method visitor to send the resulting inlined method code to, if any.
    mv: Option<Box<dyn MethodVisitor + 'a>>,
    /// The instructions that belong to each subroutine. For each label which is the target of a
    /// JSR instruction, in the order of their first JSR, bit i of the corresponding bit set is
    /// set iff instruction at index i belongs to this subroutine.
    subroutines_insns: Vec<(Label, BitSet)>,
    /// The index of each subroutine in `subroutines_insns`, by label.
    subroutine_indices: FxHashMap<Label, usize>,
}

impl<'a> JsrInlinerAdapter<'a> {
    /// Constructs a new [`JsrInlinerAdapter`].
    ///
    /// - `method_visitor`: the method visitor to send the resulting inlined method code to, or
    ///   `None`.
    /// - `access`: the method's access flags (see [`oxasm::opcodes`]). This parameter also
    ///   indicates if the method is synthetic and/or deprecated.
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor.
    /// - `signature`: the method's signature, or `None`.
    /// - `exceptions`: the internal names of the method's exception classes.
    pub fn new(
        method_visitor: Option<Box<dyn MethodVisitor + 'a>>,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Self {
        JsrInlinerAdapter {
            method_node: MethodNode::from_strings(
                Arc::default(),
                access,
                name,
                descriptor,
                signature,
                exceptions,
            ),
            mv: method_visitor,
            subroutines_insns: Vec::new(),
            subroutine_indices: FxHashMap::default(),
        }
    }

    /// The method node of this adapter: what it visited, with the subroutines inlined once
    /// `visit_end` has been called.
    pub fn method_node(&self) -> &MethodNode {
        &self.method_node
    }

    /// The method node of this adapter; see [`method_node`](Self::method_node).
    pub fn into_method_node(self) -> MethodNode {
        self.method_node
    }
}

impl fmt::Debug for JsrInlinerAdapter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsrInlinerAdapter")
            .field("method_node", &self.method_node)
            .field("subroutines_insns", &self.subroutines_insns)
            .finish_non_exhaustive()
    }
}

impl MethodVisitor for JsrInlinerAdapter<'_> {
    /// Every visit method that this adapter does not override is stored in its method node.
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.method_node)
    }

    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.method_node.visit_jump_insn(opcode, label);
        let last = self.method_node.instructions.last();
        let Some(Insn::Jump {
            label: label_node, ..
        }) = last.map(|id| &self.method_node.instructions[id].insn)
        else {
            unreachable!("a method node adds a jump instruction for visit_jump_insn");
        };
        let label_node = *label_node;
        if opcode == JSR && !self.subroutine_indices.contains_key(&label_node) {
            self.subroutine_indices
                .insert(label_node, self.subroutines_insns.len());
            self.subroutines_insns.push((label_node, BitSet::default()));
        }
    }

    fn visit_end(&mut self) {
        if !self.subroutines_insns.is_empty() {
            // If the code contains at least one JSR instruction, inline the subroutines.
            let instructions = std::mem::take(&mut self.method_node.instructions);
            let subroutines_insns = std::mem::take(&mut self.subroutines_insns);
            let mut inliner = Inliner::new(&instructions, subroutines_insns);
            inliner.find_subroutines_insns(&self.method_node.try_catch_blocks);
            let (new_instructions, new_try_catch_blocks, new_local_variables) =
                inliner.emit_code(&self.subroutine_indices, &self.method_node);
            self.subroutines_insns = inliner.subroutines_insns;
            self.method_node.instructions = new_instructions;
            self.method_node.try_catch_blocks = new_try_catch_blocks;
            self.method_node.local_variables = new_local_variables;
        }
        if let Some(mv) = &mut self.mv {
            self.method_node.accept_method_visitor(&mut **mv);
        }
    }
}

/// The inlining state of `visit_end`: the original instructions, and to which subroutine(s)
/// each of them belongs.
struct Inliner<'n> {
    /// The original instructions.
    instructions: &'n InsnList,
    /// The original instructions, by index.
    insns: Vec<&'n InsnNode>,
    /// The instructions that belong to the main "subroutine". Bit i is set iff instruction at
    /// index i belongs to this main "subroutine".
    main_subroutine_insns: BitSet,
    /// The instructions that belong to each subroutine (see
    /// [`JsrInlinerAdapter::subroutines_insns`]).
    subroutines_insns: Vec<(Label, BitSet)>,
    /// The instructions that belong to more that one subroutine. Bit i is set iff instruction at
    /// index i belongs to more than one subroutine.
    shared_subroutine_insns: BitSet,
    /// The instantiations created so far: an [`Instantiation`]'s parent is an index in this
    /// list.
    instantiations: Vec<Instantiation>,
}

/// A subroutine: the main one, or the one at an index of `subroutines_insns` (ASM compares the
/// subroutines' bit sets by identity).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Subroutine {
    Main,
    Jsr(usize),
}

/// The output of [`Inliner::emit_code`]: the new instructions, try catch blocks and local
/// variables.
type Emitted = (InsnList, Vec<TryCatchBlockNode>, Vec<LocalVariableNode>);

impl<'n> Inliner<'n> {
    fn new(instructions: &'n InsnList, subroutines_insns: Vec<(Label, BitSet)>) -> Self {
        Inliner {
            instructions,
            insns: instructions.iter().collect(),
            main_subroutine_insns: BitSet::default(),
            subroutines_insns,
            shared_subroutine_insns: BitSet::default(),
            instantiations: Vec::new(),
        }
    }

    /// ASM's `instructions.indexOf(labelNode)`: -1 for a label that is not in the list (Java's
    /// index of a node that was never added).
    fn index_of(&self, label: Label) -> i32 {
        self.instructions
            .label_index(label)
            .map_or(-1, |index| index as i32)
    }

    fn size(&self) -> i32 {
        self.insns.len() as i32
    }

    fn subroutine_insns(&self, subroutine: Subroutine) -> &BitSet {
        match subroutine {
            Subroutine::Main => &self.main_subroutine_insns,
            Subroutine::Jsr(index) => &self.subroutines_insns[index].1,
        }
    }

    /// Determines, for each instruction, to which subroutine(s) it belongs.
    fn find_subroutines_insns(&mut self, try_catch_blocks: &[TryCatchBlockNode]) {
        // Find the instructions that belong to main subroutine.
        let mut visited_insns = BitSet::default();
        let mut main_subroutine_insns = std::mem::take(&mut self.main_subroutine_insns);
        self.find_subroutine_insns(
            0,
            &mut main_subroutine_insns,
            &mut visited_insns,
            try_catch_blocks,
        );
        self.main_subroutine_insns = main_subroutine_insns;
        // For each subroutine, find the instructions that belong to this subroutine. (ASM
        // iterates over a HashMap, in an order that does not change the result.)
        for index in 0..self.subroutines_insns.len() {
            let jsr_label_node = self.subroutines_insns[index].0;
            let mut subroutine_insns = std::mem::take(&mut self.subroutines_insns[index].1);
            self.find_subroutine_insns(
                self.index_of(jsr_label_node),
                &mut subroutine_insns,
                &mut visited_insns,
                try_catch_blocks,
            );
            self.subroutines_insns[index].1 = subroutine_insns;
        }
    }

    /// Finds the instructions that belong to the subroutine starting at the given instruction
    /// index. For this the control flow graph is visited with a depth first search (this
    /// includes the normal control flow and the exception handlers).
    ///
    /// - `start_insn_index`: the index of the first instruction of the subroutine.
    /// - `subroutine_insns`: where the indices of the instructions of the subroutine must be
    ///   stored.
    /// - `visited_insns`: the indices of the instructions that have been visited so far
    ///   (including in previous calls to this method). This bit set is updated by this method
    ///   each time a new instruction is visited. It is used to make sure each instruction is
    ///   visited at most once.
    fn find_subroutine_insns(
        &mut self,
        start_insn_index: i32,
        subroutine_insns: &mut BitSet,
        visited_insns: &mut BitSet,
        try_catch_blocks: &[TryCatchBlockNode],
    ) {
        // First find the instructions reachable via normal execution.
        self.find_reachable_insns(start_insn_index, subroutine_insns, visited_insns);

        // Then find the instructions reachable via the applicable exception handlers.
        loop {
            let mut applicable_handler_found = false;
            for try_catch_block_node in try_catch_blocks {
                // If the handler has already been processed, skip it.
                let handler_index = self.index_of(try_catch_block_node.handler);
                if subroutine_insns.get(handler_index) {
                    continue;
                }

                // If an instruction in the exception handler range belongs to the subroutine,
                // the handler can be reached from the routine, and its instructions must be added
                // to the subroutine.
                let start_index = self.index_of(try_catch_block_node.start);
                let end_index = self.index_of(try_catch_block_node.end);
                let first_subroutine_insn_after_try_catch_start =
                    subroutine_insns.next_set_bit(start_index);
                if first_subroutine_insn_after_try_catch_start >= start_index
                    && first_subroutine_insn_after_try_catch_start < end_index
                {
                    self.find_reachable_insns(handler_index, subroutine_insns, visited_insns);
                    applicable_handler_found = true;
                }
            }
            // If an applicable exception handler has been found, other handlers may become
            // applicable, so we must examine them again.
            if !applicable_handler_found {
                return;
            }
        }
    }

    /// Finds the instructions that are reachable from the given instruction, without following
    /// any JSR instruction nor any exception handler. For this the control flow graph is visited
    /// with a depth first search.
    ///
    /// Unlike ASM's, which recurses on each jump, this search keeps the jump targets that remain
    /// to be visited in a stack (ADR-06: no recursion on input-controlled depth). The result
    /// does not depend on the order in which the targets are visited.
    ///
    /// - `insn_index`: the index of an instruction of the subroutine.
    /// - `subroutine_insns`: where the indices of the instructions of the subroutine must be
    ///   stored.
    /// - `visited_insns`: the indices of the instructions that have been visited so far
    ///   (including in previous calls to this method). This bit set is updated by this method
    ///   each time a new instruction is visited. It is used to make sure each instruction is
    ///   visited at most once.
    fn find_reachable_insns(
        &mut self,
        insn_index: i32,
        subroutine_insns: &mut BitSet,
        visited_insns: &mut BitSet,
    ) {
        let mut pending = vec![insn_index];
        while let Some(mut current_insn_index) = pending.pop() {
            // We implicitly assume below that execution can always fall through to the next
            // instruction after a JSR. But a subroutine may never return, in which case the code
            // after the JSR is unreachable and can be anything. In particular, it can seem to
            // fall off the end of the method, so we must handle this case here (we could instead
            // detect whether execution can return or not from a JSR, but this is more
            // complicated).
            while current_insn_index < self.size() {
                // Visit each instruction at most once.
                if subroutine_insns.get(current_insn_index) {
                    break;
                }
                subroutine_insns.set(current_insn_index);

                // Check if this instruction has already been visited by another subroutine.
                if visited_insns.get(current_insn_index) {
                    self.shared_subroutine_insns.set(current_insn_index);
                }
                visited_insns.set(current_insn_index);

                let current_insn_node = &self.insns[current_insn_index as usize].insn;
                let mut targets = Vec::new();
                match current_insn_node {
                    // Don't follow JSR instructions in the control flow graph.
                    Insn::Jump { opcode, label } if *opcode != JSR => targets.push(*label),
                    insn @ (Insn::TableSwitch(_) | Insn::LookupSwitch(_)) => {
                        let (dflt, labels) = insn.switch_labels().expect("a switch");
                        targets.push(dflt);
                        targets.extend_from_slice(labels);
                    }
                    _ => {}
                }
                // Push in reverse order, so that the targets are visited in ASM's order.
                for label in targets.into_iter().rev() {
                    pending.push(self.index_of(label));
                }

                // Check if this instruction falls through to the next instruction; if not, stop.
                match current_insn_node.opcode() {
                    Some(
                        GOTO | RET | TABLESWITCH | LOOKUPSWITCH | IRETURN | LRETURN | FRETURN
                        | DRETURN | ARETURN | RETURN | ATHROW,
                    ) => {
                        // Note: this either returns from this subroutine, or from a parent
                        // subroutine.
                        break;
                    }
                    _ => {
                        // Go to the next instruction.
                        current_insn_index += 1;
                    }
                }
            }
        }
    }

    /// Creates the new instructions, inlining each instantiation of each subroutine until the
    /// code is fully elaborated.
    fn emit_code(
        &mut self,
        subroutine_indices: &FxHashMap<Label, usize>,
        method_node: &MethodNode,
    ) -> Emitted {
        let mut worklist = VecDeque::new();
        // Create an instantiation of the main "subroutine", which is just the main routine.
        worklist.push_back(self.new_instantiation(None, Subroutine::Main));

        // Emit instantiations of each subroutine we encounter, including the main subroutine.
        let mut emitted = (InsnList::new(), Vec::new(), Vec::new());
        while let Some(instantiation) = worklist.pop_front() {
            self.emit_instantiation(
                instantiation,
                &mut worklist,
                subroutine_indices,
                method_node,
                &mut emitted,
            );
        }
        emitted
    }

    /// Emits an instantiation of a subroutine, specified by `instantiation`. May add new
    /// instantiations that are invoked by this one to the `worklist`, and new try/catch blocks
    /// and local variables to `emitted`.
    fn emit_instantiation(
        &mut self,
        instantiation: usize,
        worklist: &mut VecDeque<usize>,
        subroutine_indices: &FxHashMap<Label, usize>,
        method_node: &MethodNode,
        emitted: &mut Emitted,
    ) {
        let (new_instructions, new_try_catch_blocks, new_local_variables) = emitted;
        let mut previous_label_node = None;
        for i in 0..self.insns.len() {
            let insn_node = self.insns[i];
            if let Insn::Label(label_node) = insn_node.insn {
                // Always clone all labels, while avoiding to add the same label more than once.
                let cloned_label_node = self.get_cloned_label(instantiation, label_node);
                if cloned_label_node != previous_label_node {
                    if let Some(cloned_label_node) = cloned_label_node {
                        new_instructions.add(Insn::Label(cloned_label_node));
                    }
                    previous_label_node = cloned_label_node;
                }
            } else if self.find_owner(instantiation, i as i32) == Some(instantiation) {
                // Don't emit instructions that were already emitted by an ancestor subroutine.
                // Note that it is still possible for a given instruction to be emitted twice
                // because it may belong to two subroutines that do not invoke each other.

                match insn_node.insn {
                    Insn::Var { opcode: RET, .. } => {
                        // Translate RET instruction(s) to a jump to the return label for the
                        // appropriate instantiation. The problem is that the subroutine may "fall
                        // through" to the ret of a parent subroutine; therefore, to find the
                        // appropriate ret label we find the oldest instantiation that claims to
                        // own this instruction.
                        let mut ret_label = None;
                        let mut ret_label_owner = Some(instantiation);
                        while let Some(owner) = ret_label_owner {
                            let owner = &self.instantiations[owner];
                            if self.subroutine_insns(owner.subroutine).get(i as i32) {
                                ret_label = owner.return_label;
                            }
                            ret_label_owner = owner.parent;
                        }
                        let Some(ret_label) = ret_label else {
                            // This is only possible if the mainSubroutine owns a RET
                            // instruction, which should never happen for verifiable code.
                            crate::support::illegal_argument(format_args!(
                                "Instruction #{i} is a RET not owned by any subroutine"
                            ));
                        };
                        new_instructions.add(Insn::Jump {
                            opcode: GOTO,
                            label: ret_label,
                        });
                    }
                    Insn::Jump {
                        opcode: JSR,
                        label: jsr_label_node,
                    } => {
                        let subroutine = Subroutine::Jsr(subroutine_indices[&jsr_label_node]);
                        let new_instantiation =
                            self.new_instantiation(Some(instantiation), subroutine);
                        let cloned_jsr_label_node =
                            self.get_cloned_label_for_jump_insn(new_instantiation, jsr_label_node);
                        // Replace the JSR instruction with a GOTO to the instantiated subroutine,
                        // and push NULL for what was once the return address value. This hack
                        // allows us to avoid doing any sort of data flow analysis to figure out
                        // which instructions manipulate the old return address value pointer
                        // which is now known to be unneeded.
                        new_instructions.add(Insn::Simple {
                            opcode: ACONST_NULL,
                        });
                        new_instructions.add(Insn::Jump {
                            opcode: GOTO,
                            label: cloned_jsr_label_node,
                        });
                        let return_label = self.instantiations[new_instantiation]
                            .return_label
                            .expect("a subroutine instantiation has a return label");
                        new_instructions.add(Insn::Label(return_label));
                        // Insert this new instantiation into the queue to be emitted later.
                        worklist.push_back(new_instantiation);
                    }
                    _ => {
                        let cloned_labels = self.cloned_labels_for(instantiation, &insn_node.insn);
                        let clone = new_instructions.add(insn_node.clone_with(&cloned_labels));
                        // ASM's clone also copies the type annotations of a real instruction.
                        if insn_node.opcode().is_some()
                            && let Some(annotations) = self
                                .instructions
                                .id_at(i)
                                .and_then(|id| self.instructions.type_annotations(id))
                        {
                            *new_instructions.type_annotations_mut(clone) = annotations.clone();
                        }
                    }
                }
            }
        }

        // Emit the try/catch blocks that are relevant for this instantiation.
        for try_catch_block_node in &method_node.try_catch_blocks {
            let start = self.get_cloned_label(instantiation, try_catch_block_node.start);
            let end = self.get_cloned_label(instantiation, try_catch_block_node.end);
            if start != end {
                let handler = self
                    .get_cloned_label_for_jump_insn(instantiation, try_catch_block_node.handler);
                let (Some(start), Some(end)) = (start, end) else {
                    panic!("java.lang.AssertionError: Internal error!");
                };
                new_try_catch_blocks.push(TryCatchBlockNode::new(
                    start,
                    end,
                    handler,
                    try_catch_block_node.type_,
                ));
            }
        }

        // Emit the local variable nodes that are relevant for this instantiation.
        for local_variable_node in &method_node.local_variables {
            let start = self.get_cloned_label(instantiation, local_variable_node.start);
            let end = self.get_cloned_label(instantiation, local_variable_node.end);
            if start != end {
                // ASM stores a null label here, and fails when the node is visited.
                let (Some(start), Some(end)) = (start, end) else {
                    panic!("java.lang.NullPointerException");
                };
                new_local_variables.push(LocalVariableNode::new(
                    local_variable_node.name,
                    local_variable_node.desc,
                    local_variable_node.signature,
                    start,
                    end,
                    local_variable_node.index,
                ));
            }
        }
    }

    /// Creates an instantiation of the given subroutine, invoked from `parent` (`None` for the
    /// instantiation of the main "subroutine"), and returns its index.
    fn new_instantiation(&mut self, parent: Option<usize>, subroutine: Subroutine) -> usize {
        let mut ancestor = parent;
        while let Some(index) = ancestor {
            let instantiation = &self.instantiations[index];
            if instantiation.subroutine == subroutine {
                crate::support::illegal_argument(format_args!(
                    "Recursive invocation of {}",
                    self.subroutine_insns(subroutine)
                ));
            }
            ancestor = instantiation.parent;
        }

        let this = self.instantiations.len();
        self.instantiations.push(Instantiation {
            parent,
            subroutine,
            cloned_labels: FxHashMap::default(),
            return_label: parent.map(|_| Label::new()),
        });

        // Create a clone of each label in the original code of the subroutine. Note that we
        // collapse labels which point at the same instruction into one.
        let mut cloned_labels = FxHashMap::default();
        let mut cloned_label_node = None;
        for (insn_index, insn_node) in self.insns.iter().enumerate() {
            if let Insn::Label(label_node) = insn_node.insn {
                // If we already have a label pointing at this spot, don't recreate it.
                let cloned = *cloned_label_node.get_or_insert_with(Label::new);
                cloned_labels.insert(label_node, cloned);
            } else if self.find_owner(this, insn_index as i32) == Some(this) {
                // We will emit this instruction, so clear the duplicateLabelNode flag since the
                // next Label will refer to a distinct instruction.
                cloned_label_node = None;
            }
        }
        self.instantiations[this].cloned_labels = cloned_labels;
        this
    }

    /// Returns the "owner" of a particular instruction relative to the given instantiation: the
    /// owner refers to the instantiation which will emit the version of this instruction that we
    /// will execute.
    ///
    /// Typically, the return value is either `instantiation` or `None`. `instantiation`
    /// indicates that this instantiation will generate the version of this instruction that we
    /// will execute, and `None` indicates that this instantiation never executes the given
    /// instruction.
    ///
    /// Sometimes, however, an instruction can belong to multiple subroutines; this is called a
    /// shared instruction, and occurs when multiple subroutines branch to common points of
    /// control. In this case, the owner is the oldest instantiation which owns the instruction in
    /// question (parent instantiations are older than their children).
    fn find_owner(&self, instantiation: usize, insn_index: i32) -> Option<usize> {
        let this = &self.instantiations[instantiation];
        if !self.subroutine_insns(this.subroutine).get(insn_index) {
            return None;
        }
        if !self.shared_subroutine_insns.get(insn_index) {
            return Some(instantiation);
        }
        let mut owner = instantiation;
        let mut ancestor = this.parent;
        while let Some(index) = ancestor {
            let instantiation = &self.instantiations[index];
            if self
                .subroutine_insns(instantiation.subroutine)
                .get(insn_index)
            {
                owner = index;
            }
            ancestor = instantiation.parent;
        }
        Some(owner)
    }

    /// Returns the clone of the given original label that is appropriate for use in a jump
    /// instruction.
    fn get_cloned_label_for_jump_insn(&self, instantiation: usize, label_node: Label) -> Label {
        // findOwner should never return null, because owner is null only if an instruction
        // cannot be reached from this subroutine.
        let Some(owner) = self.find_owner(instantiation, self.index_of(label_node)) else {
            panic!("java.lang.NullPointerException");
        };
        // ASM's map returns null for a label it does not contain, which fails later.
        let Some(&cloned) = self.instantiations[owner].cloned_labels.get(&label_node) else {
            panic!("java.lang.NullPointerException");
        };
        cloned
    }

    /// Returns the clone of the given original label that is appropriate for use by a try/catch
    /// block or a variable annotation (`None` for a label that is not in the original code).
    fn get_cloned_label(&self, instantiation: usize, label_node: Label) -> Option<Label> {
        self.instantiations[instantiation]
            .cloned_labels
            .get(&label_node)
            .copied()
    }

    /// The image of the labels of the given instruction by the instantiation (ASM's
    /// `Instantiation`, as a `Map` for `AbstractInsnNode.clone`, whose `get` is
    /// `getClonedLabelForJumpInsn`).
    fn cloned_labels_for(&self, instantiation: usize, insn: &Insn) -> FxHashMap<Label, Label> {
        let mut cloned_labels = FxHashMap::default();
        let mut add = |label: Label| {
            cloned_labels.insert(
                label,
                self.get_cloned_label_for_jump_insn(instantiation, label),
            );
        };
        match insn {
            Insn::Jump { label, .. } => add(*label),
            Insn::TableSwitch(_) | Insn::LookupSwitch(_) => {
                let (dflt, labels) = insn.switch_labels().expect("a switch");
                add(dflt);
                labels.iter().for_each(|&label| add(label));
            }
            Insn::Frame(frame) => {
                for value in frame.local().iter().chain(frame.stack()) {
                    if let oxasm_tree::SymFrameValue::Uninitialized(label) = value {
                        add(*label);
                    }
                }
            }
            Insn::Line { start, .. } => add(*start),
            _ => debug_assert!(!matches!(insn.kind(), InsnKind::Label)),
        }
        cloned_labels
    }
}

/// An instantiation of a subroutine.
#[derive(Debug)]
struct Instantiation {
    /// The instantiation from which this one was created (or `None` for the instantiation of
    /// the main "subroutine").
    parent: Option<usize>,
    /// The subroutine which is instantiated: its original instructions are those of its bit
    /// set.
    subroutine: Subroutine,
    /// A map from labels from the original code to labels pointing at code specific to this
    /// instantiation, for use in remapping try/catch blocks, as well as jumps.
    ///
    /// Note that in the presence of instructions belonging to several subroutines, we map the
    /// target label of a GOTO to the label used by the oldest instantiation (parent
    /// instantiations are older than their children). This avoids code duplication during
    /// inlining in most cases.
    cloned_labels: FxHashMap<Label, Label>,
    /// The return label for this instantiation, to which all original returns will be mapped.
    return_label: Option<Label>,
}

/// The subset of Java's `BitSet` that the inliner uses, with its exceptions on negative
/// indices.
#[derive(Clone, Default, PartialEq, Eq)]
struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    /// Java's `BitSet.get`.
    fn get(&self, index: i32) -> bool {
        if index < 0 {
            panic!("java.lang.IndexOutOfBoundsException: bitIndex < 0: {index}");
        }
        let index = index as usize;
        self.words
            .get(index / 64)
            .is_some_and(|word| word & (1 << (index % 64)) != 0)
    }

    /// Java's `BitSet.set`.
    fn set(&mut self, index: i32) {
        if index < 0 {
            panic!("java.lang.IndexOutOfBoundsException: bitIndex < 0: {index}");
        }
        let index = index as usize;
        if index / 64 >= self.words.len() {
            self.words.resize(index / 64 + 1, 0);
        }
        self.words[index / 64] |= 1 << (index % 64);
    }

    /// Java's `BitSet.nextSetBit`: the index of the first bit that is set at or after the given
    /// index, or -1.
    fn next_set_bit(&self, from_index: i32) -> i32 {
        if from_index < 0 {
            panic!("java.lang.IndexOutOfBoundsException: fromIndex < 0: {from_index}");
        }
        let from_index = from_index as usize;
        let mut word_index = from_index / 64;
        let Some(&first) = self.words.get(word_index) else {
            return -1;
        };
        let mut word = first & (u64::MAX << (from_index % 64));
        loop {
            if word != 0 {
                return (word_index * 64 + word.trailing_zeros() as usize) as i32;
            }
            word_index += 1;
            match self.words.get(word_index) {
                Some(&next) => word = next,
                None => return -1,
            }
        }
    }
}

/// Java's `BitSet.toString`: `{0, 2, 4}`.
impl fmt::Display for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        let mut index = self.next_set_bit(0);
        let mut first = true;
        while index >= 0 {
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            write!(f, "{index}")?;
            index = self.next_set_bit(index + 1);
        }
        f.write_str("}")
    }
}

impl fmt::Debug for BitSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_set_is_java_s() {
        let mut bits = BitSet::default();
        assert_eq!(bits.to_string(), "{}");
        assert_eq!(bits.next_set_bit(0), -1);
        for index in [0, 2, 63, 64, 130] {
            bits.set(index);
        }
        assert_eq!(bits.to_string(), "{0, 2, 63, 64, 130}");
        assert_eq!(bits.next_set_bit(3), 63);
        assert_eq!(bits.next_set_bit(65), 130);
        assert_eq!(bits.next_set_bit(131), -1);
        assert_eq!(bits.next_set_bit(1000), -1);
        assert!(bits.get(64) && !bits.get(65) && !bits.get(10_000));
    }
}
