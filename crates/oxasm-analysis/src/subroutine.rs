//! `Subroutine`: ASM's `Subroutine`, a method subroutine (the target of `JSR` instructions).

use oxasm::Label;
use oxasm_tree::InsnId;

/// A method subroutine, which corresponds to the `JSR` instructions jumping to `start`.
#[derive(Clone, Debug)]
pub(crate) struct Subroutine {
    /// The start of this subroutine. Always set: ASM's `null` start is its "main subroutine",
    /// which the analyzer does not make (it marks the instructions this one reaches instead).
    pub(crate) start: Option<Label>,
    /// The local variables that are read or written by this subroutine: `locals_used[i]` is
    /// whether local variable `i` is.
    pub(crate) locals_used: Vec<bool>,
    /// The `JSR` instructions that jump to this subroutine.
    pub(crate) callers: Vec<InsnId>,
}

impl Subroutine {
    /// A new subroutine starting at `start`, with `caller` as its only caller.
    pub(crate) fn new(start: Option<Label>, max_locals: usize, caller: Option<InsnId>) -> Self {
        Subroutine {
            start,
            locals_used: vec![false; max_locals],
            callers: caller.into_iter().collect(),
        }
    }

    /// Merges the given subroutine into this one: the local variables it reads or writes are
    /// marked as read or written by this one, and its callers are added to this one's if both
    /// have the same start. Returns whether this subroutine changed.
    pub(crate) fn merge(&mut self, subroutine: &Subroutine) -> bool {
        let mut changed = false;
        for (used, &other) in self.locals_used.iter_mut().zip(&subroutine.locals_used) {
            if other && !*used {
                *used = true;
                changed = true;
            }
        }
        if subroutine.start == self.start {
            for caller in &subroutine.callers {
                if !self.callers.contains(caller) {
                    self.callers.push(*caller);
                    changed = true;
                }
            }
        }
        changed
    }
}
