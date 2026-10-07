//! [`AnnotationRemapper`]: ASM's `AnnotationRemapper`.

use std::borrow::Cow;

use oxasm::{AnnotationValue, AnnotationVisitor, JavaCompactStr, JavaStr};

use crate::remapper::{Remapper, defaults};

/// An [`AnnotationVisitor`] that remaps types with a [`Remapper`].
///
/// Unlike ASM's, it always wraps the nested annotation and array visitors of its delegate in new
/// annotation remappers: ASM returns itself when the delegate returns itself, which a boxed child
/// visitor cannot express (and which no ASM visitor does).
pub struct AnnotationRemapper<'r, A, R: ?Sized> {
    /// The descriptor of the visited annotation. `None`, for instance, for
    /// `AnnotationVisitor::visit_array`.
    descriptor: Option<JavaCompactStr>,
    av: A,
    remapper: &'r R,
}

impl<'r, A: AnnotationVisitor, R: Remapper + ?Sized> AnnotationRemapper<'r, A, R> {
    /// Constructs a new annotation remapper.
    ///
    /// - `descriptor`: the descriptor of the visited annotation, or `None` (for an array, or an
    ///   annotation default value).
    /// - `annotation_visitor`: the annotation visitor this remapper must delegate to.
    /// - `remapper`: the remapper to use to remap the types in the visited annotation.
    pub fn new(descriptor: Option<&JavaStr>, annotation_visitor: A, remapper: &'r R) -> Self {
        AnnotationRemapper {
            descriptor: descriptor.map(JavaCompactStr::from),
            av: annotation_visitor,
            remapper,
        }
    }

    /// The downstream annotation visitor.
    pub fn delegate_ref(&self) -> &A {
        &self.av
    }
}

impl<A, R: ?Sized> std::fmt::Debug for AnnotationRemapper<'_, A, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnnotationRemapper")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// ASM's private `mapAnnotationAttributeName`: the name, mapped if the annotation's descriptor is
/// known.
fn map_annotation_attribute_name<'a, R: Remapper + ?Sized>(
    remapper: &'a R,
    descriptor: Option<&'a JavaCompactStr>,
    name: Option<&'a JavaStr>,
) -> Option<Cow<'a, JavaStr>> {
    let name = name?;
    Some(match descriptor {
        None => Cow::Borrowed(name),
        Some(descriptor) => remapper.map_annotation_attribute_name(descriptor, name),
    })
}

impl<A: AnnotationVisitor, R: Remapper + ?Sized> AnnotationVisitor
    for AnnotationRemapper<'_, A, R>
{
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        Some(&mut self.av)
    }

    fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        let name = map_annotation_attribute_name(self.remapper, self.descriptor.as_ref(), name);
        self.av.visit(
            name.as_deref(),
            &defaults::map_annotation_value(self.remapper, value),
        );
    }

    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        let name = map_annotation_attribute_name(self.remapper, self.descriptor.as_ref(), name);
        self.av
            .visit_enum(name.as_deref(), &self.remapper.map_desc(descriptor), value);
    }

    fn visit_annotation<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let name = map_annotation_attribute_name(remapper, self.descriptor.as_ref(), name);
        let annotation_visitor = self
            .av
            .visit_annotation(name.as_deref(), &remapper.map_desc(descriptor))?;
        Some(Box::new(AnnotationRemapper::new(
            Some(descriptor),
            annotation_visitor,
            remapper,
        )))
    }

    fn visit_array<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let remapper = self.remapper;
        let name = map_annotation_attribute_name(remapper, self.descriptor.as_ref(), name);
        let annotation_visitor = self.av.visit_array(name.as_deref())?;
        Some(Box::new(AnnotationRemapper::new(
            /* descriptor= */ None,
            annotation_visitor,
            remapper,
        )))
    }
}
