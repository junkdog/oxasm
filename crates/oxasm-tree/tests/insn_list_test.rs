//! Port of ASM's `InsnListTest`, plus oxasm-specific tests of the handles (ADR-11) and a
//! property test of random edits against a `Vec` model.
//!
//! Java's tests use `CheckedInsnList`, which throws `NoSuchElementException` for a node that is
//! not in the list, and `IllegalArgumentException` for a node already in a list or a list added
//! to itself. Here accessors return `None` and mutators panic for a handle that is not an
//! instruction of the list, and the other two cases cannot be written (a list owns its nodes,
//! and the borrow checker rejects adding a list to itself):
//!
//! - `testAdd_illegalArgument`, `testAddList_illegalArgument`, `testInsert_illegalArgument` and
//!   `testInsertList_illegalArgument` are dropped;
//! - `testIterator_cacheIsNull` is dropped: the position cache is private;
//! - `testResetLabels` is dropped: labels are plain ids with no per-writer state (ADR-03), and
//!   `resetLabels` is gone.

use oxasm::opcodes::HandleKind;
use oxasm::{FrameKind, Interner, Label, MethodVisitor};
use oxasm_tree::{
    FrameNode, Insn, InsnId, InsnList, InsnNode, SymConstant, SymConstantDynamic, SymFrameValue,
    SymHandle,
};
use proptest::prelude::*;

/// ASM's `new InsnNode(opcode)`.
fn new_insn(opcode: u8) -> InsnNode {
    Insn::Simple { opcode }.into()
}

fn new_insn_list() -> InsnList {
    InsnList::new()
}

/// ASM's `newInsnList(insn1, insn2)`, with `insn1 = new InsnNode(0)` and `insn2 = new InsnNode(1)`:
/// the list and the handles of the two instructions.
fn new_insn_list_2() -> (InsnList, InsnId, InsnId) {
    let mut insn_list = InsnList::new();
    let insn1 = insn_list.add(new_insn(0));
    let insn2 = insn_list.add(new_insn(1));
    (insn_list, insn1, insn2)
}

/// A handle that is not an instruction of any list used by a test (ASM's `new InsnNode(0)`
/// passed where a node of the list is expected).
fn foreign() -> InsnId {
    InsnList::new().add(new_insn(0))
}

fn ids(insn_list: &InsnList) -> Vec<InsnId> {
    insn_list.ids().collect()
}

fn opcodes(insn_list: &InsnList) -> Vec<u8> {
    insn_list
        .iter()
        .map(|node| node.opcode().unwrap())
        .collect()
}

#[test]
fn test_size_empty_list() {
    assert_eq!(new_insn_list().len(), 0);
}

#[test]
fn test_get_first_empty_list() {
    assert_eq!(new_insn_list().first(), None);
}

#[test]
fn test_get_last_empty_list() {
    assert_eq!(new_insn_list().last(), None);
}

#[test]
fn test_get_out_of_bounds() {
    assert_eq!(new_insn_list().get(0), None);
    assert_eq!(new_insn_list().id_at(0), None);
}

#[test]
fn test_contains() {
    assert!(!new_insn_list().contains(foreign()));
}

#[test]
fn test_index_of_no_such_element() {
    let insn_list = new_insn_list();

    assert_eq!(insn_list.index_of(foreign()), None);
}

#[test]
fn test_index_of() {
    let (insn_list, insn1, insn2) = new_insn_list_2();

    let index1 = insn_list.index_of(insn1);
    let index2 = insn_list.index_of(insn2);

    assert_eq!(index1, Some(0));
    assert_eq!(index2, Some(1));
}

#[test]
fn test_accept_clone_list_visitor() {
    struct CloneVisitor<'a>(&'a mut InsnList);
    impl MethodVisitor for CloneVisitor<'_> {
        fn visit_insn(&mut self, opcode: u8) {
            self.0.add(new_insn(opcode));
        }
    }
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(55));
    insn_list.add(new_insn(77));
    let mut dst_insn_list = InsnList::new();

    insn_list.accept(&Interner::new(), &mut CloneVisitor(&mut dst_insn_list));

    assert_eq!(dst_insn_list.len(), 2);
    assert_eq!(dst_insn_list.get(0).unwrap().opcode(), Some(55));
    assert_eq!(dst_insn_list.get(1).unwrap().opcode(), Some(77));
}

#[test]
fn test_iterator_next_no_such_element() {
    let mut insn_list = new_insn_list();
    let mut iterator = insn_list.cursor(0);

    assert_eq!(iterator.next(), None);
}

#[test]
fn test_iterator_next_non_empty_list() {
    let (mut insn_list, _, insn2) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);

    let insn = iterator.next();

    assert_eq!(insn, Some(insn2));
    assert!(!iterator.has_next());
    assert_eq!(iterator.next_index(), 2);
    assert!(iterator.has_previous());
    assert_eq!(iterator.previous_index(), Some(1));
}

#[test]
fn test_iterator_previous_no_such_element() {
    let mut insn_list = new_insn_list();
    let mut iterator = insn_list.cursor(0);

    assert_eq!(iterator.previous(), None);
}

#[test]
fn test_iterator_previous_non_empty_list() {
    let (mut insn_list, insn1, _) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);

    let insn = iterator.previous();

    assert_eq!(insn, Some(insn1));
    assert!(iterator.has_next());
    assert_eq!(iterator.next_index(), 0);
    assert!(!iterator.has_previous());
    assert_eq!(iterator.previous_index(), None);
}

#[test]
fn test_iterator_add_empty_list() {
    let mut insn_list = new_insn_list();

    let insn = insn_list.cursor(0).add(new_insn(0));

    assert_eq!(ids(&insn_list), [insn]);
}

