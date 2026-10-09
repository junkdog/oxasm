//! The cycles of a cursor that a negative `attribute_length` moves back.
//!
//! The reader computes offsets as ASM does, with signed integers checked only when it reads at
//! them, so a negative `attribute_length` moves an attribute loop's cursor back, and -6 makes it
//! stand still. A loop over 65,535 attributes then visits the same few attributes again and
//! again, and the nested loops (members × attributes, methods × Code attributes) run billions of
//! times on a class of a few hundred bytes: seconds in ASM and in a faithful port.
//!
//! Where an iteration ends depends only on where it starts, so the cursor's offsets are
//! eventually periodic, and a period has a step back. When one period changes nothing that
//! repeating it would change again (it only assigns values read at its offsets, or sets flags,
//! and visits nothing), the remaining whole periods are skipped: the loop ends in exactly the
//! state the remaining iterations would have left. A period that has an effect (a non standard
//! attribute to visit, a line number to add) repeats it, as ASM does.

/// Brent's cycle detection over the offsets of one loop's cursor after each step back. The
/// offsets after the steps back are eventually periodic too, and a well-formed class has no step
/// back: a loop only tests the length of each attribute, and this state is not touched.
pub(super) struct CursorCycles {
    state: Option<Brent>,
}

struct Brent {
    /// The offset the cursor is compared with, after a step back.
    mark: i64,
    /// The iterations and effects that remained, and had been counted, at `mark`.
    mark_remaining: u16,
    mark_effects: u32,
    /// The steps back since `mark`.
    steps: u32,
    /// The number of steps back at which `mark` moves to the cursor (a power of two).
    limit: u32,
    /// Whether a cycle was found: it was skipped, or it repeats its effects to the end.
    done: bool,
}

impl CursorCycles {
    #[inline(always)]
    pub(super) const fn new() -> Self {
        CursorCycles { state: None }
    }

    /// Records an iteration that moved the cursor back (or not at all) to `offset`, with
    /// `remaining` iterations left and `effects` iterations with an effect so far. Returns the
    /// iterations left: fewer, by whole periods, if the cursor has come back to an offset with
    /// no effect since.
    #[cold]
    #[inline(never)]
    pub(super) fn step_back(&mut self, offset: i64, remaining: u16, effects: u32) -> u16 {
        let Some(brent) = &mut self.state else {
            self.state = Some(Brent {
                mark: offset,
                mark_remaining: remaining,
                mark_effects: effects,
                steps: 0,
                limit: 1,
                done: false,
            });
            return remaining;
        };
        if brent.done {
            return remaining;
        }
        brent.steps += 1;
        if offset == brent.mark {
            brent.done = true;
            // The iterations since `mark` are whole periods of the cursor.
            if effects == brent.mark_effects {
                return remaining % (brent.mark_remaining - remaining);
            }
        } else if brent.steps == brent.limit {
            brent.mark = offset;
            brent.mark_remaining = remaining;
            brent.mark_effects = effects;
            brent.steps = 0;
            brent.limit *= 2;
        }
        remaining
    }
}

#[cfg(test)]
mod tests {
    use super::CursorCycles;

    /// Runs `count` iterations of the loop `next` describes from `start` (each returns the next
    /// offset and whether the iteration had an effect), with the cycle skips, and returns the
    /// offsets visited.
    fn run(start: i64, count: u16, next: impl Fn(i64) -> (i64, bool)) -> Vec<i64> {
        let mut cycles = CursorCycles::new();
        let mut offset = start;
        let mut visited = vec![offset];
        let mut remaining = count;
        let mut effects = 0;
        while remaining > 0 {
            remaining -= 1;
            let (next_offset, effect) = next(offset);
            effects += u32::from(effect);
            let forward = next_offset > offset;
            offset = next_offset;
            visited.push(offset);
            if !forward {
                remaining = cycles.step_back(offset, remaining, effects);
            }
        }
        visited
    }

    fn naive(start: i64, count: u16, next: impl Fn(i64) -> (i64, bool)) -> i64 {
        (0..count).fold(start, |offset, _| next(offset).0)
    }

    #[test]
    fn a_cursor_that_stands_still_stops_at_once() {
        let next = |offset: i64| (if offset < 30 { offset + 10 } else { offset }, false);
        let visited = run(0, u16::MAX, next);
        assert_eq!(visited.last(), Some(&naive(0, u16::MAX, next)));
        assert!(visited.len() < 10, "{visited:?}");
    }

    #[test]
    fn every_count_ends_where_the_whole_loop_does() {
        // A tail of 3 forward steps, then a cycle of 5 offsets with one step back.
        let next = |offset: i64| (if offset == 70 { 30 } else { offset + 10 }, false);
        // Two steps back in a period of 6: 20, 30, 40, 25, 35, 50, 20, ...
        let two_back = |offset: i64| {
            let next = match offset {
                40 => 25,
                25 => 35,
                35 => 50,
                50 => 20,
                _ => offset + 10,
            };
            (next, false)
        };
        for count in (0..200).chain([u16::MAX - 1, u16::MAX]) {
            for next in [&next as &dyn Fn(i64) -> (i64, bool), &two_back] {
                let visited = run(0, count, next);
                assert_eq!(
                    visited.last(),
                    Some(&naive(0, count, next)),
                    "count {count}"
                );
                assert!(visited.len() < 40, "count {count}: {} steps", visited.len());
            }
        }
    }

    #[test]
    fn a_cycle_with_an_effect_runs_to_the_end() {
        let next = |offset: i64| {
            if offset == 70 {
                (30, false)
            } else {
                (offset + 10, offset == 50)
            }
        };
        let visited = run(0, 1000, next);
        assert_eq!(visited.len(), 1001);
        assert_eq!(visited.last(), Some(&naive(0, 1000, next)));
    }
}
