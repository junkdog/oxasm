//! [`FieldVisitor`]: ASM's `FieldVisitor`.

use crate::attribute::Attribute;
use crate::jstr::JavaStr;
use crate::type_path::TypePath;
use crate::type_reference::TypeReference;
use crate::visitor::AnnotationVisitor;

/// A visitor to visit a Java field. The methods of this trait must be called in the following
/// order: ( `visit_annotation` | `visit_type_annotation` | `visit_attribute` )* `visit_end`.
///
/// See the [module docs](crate::visitor) for the delegation and child visitor conventions.
pub trait FieldVisitor {
    /// The field visitor to which this visitor must delegate method calls; `None` (the default)
    /// ends the chain. Must return the same visitor every time.
    fn delegate(&mut self) -> Option<&mut dyn FieldVisitor> {
        None
    }

    /// Visits an annotation of the field.
    ///
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_annotation(descriptor, visible)
    }

    /// Visits an annotation on the type of the field.
    ///
    /// - `type_ref`: a reference to the annotated type. The sort of this type reference must be
    ///   [`TypeReference::FIELD`].
    /// - `type_path`: the path to the annotated type argument, wildcard bound, array element
    ///   type, or static inner type within `type_ref`. `None` if the annotation targets
    ///   `type_ref` as a whole.
    /// - `descriptor`: the class descriptor of the annotation class.
    /// - `visible`: `true` if the annotation is visible at runtime.
    ///
    /// Returns a visitor to visit the annotation values, or `None` if this visitor is not
    /// interested in visiting this annotation.
    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?
            .visit_type_annotation(type_ref, type_path, descriptor, visible)
    }

    /// Visits a non standard attribute of the field.
    fn visit_attribute(&mut self, attribute: &Attribute) {
        if let Some(d) = self.delegate() {
            d.visit_attribute(attribute);
        }
    }

    /// Visits the end of the field. This method, which is the last one to be called, is used to
    /// inform the visitor that all the annotations and attributes of the field have been visited.
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

macro_rules! forward_field_visitor {
    ($($ty:ty),*) => {$(
        impl<T: FieldVisitor + ?Sized> FieldVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn FieldVisitor> {
                (**self).delegate()
            }
            fn visit_annotation<'a>(
                &'a mut self,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_annotation(descriptor, visible)
            }
            fn visit_type_annotation<'a>(
                &'a mut self,
                type_ref: TypeReference,
                type_path: Option<&TypePath>,
                descriptor: &JavaStr,
                visible: bool,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_type_annotation(type_ref, type_path, descriptor, visible)
            }
            fn visit_attribute(&mut self, attribute: &Attribute) {
                (**self).visit_attribute(attribute);
            }
            fn visit_end(&mut self) {
                (**self).visit_end();
            }
        }
    )*};
}

forward_field_visitor!(&mut T, Box<T>);