#[test]
fn test_iterator_add_first_insn() {
    let (mut insn_list, insn1, insn2) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);

    let insn = iterator.add(new_insn(0));

    assert_eq!(ids(&insn_list), [insn1, insn, insn2]);
}

#[test]
#[should_panic(expected = "no instruction to remove")]
fn test_iterator_remove_illegal_state() {
    let (mut insn_list, _, _) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);

    iterator.remove();
}

#[test]
fn test_iterator_remove_after_next() {
    let (mut insn_list, insn1, _) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);
    iterator.next();

    iterator.remove();

    assert_eq!(ids(&insn_list), [insn1]);
}

#[test]
fn test_iterator_remove_after_previous() {
    let (mut insn_list, _, insn2) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);
    iterator.previous();

    iterator.remove();

    assert_eq!(ids(&insn_list), [insn2]);
}

#[test]
fn test_iterator_add_last_insn() {
    let (mut insn_list, insn1, insn2) = new_insn_list_2();
    let mut iterator = insn_list.cursor(2);

    let insn = iterator.add(new_insn(0));

    assert_eq!(ids(&insn_list), [insn1, insn2, insn]);
}

#[test]
#[should_panic(expected = "no instruction to replace")]
fn test_iterator_set_illegal_state() {
    let (mut insn_list, _, _) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);

    iterator.set(new_insn(0));
}

#[test]
fn test_iterator_set_after_next() {
    let (mut insn_list, insn1, _) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);
    iterator.next();

    let (insn, _) = iterator.set(new_insn(0));

    assert_eq!(ids(&insn_list), [insn1, insn]);
}

#[test]
fn test_iterator_set_after_previous() {
    let (mut insn_list, _, insn2) = new_insn_list_2();
    let mut iterator = insn_list.cursor(1);
    iterator.previous();

    let (insn, _) = iterator.set(new_insn(0));

    assert_eq!(ids(&insn_list), [insn, insn2]);
}

#[test]
fn test_to_array_empty_list() {
    assert_eq!(new_insn_list().ids().len(), 0);
}

#[test]
fn test_to_array_non_empty_list() {
    let (insn_list, insn1, insn2) = new_insn_list_2();

    let insn_array = ids(&insn_list);

    assert_eq!(insn_array, [insn1, insn2]);
}

#[test]
#[should_panic(expected = "is not an instruction of this list")]
fn test_set_no_such_element() {
    let mut insn_list = new_insn_list();

    insn_list.set(foreign(), new_insn(0));
}

#[test]
fn test_set_single_insn() {
    let mut insn_list = new_insn_list();
    let insn1 = insn_list.add(new_insn(0));

    let (insn, _) = insn_list.set(insn1, new_insn(0));

    assert_eq!(insn_list.len(), 1);
    assert_eq!(insn_list.first(), Some(insn));
}

#[test]
fn test_set_first_insn() {
    let (mut insn_list, insn1, _) = new_insn_list_2();

    let (insn, _) = insn_list.set(insn1, new_insn(0));

    assert_eq!(insn_list.len(), 2);
    assert_eq!(insn_list.first(), Some(insn));
}

#[test]
fn test_set_last_insn() {
    let (mut insn_list, _, insn2) = new_insn_list_2();
    let _ = insn_list.get(0); // Builds the position cache, as ASM's toArray() does.

    let (insn, _) = insn_list.set(insn2, new_insn(0));

    assert_eq!(insn_list.len(), 2);
    assert_eq!(insn_list.last(), Some(insn));
    // oxasm-specific: the position cache was updated, not left stale.
    assert_eq!(insn_list.index_of(insn), Some(1));
    assert_eq!(insn_list.id_at(1), Some(insn));
}

#[test]
fn test_add_in_empty_list() {
    let mut insn_list = new_insn_list();

    let insn = insn_list.add(new_insn(0));

    assert_eq!(insn_list.len(), 1);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(insn_list.id_at(0), Some(insn));
    assert!(insn_list.contains(insn));
    assert_eq!(insn_list.index_of(insn), Some(0));
    assert_eq!(ids(&insn_list), [insn]);
    assert_eq!(insn_list.previous(insn), None);
    assert_eq!(insn_list.next(insn), None);
}

#[test]
fn test_add_in_non_empty_list() {
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(0));

    let insn = insn_list.add(new_insn(0));

    assert_eq!(insn_list.len(), 2);
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(insn_list.index_of(insn), Some(1));
    assert_eq!(insn_list.id_at(1), Some(insn));
    assert!(insn_list.contains(insn));
}

#[test]
fn test_add_list_in_empty_list_empty_list() {
    let mut insn_list = new_insn_list();

    let moved = insn_list.add_list(&mut new_insn_list());

    assert_eq!(moved, None);
    assert_eq!(insn_list.len(), 0);
    assert_eq!(insn_list.first(), None);
    assert_eq!(insn_list.last(), None);
    assert_eq!(ids(&insn_list), []);
}

#[test]
fn test_add_list_in_empty_list_non_empty_list() {
    let mut insn_list = new_insn_list();
    let (mut other, old_insn1, _) = new_insn_list_2();

    let (insn1, insn2) = insn_list.add_list(&mut other).unwrap();

    assert_eq!(insn_list.len(), 2);
    assert_eq!(insn_list.first(), Some(insn1));
    assert_eq!(insn_list.last(), Some(insn2));
    assert_eq!(insn_list.id_at(0), Some(insn1));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn1), Some(0));
    assert_eq!(insn_list.index_of(insn2), Some(1));
    assert_eq!(ids(&insn_list), [insn1, insn2]);
    assert_eq!(opcodes(&insn_list), [0, 1]);
    // The source list is cleared, and its handles are stale.
    assert!(other.is_empty());
    assert!(!other.contains(old_insn1));
    assert!(!insn_list.contains(old_insn1));
}

