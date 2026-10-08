//! [`TryCatchBlockSorter`]: ASM's `TryCatchBlockSorter`.

use std::fmt;
use std::sync::Arc;

use oxasm::{JavaStr, MethodVisitor};
use oxasm_tree::{MethodNode, TryCatchBlockNode};

/// A [`MethodVisitor`] adapter to sort the exception handlers. The handlers are sorted in a
/// method innermost-to-outermost. This allows the programmer to add handlers without worrying
/// about ordering them correctly with respect to existing, in-code handlers.
///
/// Behavior is only defined for properly-nested handlers. If any "try" blocks overlap (something
/// that isn't possible in Java code) then this may not do what you want. In fact, this adapter
/// just sorts by the length of the "try" block, taking advantage of the fact that a given try
/// block must be larger than any block it contains).
///
/// ASM's `TryCatchBlockSorter` extends `MethodNode`; this one holds a [`MethodNode`] (ADR-13),
/// into which it forwards every event, and which it sorts and makes the downstream visitor visit
/// at [`visit_end`](MethodVisitor::visit_end).
pub struct TryCatchBlockSorter<'a> {
    /// The method, as visited so far.
    method_node: MethodNode,
    /// The method visitor to which the sorted method is sent at `visit_end`.
    mv: Option<Box<dyn MethodVisitor + 'a>>,
}

impl<'a> TryCatchBlockSorter<'a> {
    /// Constructs a new [`TryCatchBlockSorter`].
    ///
    /// - `method_visitor`: the method visitor to which this visitor must delegate method calls,
    ///   or `None`.
    /// - `access`: the method's access flags (see [`oxasm::opcodes`]). This parameter also
    ///   indicates if the method is synthetic and/or deprecated.
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`oxasm::Type`]).
    /// - `signature`: the method's signature. `None` if the method parameters, return type and
    ///   exceptions do not use generic types.
    /// - `exceptions`: the internal names of the method's exception classes.
    pub fn new(
        method_visitor: Option<Box<dyn MethodVisitor + 'a>>,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Self {
        TryCatchBlockSorter {
            method_node: MethodNode::from_strings(
                Arc::default(),
                access,
                name,
                descriptor,
                signature,
                exceptions,
            ),
            mv: method_visitor,
        }
    }

    /// The method node this sorter fills (and sorts, at `visit_end`).
    pub fn method_node(&self) -> &MethodNode {
        &self.method_node
    }

    /// Returns the method node this sorter fills (and sorts, at `visit_end`).
    pub fn into_method_node(self) -> MethodNode {
        self.method_node
    }
}

impl fmt::Debug for TryCatchBlockSorter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TryCatchBlockSorter")
            .field("method_node", &self.method_node)
            .finish_non_exhaustive()
    }
}

impl MethodVisitor for TryCatchBlockSorter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut self.method_node)
    }

    fn visit_end(&mut self) {
        // Sort the TryCatchBlockNode elements by the length of their "try" block (a stable sort,
        // like Java's Collections.sort).
        let instructions = &self.method_node.instructions;
        // ASM's indexOf of a label node that is not in the list is the node's initial index, -1.
        let index_of = |label| instructions.label_index(label).map_or(-1, |i| i as i64);
        let block_length = |block: &TryCatchBlockNode| index_of(block.end) - index_of(block.start);
        self.method_node
            .try_catch_blocks
            .sort_by_cached_key(block_length);
        // Update the 'target' of each try catch block annotation.
        for (i, try_catch_block) in self.method_node.try_catch_blocks.iter_mut().enumerate() {
            try_catch_block.update_index(i);
        }
        if let Some(mv) = &mut self.mv {
            self.method_node.accept_method_visitor(&mut **mv);
        }
    }
}
