//! [`RecordComponentRemapper`]: ASM's `RecordComponentRemapper`.

use oxasm::{AnnotationVisitor, JavaStr, RecordComponentVisitor, TypePath, TypeReference};

use crate::annotation_remapper::AnnotationRemapper;
use crate::remapper::Remapper;

/// A [`RecordComponentVisitor`] that remaps types with a [`Remapper`].
pub struct RecordComponentRemapper<'r, V, R: ?Sized> {
    record_component_visitor: V,
    remapper: &'r R,
}

impl<'r, V: RecordComponentVisitor, R: Remapper + ?Sized> RecordComponentRemapper<'r, V, R> {
    /// Constructs a new record component remapper.
    ///
    /// - `record_component_visitor`: the record component visitor this remapper must delegate
    ///   to.
    /// - `remapper`: the remapper to use to remap the types in the visited record component.
    pub fn new(record_component_visitor: V, remapper: &'r R) -> Self {
        RecordComponentRemapper {
            record_component_visitor,
            remapper,
        }
    }
}

impl<V, R: ?Sized> std::fmt::Debug for RecordComponentRemapper<'_, V, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordComponentRemapper")
            .finish_non_exhaustive()
    }
}

impl<V: RecordComponentVisitor, R: Remapper + ?Sized> RecordComponentVisitor
    for RecordComponentRemapper<'_, V, R>
{
    fn delegate(&mut self) -> Option<&mut dyn RecordComponentVisitor> {
        Some(&mut self.record_component_visitor)
    }

    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let annotation_visitor = self
            .record_component_visitor
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
        let annotation_visitor = self.record_component_visitor.visit_type_annotation(
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