#[test]
fn test_add_list_in_non_empty_list_non_empty_list() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));

    let (insn1, insn2) = insn_list.add_list(&mut new_insn_list_2().0).unwrap();

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.last(), Some(insn2));
    assert_eq!(insn_list.id_at(0), Some(insn));
    assert!(insn_list.contains(insn));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn), Some(0));
    assert_eq!(insn_list.index_of(insn1), Some(1));
    assert_eq!(insn_list.index_of(insn2), Some(2));
    assert_eq!(ids(&insn_list), [insn, insn1, insn2]);
}

#[test]
fn test_insert_in_empty_list() {
    let mut insn_list = new_insn_list();

    let insn = insn_list.insert(new_insn(0));

    assert_eq!(insn_list.len(), 1);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(insn_list.id_at(0), Some(insn));
    assert!(insn_list.contains(insn));
    assert_eq!(insn_list.index_of(insn), Some(0));
    assert_eq!(ids(&insn_list), [insn]);
}

#[test]
fn test_insert_in_non_empty_list() {
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(0));

    let insn = insn_list.insert(new_insn(0));

    assert_eq!(insn_list.len(), 2);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.id_at(0), Some(insn));
    assert!(insn_list.contains(insn));
    assert_eq!(insn_list.index_of(insn), Some(0));
}

#[test]
fn test_insert_list_in_empty_list_empty_list() {
    let mut insn_list = new_insn_list();

    let moved = insn_list.insert_list(&mut new_insn_list());

    assert_eq!(moved, None);
    assert_eq!(insn_list.len(), 0);
    assert_eq!(insn_list.first(), None);
    assert_eq!(insn_list.last(), None);
    assert_eq!(ids(&insn_list), []);
}

#[test]
fn test_insert_list_in_empty_list_non_empty_list() {
    let mut insn_list = new_insn_list();

    let (insn1, insn2) = insn_list.insert_list(&mut new_insn_list_2().0).unwrap();

    assert_eq!(insn_list.len(), 2);
    assert_eq!(insn_list.first(), Some(insn1));
    assert_eq!(insn_list.last(), Some(insn2));
    assert_eq!(insn_list.id_at(0), Some(insn1));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn1), Some(0));
    assert_eq!(insn_list.index_of(insn2), Some(1));
    assert_eq!(ids(&insn_list), [insn1, insn2]);
}

#[test]
fn test_insert_list_in_non_empty_list_non_empty_list() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));

    let (insn1, insn2) = insn_list.insert_list(&mut new_insn_list_2().0).unwrap();

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn1));
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(insn_list.id_at(0), Some(insn1));
    assert!(insn_list.contains(insn));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn1), Some(0));
    assert_eq!(insn_list.index_of(insn2), Some(1));
    assert_eq!(insn_list.index_of(insn), Some(2));
    assert_eq!(ids(&insn_list), [insn1, insn2, insn]);
}

#[test]
#[should_panic(expected = "is not an instruction of this list")]
fn test_insert_after_no_such_element() {
    let (mut insn_list, _, _) = new_insn_list_2();

    insn_list.insert_after(foreign(), new_insn(0));
}

#[test]
fn test_insert_after_last_insn() {
    let (mut insn_list, insn1, insn2) = new_insn_list_2();

    let insn = insn_list.insert_after(insn2, new_insn(0));

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn1));
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(insn_list.id_at(0), Some(insn1));
    assert!(insn_list.contains(insn));
    assert_eq!(insn_list.index_of(insn), Some(2));
    assert_eq!(ids(&insn_list), [insn1, insn2, insn]);
}

#[test]
fn test_insert_after_not_last_insn() {
    let (mut insn_list, insn1, insn2) = new_insn_list_2();

    let insn = insn_list.insert_after(insn1, new_insn(0));

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn1));
    assert_eq!(insn_list.last(), Some(insn2));
    assert_eq!(insn_list.id_at(0), Some(insn1));
    assert!(insn_list.contains(insn));
    assert_eq!(insn_list.index_of(insn), Some(1));
    assert_eq!(ids(&insn_list), [insn1, insn, insn2]);
}

#[test]
#[should_panic(expected = "is not an instruction of this list")]
fn test_insert_list_after_no_such_element() {
    let (mut insn_list, _, _) = new_insn_list_2();

    insn_list.insert_list_after(foreign(), &mut new_insn_list());
}

#[test]
fn test_insert_list_after_last_insn_empty_list() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));

    insn_list.insert_list_after(insn, &mut new_insn_list());

    assert_eq!(insn_list.len(), 1);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(ids(&insn_list), [insn]);
}

#[test]
fn test_insert_list_after_last_insn_non_empty_list() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));

    let (insn1, insn2) = insn_list
        .insert_list_after(insn, &mut new_insn_list_2().0)
        .unwrap();

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.last(), Some(insn2));
    assert_eq!(insn_list.id_at(0), Some(insn));
    assert!(insn_list.contains(insn));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn), Some(0));
    assert_eq!(insn_list.index_of(insn1), Some(1));
    assert_eq!(insn_list.index_of(insn2), Some(2));
    assert_eq!(ids(&insn_list), [insn, insn1, insn2]);
}

