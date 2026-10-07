//! [`FieldRemapper`]: ASM's `FieldRemapper`.

use oxasm::{AnnotationVisitor, FieldVisitor, JavaStr, TypePath, TypeReference};

use crate::annotation_remapper::AnnotationRemapper;
use crate::remapper::Remapper;

/// A [`FieldVisitor`] that remaps types with a [`Remapper`].
pub struct FieldRemapper<'r, F, R: ?Sized> {
    fv: F,
    remapper: &'r R,
}

impl<'r, F: FieldVisitor, R: Remapper + ?Sized> FieldRemapper<'r, F, R> {
    /// Constructs a new field remapper.
    ///
    /// - `field_visitor`: the field visitor this remapper must delegate to.
    /// - `remapper`: the remapper to use to remap the types in the visited field.
    pub fn new(field_visitor: F, remapper: &'r R) -> Self {
        FieldRemapper {
            fv: field_visitor,
            remapper,
        }
    }
}

impl<F, R: ?Sized> std::fmt::Debug for FieldRemapper<'_, F, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldRemapper").finish_non_exhaustive()
    }
}

impl<F: FieldVisitor, R: Remapper + ?Sized> FieldVisitor for FieldRemapper<'_, F, R> {
    fn delegate(&mut self) -> Option<&mut dyn FieldVisitor> {
        Some(&mut self.fv)
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self
            .fv
            .visit_annotation(&remapper.map_desc(descriptor), visible)?;
        Some(Box::new(AnnotationRemapper::new(
            Some(descriptor),
            annotation_visitor,
            remapper,
        )))
    }

    fn visit_type_annotation<'b>(
        &'b mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self.fv.visit_type_annotation(
            type_ref,
            type_path,
            &remapper.map_desc(descriptor),
            visible,
        )?;
        Some(Box::new(AnnotationRemapper::new(
            Some(descriptor),
            annotation_visitor,
            remapper,
        )))
    }
}
