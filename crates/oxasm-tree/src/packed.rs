//! [`Packed`] and [`PackedBox`]: a value and an owning pointer aligned to 4 bytes.
//!
//! They keep a [`SymConstant`](crate::SymConstant) at 12 bytes with an alignment of 4, so that an
//! [`Insn`](crate::Insn) holding one fits in 16 bytes, and an instruction list's slot in 32 (T13
//! in oxasm-optimization.md): an `i64`, an `f64` or a `Box` would align it to 8.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

/// A value stored with an alignment of at most 4 bytes: the `long` and `double` values of a
/// [`SymConstant`](crate::SymConstant).
///
/// Its field is read by value (`packed.0` or [`get`](Self::get)); a reference to it can't be
/// taken, since it may be unaligned.
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
pub struct Packed<T: Copy>(pub T);

impl<T: Copy> Packed<T> {
    /// The value.
    pub const fn get(self) -> T {
        self.0
    }
}

impl<T: Copy> From<T> for Packed<T> {
    fn from(value: T) -> Self {
        Packed(value)
    }
}

impl<T: Copy + fmt::Debug> fmt::Debug for Packed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl<T: Copy + PartialEq> PartialEq for Packed<T> {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl<T: Copy + Eq> Eq for Packed<T> {}

impl<T: Copy + Hash> Hash for Packed<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get().hash(state);
    }
}

/// An owning pointer to a heap value, like a `Box`, stored with an alignment of 4 bytes: the
/// handle and dynamic constants of a [`SymConstant`](crate::SymConstant).
#[repr(C, packed(4))]
pub struct PackedBox<T> {
    ptr: NonNull<T>,
    /// The value is owned (for the drop check).
    _owns: PhantomData<T>,
}

// SAFETY: a `PackedBox` owns its value like a `Box`, which is `Send` when the value is.
unsafe impl<T: Send> Send for PackedBox<T> {}

// SAFETY: as above: `&PackedBox<T>` only gives out `&T`.
unsafe impl<T: Sync> Sync for PackedBox<T> {}

impl<T> PackedBox<T> {
    /// Moves the given value to the heap.
    pub fn new(value: T) -> PackedBox<T> {
        PackedBox {
            ptr: NonNull::from(Box::leak(Box::new(value))),
            _owns: PhantomData,
        }
    }

    /// The value, moved out of the heap.
    pub fn into_inner(self) -> T {
        let ptr = self.ptr;
        std::mem::forget(self);
        // SAFETY: the pointer is from `Box::leak`, and `self`, which owned it, is forgotten.
        *unsafe { Box::from_raw(ptr.as_ptr()) }
    }
}

impl<T> From<T> for PackedBox<T> {
    fn from(value: T) -> Self {
        PackedBox::new(value)
    }
}

impl<T> Deref for PackedBox<T> {
    type Target = T;

    fn deref(&self) -> &T {
        let ptr = self.ptr;
        // SAFETY: the pointer is from `Box::leak`, and the value lives as long as `self`, which
        // owns it.
        unsafe { ptr.as_ref() }
    }
}

impl<T> DerefMut for PackedBox<T> {
    fn deref_mut(&mut self) -> &mut T {
        let mut ptr = self.ptr;
        // SAFETY: as in `deref`, and `&mut self` makes the access unique.
        unsafe { ptr.as_mut() }
    }
}

impl<T> Drop for PackedBox<T> {
    fn drop(&mut self) {
        let ptr = self.ptr;
        // SAFETY: the pointer is from `Box::leak`, and this is its owner's only drop.
        drop(unsafe { Box::from_raw(ptr.as_ptr()) });
    }
}

impl<T: Clone> Clone for PackedBox<T> {
    fn clone(&self) -> Self {
        PackedBox::new(T::clone(self))
    }
}

impl<T: fmt::Debug> fmt::Debug for PackedBox<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        T::fmt(self, f)
    }
}

impl<T: PartialEq> PartialEq for PackedBox<T> {
    fn eq(&self, other: &Self) -> bool {
        T::eq(self, other)
    }
}

impl<T: Eq> Eq for PackedBox<T> {}

impl<T: Hash> Hash for PackedBox<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        T::hash(self, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_values_are_aligned_to_4() {
        assert_eq!(align_of::<Packed<i64>>(), 4);
        assert_eq!(size_of::<Packed<f64>>(), 8);
        assert_eq!(align_of::<PackedBox<u64>>(), 4);
        assert_eq!(size_of::<PackedBox<u64>>(), 8);
    }

    #[test]
    fn packed_reads_and_compares_by_value() {
        let values = [Packed(1_i64), Packed(i64::MIN), Packed(i64::MAX)];
        assert_eq!(values.map(Packed::get), [1, i64::MIN, i64::MAX]);
        assert_eq!(Packed::from(2.5_f64).get(), 2.5);
        assert_eq!(Packed(3_i64), Packed(3_i64));
        assert_ne!(Packed(3_i64), Packed(4_i64));
        assert_eq!(format!("{:?}", Packed(7_i64)), "7");
    }

    #[test]
    fn packed_box_owns_its_value() {
        let mut boxed = PackedBox::new(vec![1, 2, 3]);
        boxed.push(4);
        assert_eq!(*boxed, [1, 2, 3, 4]);
        let clone = boxed.clone();
        boxed.clear();
        assert_eq!(*clone, [1, 2, 3, 4]);
        assert!(boxed.is_empty());
        assert_ne!(boxed, clone);
        assert_eq!(clone.into_inner(), [1, 2, 3, 4]);
        assert_eq!(format!("{boxed:?}"), "[]");
    }

    #[test]
    fn packed_boxes_unaligned_in_a_vector() {
        // A `(u32, PackedBox)` is 12 bytes, so every other box is at an address that isn't a
        // multiple of 8.
        let boxes: Vec<(u32, PackedBox<String>)> =
            (0..9).map(|i| (i, PackedBox::new(i.to_string()))).collect();
        assert_eq!(size_of::<(u32, PackedBox<String>)>(), 12);
        let copies = boxes.clone();
        for (i, boxed) in &copies {
            assert_eq!(**boxed, i.to_string());
        }
        drop(boxes);
        assert_eq!(copies.len(), 9);
    }

    #[test]
    fn packed_box_moves_to_another_thread() {
        let boxed = PackedBox::new(String::from("moved"));
        let back = std::thread::spawn(move || boxed.into_inner())
            .join()
            .unwrap();
        assert_eq!(back, "moved");
    }
}