#[test]
fn test_insert_list_after_not_last_insn_non_empty_list() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));
    insn_list.add(new_insn(0));

    let (insn1, insn2) = insn_list
        .insert_list_after(insn, &mut new_insn_list_2().0)
        .unwrap();

    assert_eq!(insn_list.len(), 4);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.id_at(0), Some(insn));
    assert!(insn_list.contains(insn));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn), Some(0));
    assert_eq!(insn_list.index_of(insn1), Some(1));
    assert_eq!(insn_list.index_of(insn2), Some(2));
}

#[test]
#[should_panic(expected = "is not an instruction of this list")]
fn test_insert_before_no_such_element() {
    let (mut insn_list, _, _) = new_insn_list_2();

    insn_list.insert_before(foreign(), new_insn(0));
}

#[test]
fn test_insert_before_first_insn() {
    let (mut insn_list, insn1, insn2) = new_insn_list_2();

    let insn = insn_list.insert_before(insn1, new_insn(0));

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.last(), Some(insn2));
    assert_eq!(insn_list.id_at(0), Some(insn));
    assert!(insn_list.contains(insn));
    assert_eq!(insn_list.index_of(insn), Some(0));
    assert_eq!(ids(&insn_list), [insn, insn1, insn2]);
}

#[test]
fn test_insert_before_not_first_insn() {
    let (mut insn_list, insn1, insn2) = new_insn_list_2();

    let insn = insn_list.insert_before(insn2, new_insn(0));

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn1));
    assert_eq!(insn_list.last(), Some(insn2));
    assert_eq!(insn_list.id_at(1), Some(insn));
    assert!(insn_list.contains(insn));
    assert_eq!(insn_list.index_of(insn), Some(1));
    assert_eq!(ids(&insn_list), [insn1, insn, insn2]);
}

#[test]
#[should_panic(expected = "is not an instruction of this list")]
fn test_insert_list_before_no_such_element() {
    let (mut insn_list, _, _) = new_insn_list_2();

    insn_list.insert_list_before(foreign(), &mut new_insn_list());
}

#[test]
fn test_insert_list_before_first_insn_empty_list() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));

    insn_list.insert_list_before(insn, &mut new_insn_list());

    assert_eq!(insn_list.len(), 1);
    assert_eq!(insn_list.first(), Some(insn));
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(ids(&insn_list), [insn]);
}

#[test]
fn test_insert_list_before_first_insn_non_empty_list() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));

    let (insn1, insn2) = insn_list
        .insert_list_before(insn, &mut new_insn_list_2().0)
        .unwrap();

    assert_eq!(insn_list.len(), 3);
    assert_eq!(insn_list.first(), Some(insn1));
    assert_eq!(insn_list.last(), Some(insn));
    assert_eq!(insn_list.id_at(0), Some(insn1));
    assert!(insn_list.contains(insn));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn), Some(2));
    assert_eq!(insn_list.index_of(insn1), Some(0));
    assert_eq!(insn_list.index_of(insn2), Some(1));
    assert_eq!(ids(&insn_list), [insn1, insn2, insn]);
}

#[test]
fn test_insert_list_before_not_first_insn_non_empty_list() {
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(0));
    let insn = insn_list.add(new_insn(0));

    let (insn1, insn2) = insn_list
        .insert_list_before(insn, &mut new_insn_list_2().0)
        .unwrap();

    assert_eq!(insn_list.len(), 4);
    assert_eq!(insn_list.id_at(1), Some(insn1));
    assert_eq!(insn_list.id_at(2), Some(insn2));
    assert!(insn_list.contains(insn));
    assert!(insn_list.contains(insn1));
    assert!(insn_list.contains(insn2));
    assert_eq!(insn_list.index_of(insn), Some(3));
    assert_eq!(insn_list.index_of(insn1), Some(1));
    assert_eq!(insn_list.index_of(insn2), Some(2));
}

#[test]
#[should_panic(expected = "is not an instruction of this list")]
fn test_remove_no_such_element() {
    let (mut insn_list, _, _) = new_insn_list_2();

    insn_list.remove(foreign());
}

#[test]
fn test_remove_single_insn() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));

    insn_list.remove(insn);

    assert_eq!(insn_list.len(), 0);
    assert_eq!(insn_list.first(), None);
    assert_eq!(insn_list.last(), None);
    assert!(!insn_list.contains(insn));
    assert_eq!(ids(&insn_list), []);
    assert_eq!(insn_list.previous(insn), None);
    assert_eq!(insn_list.next(insn), None);
}

#[test]
fn test_remove_first_insn() {
    let mut insn_list = new_insn_list();
    let insn = insn_list.add(new_insn(0));
    insn_list.add(new_insn(0));

    insn_list.remove(insn);

    assert!(!insn_list.contains(insn));
    assert_eq!(insn_list.previous(insn), None);
    assert_eq!(insn_list.next(insn), None);
}

#[test]
fn test_remove_middle_insn() {
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(0));
    let insn = insn_list.add(new_insn(0));
    insn_list.add(new_insn(0));

    insn_list.remove(insn);

    assert!(!insn_list.contains(insn));
    assert_eq!(insn_list.previous(insn), None);
    assert_eq!(insn_list.next(insn), None);
}

#[test]
fn test_remove_last_insn() {
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(0));
    let insn = insn_list.add(new_insn(0));

    insn_list.remove(insn);

    assert!(!insn_list.contains(insn));
    assert_eq!(insn_list.previous(insn), None);
    assert_eq!(insn_list.next(insn), None);
}

#[test]
fn test_clear() {
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(0));
    let insn = insn_list.add(new_insn(0));
    insn_list.add(new_insn(0));

    insn_list.clear();

    assert_eq!(insn_list.len(), 0);
    assert_eq!(insn_list.first(), None);
    assert_eq!(insn_list.last(), None);
    assert!(!insn_list.contains(insn));
    assert_eq!(ids(&insn_list), []);
    assert_eq!(insn_list.previous(insn), None);
    assert_eq!(insn_list.next(insn), None);
}

