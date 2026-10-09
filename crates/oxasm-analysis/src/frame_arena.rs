//! [`FrameArena`]: the frames of an analysis in progress, in one arena (optimization lead A1).

use crate::error::AnalyzerError;
use crate::frame::{Frame, FrameShape, incompatible_stack_heights, merge_values};
use crate::interpreter::Interpreter;
use crate::value::Value;

/// The frame at each instruction of a method being analyzed (ASM's `frames` array), with the
/// values of all the frames in one `Vec`.
///
/// A frame's values are a span of [`values`](Self::values), appended when the frame is first
/// stored. The span never changes length: a merge needs equal stack heights, and the number of
/// local variables is that of the stored frame. So a merge works in place, and storing a frame
/// allocates nothing once the arena has grown, where a `Frame` per instruction would allocate
/// its own values.
#[derive(Debug)]
pub(crate) struct FrameArena<V> {
    /// The values of every stored frame, in the order they were first stored.
    values: Vec<V>,
    /// The distinct return values of the stored frames (usually one: every frame of an analysis
    /// is a copy of the first frame).
    returns: Vec<Option<V>>,
    /// The frame at each instruction, if any.
    slots: Vec<Option<Slot>>,
}

/// Where a stored frame is.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// The span of its values in [`FrameArena::values`].
    start: usize,
    len: usize,
    /// Its index in [`FrameArena::returns`].
    return_index: usize,
    shape: FrameShape,
}

impl Slot {
    fn range(&self) -> std::ops::Range<usize> {
        self.start..self.start + self.len
    }

    fn stack_size(&self) -> usize {
        self.len - self.shape.num_locals
    }
}

impl<V> Default for FrameArena<V> {
    fn default() -> Self {
        FrameArena {
            values: Vec::new(),
            returns: Vec::new(),
            slots: Vec::new(),
        }
    }
}

impl<V: Value> FrameArena<V> {
    /// Empties the arena, for a method of `len` instructions.
    pub(crate) fn reset(&mut self, len: usize) {
        self.values.clear();
        self.returns.clear();
        self.slots.clear();
        self.slots.resize(len, None);
    }

    /// The number of instructions.
    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the instruction at `index` has a frame.
    pub(crate) fn is_set(&self, index: usize) -> bool {
        self.slots[index].is_some()
    }

    fn slot(&self, index: usize) -> Slot {
        self.slots[index].expect("an instruction with a frame")
    }

    /// Stores a copy of `frame` at `index`, which must have no frame.
    fn store(&mut self, index: usize, frame: &Frame<V>) {
        let return_index = match self.returns.last() {
            Some(last) if last == frame.return_value() => self.returns.len() - 1,
            _ => {
                self.returns.push(frame.return_value().clone());
                self.returns.len() - 1
            }
        };
        let start = self.values.len();
        self.values.extend_from_slice(frame.values());
        self.slots[index] = Some(Slot {
            start,
            len: frame.values().len(),
            return_index,
            shape: frame.shape(),
        });
    }

    /// Merges `frame` into the frame at `index`, or stores a copy of it if there is none (what
    /// ASM's `merge` does with `frames[index]`). Returns whether the frame at `index` changed.
    ///
    /// # Panics
    ///
    /// If `index` is out of bounds.
    pub(crate) fn merge<I>(
        &mut self,
        index: usize,
        frame: &Frame<V>,
        interpreter: &mut I,
    ) -> Result<bool, AnalyzerError>
    where
        I: Interpreter<V = V> + ?Sized,
    {
        let Some(slot) = self.slots[index] else {
            self.store(index, frame);
            return Ok(true);
        };
        if slot.stack_size() != frame.get_stack_size() {
            return Err(incompatible_stack_heights());
        }
        merge_values(&mut self.values[slot.range()], frame.values(), interpreter)
    }

    /// Merges the frame at `index` into the frame at `index + 1`, or stores a copy of it there
    /// if there is none. Returns whether the frame at `index + 1` changed.
    ///
    /// # Panics
    ///
    /// If the instruction at `index` has no frame, or `index + 1` is out of bounds.
    pub(crate) fn merge_into_next<I>(
        &mut self,
        index: usize,
        interpreter: &mut I,
    ) -> Result<bool, AnalyzerError>
    where
        I: Interpreter<V = V> + ?Sized,
    {
        let source = self.slot(index);
        let Some(target) = self.slots[index + 1] else {
            let start = self.values.len();
            self.values.extend_from_within(source.range());
            self.slots[index + 1] = Some(Slot { start, ..source });
            return Ok(true);
        };
        if target.stack_size() != source.stack_size() {
            return Err(incompatible_stack_heights());
        }
        // The spans of two frames are disjoint.
        let (source_values, target_values) = if source.start < target.start {
            let (head, tail) = self.values.split_at_mut(target.start);
            (&head[source.range()], &mut tail[..target.len])
        } else {
            let (head, tail) = self.values.split_at_mut(source.start);
            (&tail[..source.len], &mut head[target.range()])
        };
        merge_values(target_values, source_values, interpreter)
    }

    /// Copies the frame at `index` into `frame`, as [`Frame::init`] does.
    ///
    /// # Panics
    ///
    /// If the instruction at `index` has no frame.
    pub(crate) fn init<'f>(&self, index: usize, frame: &'f mut Frame<V>) -> &'f mut Frame<V> {
        let slot = self.slot(index);
        frame.init_from(
            &self.returns[slot.return_index],
            &self.values[slot.range()],
            slot.shape,
        )
    }

    /// Makes `frame` a clone of the frame at `index`.
    ///
    /// # Panics
    ///
    /// If the instruction at `index` has no frame.
    pub(crate) fn clone_into(&self, index: usize, frame: &mut Frame<V>) {
        let slot = self.slot(index);
        frame.assign(
            &self.returns[slot.return_index],
            &self.values[slot.range()],
            slot.shape,
        );
    }

    /// The values of the frame at `index`.
    ///
    /// # Panics
    ///
    /// If the instruction at `index` has no frame.
    pub(crate) fn values(&self, index: usize) -> &[V] {
        &self.values[self.slot(index).range()]
    }

    /// The frame at `index`, as a [`Frame`].
    fn to_frame(&self, slot: Slot) -> Frame<V> {
        Frame::from_parts(
            self.returns[slot.return_index].clone(),
            self.values[slot.range()].to_vec(),
            slot.shape,
        )
    }

    /// Moves the frames into `frames`, which has one element per instruction: their values stay
    /// in one allocation, which they share.
    pub(crate) fn finish(&mut self, frames: &mut [Option<Frame<V>>]) {
        let arena = std::sync::Arc::new(std::mem::take(&mut self.values));
        for (frame, slot) in frames.iter_mut().zip(&self.slots) {
            *frame = slot.map(|slot| {
                Frame::from_shared(
                    self.returns[slot.return_index].clone(),
                    arena.clone(),
                    slot.range(),
                    slot.shape,
                )
            });
        }
    }

    /// Writes copies of the frames into `frames`, which has one element per instruction.
    pub(crate) fn export(&self, frames: &mut [Option<Frame<V>>]) {
        for (frame, slot) in frames.iter_mut().zip(&self.slots) {
            *frame = slot.map(|slot| self.to_frame(slot));
        }
    }

    /// Replaces the frames of this arena with those of `frames`, which are taken out.
    pub(crate) fn import(&mut self, frames: &mut [Option<Frame<V>>]) {
        self.reset(frames.len());
        for (index, frame) in frames.iter_mut().enumerate() {
            if let Some(frame) = frame.take() {
                self.store(index, &frame);
            }
        }
    }
}
