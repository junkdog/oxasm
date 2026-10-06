//! [`AnnotationVisitor`]: ASM's `AnnotationVisitor`.

use crate::constant::AnnotationValue;
use crate::jstr::JavaStr;

/// A visitor to visit a Java annotation. The methods of this trait must be called in the
/// following order: ( `visit` | `visit_enum` | `visit_annotation` | `visit_array` )* `visit_end`.
///
/// See the [module docs](crate::visitor) for the delegation and child visitor conventions.
pub trait AnnotationVisitor {
    /// The annotation visitor to which this visitor must delegate method calls; `None` (the
    /// default) ends the chain. Must return the same visitor every time.
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        None
    }

    /// Visits a primitive value of the annotation.
    ///
    /// - `name`: the value name (`None` for the elements of an array).
    /// - `value`: the actual value: a primitive, a `String`, a class [`Type`](crate::Type)
    ///   (of `OBJECT` or `ARRAY` sort), or an array of primitives. Such an array can also be
    ///   visited value by value with `visit_array`.
    fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
        if let Some(d) = self.delegate() {
            d.visit(name, value);
        }
    }

    /// Visits an enumeration value of the annotation.
    ///
    /// - `name`: the value name (`None` for the elements of an array).
    /// - `descriptor`: the class descriptor of the enumeration class.
    /// - `value`: the actual enumeration value.
    fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_enum(name, descriptor, value);
        }
    }

    /// Visits a nested annotation value of the annotation.
    ///
    /// - `name`: the value name (`None` for the elements of an array).
    /// - `descriptor`: the class descriptor of the nested annotation class.
    ///
    /// Returns a visitor to visit the actual nested annotation value, or `None` if this visitor is
    /// not interested in visiting this nested annotation. *The nested annotation value must be
    /// fully visited before calling other methods on this annotation visitor*.
    fn visit_annotation<'a>(
        &'a mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_annotation(name, descriptor)
    }

    /// Visits an array value of the annotation. Note that arrays of primitive values (such as
    /// byte, boolean, short, char, int, long, float or double) can be passed as value to `visit`.
    /// This is what a class reader does for non empty arrays of primitive values.
    ///
    /// - `name`: the value name (`None` for the elements of an array).
    ///
    /// Returns a visitor to visit the actual array value elements, or `None` if this visitor is
    /// not interested in visiting these values. The `name` parameters passed to the methods of
    /// this visitor are ignored. *All the array values must be visited before calling other
    /// methods on this annotation visitor*.
    fn visit_array<'a>(
        &'a mut self,
        name: Option<&JavaStr>,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_array(name)
    }

    /// Visits the end of the annotation.
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

macro_rules! forward_annotation_visitor {
    ($($ty:ty),*) => {$(
        impl<T: AnnotationVisitor + ?Sized> AnnotationVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
                (**self).delegate()
            }
            fn visit(&mut self, name: Option<&JavaStr>, value: &AnnotationValue<'_>) {
                (**self).visit(name, value);
            }
            fn visit_enum(&mut self, name: Option<&JavaStr>, descriptor: &JavaStr, value: &JavaStr) {
                (**self).visit_enum(name, descriptor, value);
            }
            fn visit_annotation<'a>(
                &'a mut self,
                name: Option<&JavaStr>,
                descriptor: &JavaStr,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_annotation(name, descriptor)
            }
            fn visit_array<'a>(
                &'a mut self,
                name: Option<&JavaStr>,
            ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
                (**self).visit_array(name)
            }
            fn visit_end(&mut self) {
                (**self).visit_end();
            }
        }
    )*};
}

forward_annotation_visitor!(&mut T, Box<T>);