// -------------------------------------------------------------------------------------------------
// oxasm-specific tests
// -------------------------------------------------------------------------------------------------

/// A removed instruction's handle stays stale when its slot is reused: it never designates the
/// new instruction.
#[test]
fn stale_handles_never_designate_another_instruction() {
    let (mut insn_list, insn1, insn2) = new_insn_list_2();
    let removed = insn_list.remove(insn1);
    assert_eq!(removed.opcode(), Some(0));
    let insn3 = insn_list.add(new_insn(3)); // reuses insn1's slot
    assert_ne!(insn1, insn3);
    assert!(!insn_list.contains(insn1));
    assert_eq!(insn_list.node(insn1), None);
    assert_eq!(insn_list.index_of(insn1), None);
    assert_eq!(insn_list[insn3].opcode(), Some(3));
    assert_eq!(ids(&insn_list), [insn2, insn3]);

    // A handle of another list, even with the same slot and generation, is not an instruction
    // of this list.
    let (other, other1, _) = new_insn_list_2();
    assert!(other.contains(other1));
    assert!(!insn_list.contains(other1));

    // Nor is a handle of a cleared list, or of the original of a clone.
    let clone = insn_list.clone();
    assert_eq!(clone, insn_list);
    assert!(!clone.contains(insn2));
    insn_list.clear();
    let insn4 = insn_list.add(new_insn(4));
    assert!(!insn_list.contains(insn2));
    assert!(insn_list.contains(insn4));
}

#[test]
fn label_position_finds_label_nodes() {
    let (l1, l2, l3) = (Label::new(), Label::new(), Label::new());
    let mut insn_list = new_insn_list();
    insn_list.add(new_insn(0));
    let n1 = insn_list.add(Insn::Label(l1));
    insn_list.add(Insn::Jump {
        opcode: oxasm::opcodes::GOTO,
        label: l2,
    });
    let n2 = insn_list.add(Insn::Label(l2));
    assert_eq!(insn_list.label_position(l1), Some(n1));
    assert_eq!(insn_list.label_index(l2), Some(3));
    assert_eq!(insn_list.label_position(l3), None);

    // The cache follows structural changes, and edits through node_mut.
    let n0 = insn_list.insert(Insn::Label(l3));
    assert_eq!(insn_list.label_position(l3), Some(n0));
    assert_eq!(insn_list.label_index(l2), Some(4));
    insn_list.remove(n2);
    assert_eq!(insn_list.label_position(l2), None);
    insn_list[n1].insn = Insn::Label(l2);
    assert_eq!(insn_list.label_position(l1), None);
    assert_eq!(insn_list.label_position(l2), Some(n1));
}

#[test]
fn iterators_are_double_ended_and_exact() {
    let (insn_list, insn1, insn2) = new_insn_list_2();
    assert_eq!(insn_list.ids().rev().collect::<Vec<_>>(), [insn2, insn1]);
    assert_eq!(
        insn_list
            .iter()
            .rev()
            .map(|n| n.opcode().unwrap())
            .collect::<Vec<_>>(),
        [1, 0]
    );
    let mut iter = insn_list.iter();
    assert_eq!(iter.len(), 2);
    iter.next();
    assert_eq!(iter.len(), 1);
    assert_eq!(iter.next_back().unwrap().opcode(), Some(1));
    assert!(iter.next().is_none());
    assert_eq!(
        insn_list
            .iter_with_ids()
            .map(|(id, n)| (id, n.opcode()))
            .collect::<Vec<_>>(),
        [(insn1, Some(0)), (insn2, Some(1))]
    );
}

/// A cursor walk that removes every other instruction and replaces the rest.
#[test]
fn cursor_edits_while_walking() {
    let mut insn_list = new_insn_list();
    for opcode in 0..10 {
        insn_list.add(new_insn(opcode));
    }
    let mut cursor = insn_list.cursor(0);
    while let Some(id) = cursor.next() {
        let opcode = cursor.list()[id].opcode().unwrap();
        if opcode % 2 == 0 {
            cursor.remove();
        } else {
            cursor.set(new_insn(opcode + 100));
        }
    }
    assert_eq!(cursor.next_index(), 5);
    assert_eq!(opcodes(&insn_list), [101, 103, 105, 107, 109]);
}

/// oxasm-specific: a list holds the type annotations of its instructions (`InsnNode` has no
/// field for them).
fn annotate(insn_list: &mut InsnList, id: InsnId, interner: &Interner, descriptor: &str) {
    let type_ref = oxasm::TypeReference::new_type_reference(oxasm::TypeReference::NEW);
    let descriptor = interner.intern(oxasm::JavaStr::from_str(descriptor));
    insn_list
        .type_annotations_mut(id)
        .visible
        .push(oxasm_tree::TypeAnnotationNode::new(
            type_ref, None, descriptor,
        ));
}

fn annotation_count(insn_list: &InsnList, id: InsnId) -> usize {
    insn_list
        .type_annotations(id)
        .map_or(0, |annotations| annotations.visible.len())
}

