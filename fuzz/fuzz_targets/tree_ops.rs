//! Random `InsnList` edits (single instructions, moved lists and cursor edits) agree with a
//! `Vec` model, and stale handles never designate an instruction (oxasm-verification,
//! "Fuzzing"; ADR-11).

#![no_main]

use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;
use oxasm::opcodes::BIPUSH;
use oxasm_tree::{Insn, InsnId, InsnList, InsnNode};

/// The list and its model: the tag (the `BIPUSH` operand) and handle of each instruction.
struct Model {
    list: InsnList,
    model: Vec<(i32, InsnId)>,
    /// Handles of removed instructions, which must stay stale.
    stale: Vec<InsnId>,
    next_tag: i32,
}

fn int(tag: i32) -> Insn {
    Insn::Int {
        opcode: BIPUSH,
        operand: tag,
    }
}

impl Model {
    fn node(&mut self) -> (i32, InsnNode) {
        self.next_tag += 1;
        (self.next_tag, int(self.next_tag).into())
    }

    fn other_list(&mut self, n: usize) -> (InsnList, Vec<i32>) {
        let mut other = InsnList::new();
        let mut tags = Vec::new();
        for _ in 0..n {
            let (tag, node) = self.node();
            other.add(node);
            tags.push(tag);
        }
        (other, tags)
    }

    fn moved(&self, moved: Option<(InsnId, InsnId)>, tags: Vec<i32>) -> Vec<(i32, InsnId)> {
        let Some((first, last)) = moved else {
            assert!(tags.is_empty());
            return Vec::new();
        };
        let mut ids = vec![first];
        while *ids.last().unwrap() != last {
            ids.push(self.list.next(*ids.last().unwrap()).unwrap());
        }
        assert_eq!(ids.len(), tags.len());
        tags.into_iter().zip(ids).collect()
    }

    fn apply(&mut self, u: &mut Unstructured<'_>) -> Result<()> {
        let len = self.model.len();
        let op: u8 = u.int_in_range(0..=15)?;
        if len == 0 && matches!(op, 2..=5 | 8..=9 | 11..=14) {
            return Ok(());
        }
        let i = if len == 0 {
            0
        } else {
            u.choose_index(len)?
        };
        let n = u.int_in_range(0..=3)?;
        match op {
            0 => {
                let (tag, node) = self.node();
                let id = self.list.add(node);
                self.model.push((tag, id));
            }
            1 => {
                let (tag, node) = self.node();
                let id = self.list.insert(node);
                self.model.insert(0, (tag, id));
            }
            2 => {
                let (tag, node) = self.node();
                let id = self.list.insert_after(self.model[i].1, node);
                self.model.insert(i + 1, (tag, id));
            }
            3 => {
                let (tag, node) = self.node();
                let id = self.list.insert_before(self.model[i].1, node);
                self.model.insert(i, (tag, id));
            }
            4 => {
                let (tag, node) = self.node();
                let (old_tag, old_id) = self.model[i];
                let (id, old) = self.list.set(old_id, node);
                assert_eq!(old.insn, int(old_tag));
                self.stale.push(old_id);
                self.model[i] = (tag, id);
            }
            5 => {
                let (tag, id) = self.model.remove(i);
                assert_eq!(self.list.remove(id).insn, int(tag));
                self.stale.push(id);
            }
            6 => {
                let (mut other, tags) = self.other_list(n);
                let moved = self.list.add_list(&mut other);
                assert!(other.is_empty());
                let moved = self.moved(moved, tags);
                self.model.extend(moved);
            }
            7 => {
                let (mut other, tags) = self.other_list(n);
                let moved = self.list.insert_list(&mut other);
                let moved = self.moved(moved, tags);
                self.model.splice(0..0, moved);
            }
            8 => {
                let (mut other, tags) = self.other_list(n);
                let moved = self.list.insert_list_after(self.model[i].1, &mut other);
                let moved = self.moved(moved, tags);
                self.model.splice(i + 1..i + 1, moved);
            }
            9 => {
                let (mut other, tags) = self.other_list(n);
                let moved = self.list.insert_list_before(self.model[i].1, &mut other);
                let moved = self.moved(moved, tags);
                self.model.splice(i..i, moved);
            }
            10 => {
                let index = u.int_in_range(0..=len)?;
                let (tag, node) = self.node();
                let mut cursor = self.list.cursor(index);
                let id = cursor.add(node);
                assert_eq!(cursor.previous_index(), Some(index));
                self.model.insert(index, (tag, id));
            }
            11 => {
                let mut cursor = self.list.cursor(i);
                assert_eq!(cursor.next(), Some(self.model[i].1));
                let (tag, id) = self.model.remove(i);
                assert_eq!(cursor.remove().insn, int(tag));
                assert_eq!(cursor.next_index(), i);
                self.stale.push(id);
            }
            12 => {
                let mut cursor = self.list.cursor(i + 1);
                assert_eq!(cursor.previous(), Some(self.model[i].1));
                let (tag, id) = self.model.remove(i);
                assert_eq!(cursor.remove().insn, int(tag));
                assert_eq!(cursor.next_index(), i);
                self.stale.push(id);
            }
            13 => {
                let (tag, node) = self.node();
                let mut cursor = self.list.cursor(i);
                cursor.next();
                let (id, _) = cursor.set(node);
                self.stale.push(self.model[i].1);
                self.model[i] = (tag, id);
            }
            14 => {
                let (_, id) = self.model[i];
                assert_eq!(self.list.index_of(id), Some(i));
                assert_eq!(self.list.id_at(i), Some(id));
            }
            _ => {
                self.list.clear();
                self.stale.extend(self.model.drain(..).map(|(_, id)| id));
            }
        }
        Ok(())
    }

    fn check(&self) {
        assert_eq!(self.list.len(), self.model.len());
        let ids: Vec<InsnId> = self.model.iter().map(|&(_, id)| id).collect();
        assert!(self.list.ids().eq(ids.iter().copied()));
        assert!(self.list.ids().rev().eq(ids.iter().rev().copied()));
        for (index, &(tag, id)) in self.model.iter().enumerate() {
            assert_eq!(self.list[id].insn, int(tag));
            assert_eq!(self.list.index_of(id), Some(index));
            assert_eq!(
                self.list.previous(id),
                index.checked_sub(1).map(|i| ids[i])
            );
            assert_eq!(self.list.next(id), ids.get(index + 1).copied());
        }
        for &id in &self.stale {
            assert!(!self.list.contains(id));
            assert!(self.list.node(id).is_none());
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let mut model = Model {
        list: InsnList::new(),
        model: Vec::new(),
        stale: Vec::new(),
        next_tag: 0,
    };
    while !u.is_empty() {
        if model.apply(&mut u).is_err() {
            break;
        }
        model.check();
    }
});
