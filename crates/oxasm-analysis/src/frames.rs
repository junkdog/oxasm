//! [`Frames`]: the frames of an analysis, in one arena (optimization leads A1 and A5).

use std::fmt;
use std::iter::FusedIterator;

use crate::error::AnalyzerError;
use crate::frame::{Frame, FrameRef, FrameShape, incompatible_stack_heights, merge_values};
use crate::interpreter::Interpreter;
use crate::value::Value;

/// The frame at each instruction of an analyzed method (ASM's `getFrames()`): `None` for an
/// instruction that cannot be reached, or that the analysis did not reach before it failed.
/// [`get`](Self::get) and [`iter`](Self::iter) give the frames as [`FrameRef`]s, and
/// [`to_vec`](Self::to_vec) as [`Frame`]s.
///
/// The values of all the frames are in one `Vec`, during the analysis and after it. A frame's
/// values are a span of it, appended when the frame is first stored. The span never changes
/// length: a merge needs equal stack heights, and the number of local variables is that of the
/// stored frame. So a merge works in place, and storing a frame allocates nothing once the
/// arena has grown, where a `Frame` per instruction would allocate its own values.
pub struct Frames<V> {
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
    /// The span of its values in [`Frames::values`].
    start: usize,
    len: usize,
    /// Its index in [`Frames::returns`].
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

impl<V> Default for Frames<V> {
    fn default() -> Self {
        Frames {
            values: Vec::new(),
            returns: Vec::new(),
            slots: Vec::new(),
        }
    }
}

impl<V> Frames<V> {
    /// The number of instructions (and labels, ...) of the method: none for an abstract or
    /// native method.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the method has no instructions (an abstract or native method).
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The frame at the instruction at `index`, `None` if it has none or `index` is out of
    /// bounds.
    pub fn get(&self, index: usize) -> Option<FrameRef<'_, V>> {
        let slot = (*self.slots.get(index)?)?;
        Some(self.frame_ref(slot))
    }

    /// The frame at each instruction, in order.
    pub fn iter(&self) -> FramesIter<'_, V> {
        FramesIter {
            frames: self,
            slots: self.slots.iter(),
        }
    }

    /// The frames as [`Frame`]s, one per instruction (what ASM's `getFrames()` returns).
    pub fn to_vec(&self) -> Vec<Option<Frame<V>>>
    where
        V: Clone,
    {
        self.iter()
            .map(|frame| frame.map(|frame| frame.to_frame()))
            .collect()
    }

    fn frame_ref(&self, slot: Slot) -> FrameRef<'_, V> {
        FrameRef::new(
            &self.returns[slot.return_index],
            &self.values[slot.range()],
            slot.shape,
        )
    }
}

impl<V: Value> Frames<V> {
    /// Empties the arena, for a method of `len` instructions.
    pub(crate) fn reset(&mut self, len: usize) {
        self.values.clear();
        self.returns.clear();
        self.slots.clear();
        self.slots.resize(len, None);
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

    /// Updates what [`init`](Self::init) accumulates in `frame` from the frame at `index`: its
    /// capacity, the largest of those of the frames it was initialized from.
    ///
    /// # Panics
    ///
    /// If the instruction at `index` has no frame.
    pub(crate) fn init_capacity(&self, index: usize, frame: &mut Frame<V>) {
        frame.accumulate_capacity(self.slot(index).shape);
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

impl<V: Clone> Clone for Frames<V> {
    fn clone(&self) -> Self {
        Frames {
            values: self.values.clone(),
            returns: self.returns.clone(),
            slots: self.slots.clone(),
        }
    }
}

impl<V: fmt::Debug> fmt::Debug for Frames<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a, V> IntoIterator for &'a Frames<V> {
    type Item = Option<FrameRef<'a, V>>;
    type IntoIter = FramesIter<'a, V>;

    fn into_iter(self) -> FramesIter<'a, V> {
        self.iter()
    }
}

/// An iterator over the frame at each instruction of a method (see [`Frames::iter`]).
#[derive(Debug)]
pub struct FramesIter<'a, V> {
    frames: &'a Frames<V>,
    slots: std::slice::Iter<'a, Option<Slot>>,
}

impl<V> Clone for FramesIter<'_, V> {
    fn clone(&self) -> Self {
        FramesIter {
            frames: self.frames,
            slots: self.slots.clone(),
        }
    }
}

impl<'a, V> Iterator for FramesIter<'a, V> {
    type Item = Option<FrameRef<'a, V>>;

    fn next(&mut self) -> Option<Self::Item> {
        let slot = self.slots.next()?;
        Some(slot.map(|slot| self.frames.frame_ref(slot)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl<V> DoubleEndedIterator for FramesIter<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let slot = self.slots.next_back()?;
        Some(slot.map(|slot| self.frames.frame_ref(slot)))
    }
}

impl<V> ExactSizeIterator for FramesIter<'_, V> {}

impl<V> FusedIterator for FramesIter<'_, V> {}