#[test]
fn type_annotations_follow_their_instruction() {
    let interner = Interner::new();
    let (mut insn_list, insn1, insn2) = new_insn_list_2();
    annotate(&mut insn_list, insn2, &interner, "LA;");
    assert_eq!(annotation_count(&insn_list, insn1), 0);
    assert_eq!(annotation_count(&insn_list, insn2), 1);

    // Moving a list into another keeps them.
    let mut other = new_insn_list();
    other.add(new_insn(7));
    let (first, last) = other.add_list(&mut insn_list).unwrap();
    assert_eq!(annotation_count(&other, first), 0);
    assert_eq!(annotation_count(&other, last), 1);
    assert!(insn_list.type_annotations(insn2).is_none());

    // Removing or replacing an instruction drops them, and its recycled slot starts without.
    other.remove(last);
    let reused = other.add(new_insn(8));
    assert_eq!(annotation_count(&other, reused), 0);
    annotate(&mut other, reused, &interner, "LB;");
    let (replaced, _) = other.set(reused, new_insn(9));
    assert_eq!(annotation_count(&other, replaced), 0);
}

/// Counts instructions and instruction annotations.
#[derive(Default)]
struct CountingVisitor {
    insns: usize,
    annotations: usize,
}

impl MethodVisitor for CountingVisitor {
    fn visit_insn(&mut self, _opcode: u8) {
        self.insns += 1;
    }

    fn visit_insn_annotation<'a>(
        &'a mut self,
        _type_ref: oxasm::TypeReference,
        _type_path: Option<&oxasm::TypePath>,
        _descriptor: &oxasm::JavaStr,
        _visible: bool,
    ) -> Option<Box<dyn oxasm::AnnotationVisitor + 'a>> {
        self.annotations += 1;
        None
    }
}

#[test]
fn accepting_an_instruction_visits_its_type_annotations() {
    let interner = Interner::new();
    let (mut insn_list, insn1, insn2) = new_insn_list_2();
    annotate(&mut insn_list, insn2, &interner, "LA;");

    let mut visitor = CountingVisitor::default();
    insn_list.accept_insn(insn1, &interner, &mut visitor);
    assert_eq!((visitor.insns, visitor.annotations), (1, 0));
    insn_list.accept_insn(insn2, &interner, &mut visitor);
    assert_eq!((visitor.insns, visitor.annotations), (2, 1));

    let mut visitor = CountingVisitor::default();
    insn_list.accept(&interner, &mut visitor);
    assert_eq!((visitor.insns, visitor.annotations), (2, 1));
}

#[test]
fn equal_lists_have_equal_type_annotations() {
    let interner = Interner::new();
    let (mut insn_list, _, insn2) = new_insn_list_2();
    let (other, _, _) = new_insn_list_2();
    assert_eq!(insn_list, other);
    annotate(&mut insn_list, insn2, &interner, "LA;");
    assert_ne!(insn_list, other);
    assert_eq!(insn_list.clone(), insn_list);
}

#[test]
fn insn_list_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InsnList>();
    assert_send_sync::<InsnId>();
}

// -------------------------------------------------------------------------------------------------
// Random edits against a Vec model
// -------------------------------------------------------------------------------------------------

/// An edit of a list, whose positions are taken modulo the list length.
#[derive(Clone, Debug)]
enum Op {
    Add,
    Insert,
    InsertAfter(usize),
    InsertBefore(usize),
    Set(usize),
    Remove(usize),
    AddList(u8),
    InsertList(u8),
    InsertListAfter(usize, u8),
    InsertListBefore(usize, u8),
    CursorAdd(usize),
    CursorRemoveNext(usize),
    CursorRemovePrevious(usize),
    CursorSet(usize),
    IndexOf(usize),
    Clear,
}

fn op() -> impl Strategy<Value = Op> {
    let i = || 0..64usize;
    let n = || 0..4u8;
    prop_oneof![
        3 => Just(Op::Add),
        1 => Just(Op::Insert),
        2 => i().prop_map(Op::InsertAfter),
        2 => i().prop_map(Op::InsertBefore),
        2 => i().prop_map(Op::Set),
        3 => i().prop_map(Op::Remove),
        1 => n().prop_map(Op::AddList),
        1 => n().prop_map(Op::InsertList),
        1 => (i(), n()).prop_map(|(i, n)| Op::InsertListAfter(i, n)),
        1 => (i(), n()).prop_map(|(i, n)| Op::InsertListBefore(i, n)),
        1 => i().prop_map(Op::CursorAdd),
        1 => i().prop_map(Op::CursorRemoveNext),
        1 => i().prop_map(Op::CursorRemovePrevious),
        1 => i().prop_map(Op::CursorSet),
        2 => i().prop_map(Op::IndexOf),
        1 => Just(Op::Clear),
    ]
}

/// The model: each instruction is an `Int` instruction whose operand is a unique tag.
struct Model {
    list: InsnList,
    /// The tags and handles of the instructions, in order.
    model: Vec<(i32, InsnId)>,
    next_tag: i32,
}

impl Model {
    fn node(&mut self) -> (i32, InsnNode) {
        self.next_tag += 1;
        let tag = self.next_tag;
        (
            tag,
            Insn::Int {
                opcode: oxasm::opcodes::BIPUSH,
                operand: tag,
            }
            .into(),
        )
    }

    fn other_list(&mut self, n: u8) -> (InsnList, Vec<i32>) {
        let mut other = InsnList::new();
        let mut tags = Vec::new();
        for _ in 0..n {
            let (tag, node) = self.node();
            other.add(node);
            tags.push(tag);
        }
        (other, tags)
    }

    /// The handles of a moved run: the first and last, and the ones in between.
    fn moved(&self, moved: Option<(InsnId, InsnId)>, tags: Vec<i32>) -> Vec<(i32, InsnId)> {
        let Some((first, last)) = moved else {
            assert!(tags.is_empty());
            return Vec::new();
        };
        let mut ids = vec![first];
        while *ids.last().unwrap() != last {
            ids.push(self.list.next(*ids.last().unwrap()).unwrap());
        }
        tags.into_iter().zip(ids).collect()
    }

    fn apply(&mut self, op: Op) {
        let len = self.model.len();
        let at = |i: usize| i % len.max(1);
        match op {
            Op::Add => {
                let (tag, node) = self.node();
                let id = self.list.add(node);
                self.model.push((tag, id));
            }
            Op::Insert => {
                let (tag, node) = self.node();
                let id = self.list.insert(node);
                self.model.insert(0, (tag, id));
            }
            Op::InsertAfter(i) if len > 0 => {
                let (tag, node) = self.node();
                let id = self.list.insert_after(self.model[at(i)].1, node);
                self.model.insert(at(i) + 1, (tag, id));
            }
            Op::InsertBefore(i) if len > 0 => {
                let (tag, node) = self.node();
                let id = self.list.insert_before(self.model[at(i)].1, node);
                self.model.insert(at(i), (tag, id));
            }
            Op::Set(i) if len > 0 => {
                let (tag, node) = self.node();
                let (old_tag, old_id) = self.model[at(i)];
                let (id, old) = self.list.set(old_id, node);
                assert_eq!(old.insn, int(old_tag));
                assert!(!self.list.contains(old_id));
                self.model[at(i)] = (tag, id);
            }
            Op::Remove(i) if len > 0 => {
                let (tag, id) = self.model.remove(at(i));
                assert_eq!(self.list.remove(id).insn, int(tag));
                assert!(!self.list.contains(id));
            }
            Op::AddList(n) => {
                let (mut other, tags) = self.other_list(n);
                let moved = self.list.add_list(&mut other);
                assert!(other.is_empty());
                let moved = self.moved(moved, tags);
                self.model.extend(moved);
            }
            Op::InsertList(n) => {
                let (mut other, tags) = self.other_list(n);
                let moved = self.list.insert_list(&mut other);
                let moved = self.moved(moved, tags);
                self.model.splice(0..0, moved);
            }
            Op::InsertListAfter(i, n) if len > 0 => {
                let (mut other, tags) = self.other_list(n);
                let moved = self.list.insert_list_after(self.model[at(i)].1, &mut other);
                let moved = self.moved(moved, tags);
                self.model.splice(at(i) + 1..at(i) + 1, moved);
            }
            Op::InsertListBefore(i, n) if len > 0 => {
                let (mut other, tags) = self.other_list(n);
                let moved = self
                    .list
                    .insert_list_before(self.model[at(i)].1, &mut other);
                let moved = self.moved(moved, tags);
                self.model.splice(at(i)..at(i), moved);
            }
            Op::CursorAdd(i) => {
                let index = i % (len + 1);
                let (tag, node) = self.node();
                let mut cursor = self.list.cursor(index);
                let id = cursor.add(node);
                assert_eq!(cursor.previous_index(), Some(index));
                assert_eq!(cursor.next_index(), index + 1);
                self.model.insert(index, (tag, id));
            }
            Op::CursorRemoveNext(i) if len > 0 => {
                let index = at(i);
                let mut cursor = self.list.cursor(index);
                assert_eq!(cursor.next(), Some(self.model[index].1));
                let (tag, _) = self.model.remove(index);
                assert_eq!(cursor.remove().insn, int(tag));
                assert_eq!(cursor.next_index(), index);
            }
            Op::CursorRemovePrevious(i) if len > 0 => {
                let index = at(i) + 1;
                let mut cursor = self.list.cursor(index);
                assert_eq!(cursor.previous(), Some(self.model[index - 1].1));
                let (tag, _) = self.model.remove(index - 1);
                assert_eq!(cursor.remove().insn, int(tag));
                assert_eq!(cursor.next_index(), index - 1);
            }
            Op::CursorSet(i) if len > 0 => {
                let index = at(i);
                let (tag, node) = self.node();
                let mut cursor = self.list.cursor(index);
                cursor.next();
                let (id, _) = cursor.set(node);
                assert_eq!(cursor.previous_index(), Some(index));
                self.model[index] = (tag, id);
            }
            Op::IndexOf(i) if len > 0 => {
                let (_, id) = self.model[at(i)];
                assert_eq!(self.list.index_of(id), Some(at(i)));
                assert_eq!(self.list.id_at(at(i)), Some(id));
            }
            Op::Clear => {
                self.list.clear();
                for (_, id) in self.model.drain(..) {
                    assert!(!self.list.contains(id));
                }
            }
            _ => {} // an edit at a position of an empty list
        }
    }

    fn check(&self) {
        assert_eq!(self.list.len(), self.model.len());
        let ids: Vec<InsnId> = self.model.iter().map(|&(_, id)| id).collect();
        assert_eq!(self.list.ids().collect::<Vec<_>>(), ids);
        assert_eq!(self.list.ids().rev().collect::<Vec<_>>(), {
            let mut ids = ids.clone();
            ids.reverse();
            ids
        });
        assert_eq!(self.list.first(), ids.first().copied());
        assert_eq!(self.list.last(), ids.last().copied());
        for (index, &(tag, id)) in self.model.iter().enumerate() {
            assert_eq!(self.list[id].insn, int(tag));
            assert_eq!(self.list.index_of(id), Some(index));
            assert_eq!(self.list.previous(id), index.checked_sub(1).map(|i| ids[i]));
            assert_eq!(self.list.next(id), ids.get(index + 1).copied());
        }
    }
}

fn int(tag: i32) -> Insn {
    Insn::Int {
        opcode: oxasm::opcodes::BIPUSH,
        operand: tag,
    }
}

/// One instruction of each kind that owns memory, then a plain one: a box (indy, both switches,
/// a frame) or a dynamic constant.
fn insns_owning_memory(interner: &Interner) -> Vec<Insn> {
    let sym = |s: &str| interner.intern(s);
    let bsm = SymHandle {
        tag: HandleKind::InvokeStatic,
        owner: sym("C"),
        name: sym("bsm"),
        desc: sym("()V"),
        is_interface: false,
    };
    let dynamic = SymConstantDynamic {
        name: sym("d"),
        desc: sym("I"),
        bsm,
        bsm_args: vec![SymConstant::Integer(1)],
    };
    vec![
        Insn::invoke_dynamic_insn(
            sym("m"),
            sym("()V"),
            bsm,
            vec![SymConstant::String(sym("s"))],
        ),
        Insn::table_switch_insn(0, 1, Label::new(), vec![Label::new(), Label::new()]),
        Insn::lookup_switch_insn(Label::new(), vec![3], vec![Label::new()]),
        Insn::frame(FrameNode::new(
            FrameKind::Full,
            &[SymFrameValue::Integer],
            &[],
        )),
        Insn::Ldc(SymConstant::Dynamic(Box::new(dynamic))),
        Insn::Simple { opcode: 0 },
    ]
}

/// oxasm-specific: a list drops the memory of its instructions however they leave it: when it is
/// dropped or cleared, or by removal, replacement or a move to another list (T3 in
/// oxasm-optimization.md: the list's slots have no drop glue, and its `Drop` frees the
/// instructions that own memory; Miri reports a leak).
#[test]
fn instructions_owning_memory_are_dropped() {
    let interner = Interner::new();
    let filled = || {
        let mut insn_list = InsnList::new();
        let ids: Vec<InsnId> = insns_owning_memory(&interner)
            .into_iter()
            .map(|insn| insn_list.add(insn))
            .collect();
        (insn_list, ids)
    };

    let (dropped, _) = filled();
    drop(dropped);

    let (mut cleared, _) = filled();
    cleared.clear();
    assert!(cleared.is_empty());

    let (mut edited, ids) = filled();
    drop(edited.remove(ids[0]));
    drop(edited.set(ids[1], Insn::Simple { opcode: 0 }));
    edited.node_mut(ids[2]).unwrap().insn = Insn::Simple { opcode: 0 };
    let (mut moved, _) = filled();
    edited.add_list(&mut moved);
    assert!(moved.is_empty());
    assert_eq!(edited.len(), 11);
    let (mut reused, ids) = filled();
    reused.remove(ids[3]);
    reused.add(insns_owning_memory(&interner).swap_remove(3));
    drop((edited, reused));

    // T9: the list drops only the slots it has chained as owners, so every way of storing an
    // instruction that owns memory in a slot must chain it: through `IndexMut` and `node_mut`
    // in a slot that held a plain instruction, by an insertion or replacement in a recycled
    // slot (once or twice), and in a copy.
    let mut plain = InsnList::new();
    let ids: Vec<InsnId> = (0..8).map(|tag| plain.add(int(tag))).collect();
    let mut owning = insns_owning_memory(&interner).into_iter();
    plain[ids[0]].insn = owning.next().unwrap();
    plain.node_mut(ids[1]).unwrap().insn = owning.next().unwrap();
    plain.remove(ids[2]);
    let recycled = plain.insert(owning.next().unwrap());
    plain.remove(recycled);
    let recycled = plain.insert_after(ids[3], int(8));
    plain.remove(recycled);
    plain.insert_before(ids[3], owning.next().unwrap());
    plain.set(ids[4], owning.next().unwrap());
    plain
        .cursor(0)
        .add(insns_owning_memory(&interner).swap_remove(3));
    let copy = plain.clone();
    drop((plain, copy));
}

/// oxasm-specific: a frame owns memory only if it has values (T7 in oxasm-optimization.md: an
/// empty frame allocates nothing, and the list leaves it out of its chain of owners); every
/// other one is freed, however it leaves the list (Miri reports a leak).
#[test]
fn frames_with_values_are_dropped() {
    let interner = Interner::new();
    let object = SymFrameValue::Object(interner.intern("A"));
    let frames = || {
        [
            FrameNode::new(FrameKind::Same, &[], &[]),
            FrameNode::new(FrameKind::Same1, &[], &[object]),
            FrameNode::new(FrameKind::Full, &[object, SymFrameValue::Long], &[object]),
            FrameNode::from_parts(FrameKind::Chop(1), &[object], &[]),
            FrameNode::new(FrameKind::Chop(1), &[], &[]),
        ]
        .map(Insn::frame)
    };
    let mut insn_list = InsnList::new();
    let ids = frames().map(|frame| insn_list.add(frame));
    drop(insn_list.remove(ids[1]));
    drop(insn_list.set(ids[0], frames()[2].clone()));
    insn_list[ids[3]].insn = frames()[1].clone();
    let no_labels = std::collections::HashMap::<Label, Label>::new();
    let mut other = InsnList::new();
    for node in insn_list.iter() {
        other.add(node.clone_with(&no_labels));
    }
    let mut moved = InsnList::new();
    for frame in frames() {
        moved.add(frame);
    }
    insn_list.add_list(&mut moved);
    assert_eq!(insn_list.len(), 9);
    drop((insn_list.clone(), insn_list, other));
}

proptest! {
    #[test]
    fn random_edits_match_a_vec_model(ops in proptest::collection::vec(op(), 0..200)) {
        let mut model = Model { list: InsnList::new(), model: Vec::new(), next_tag: 0 };
        for op in ops {
            model.apply(op);
            model.check();
        }
    }
}
