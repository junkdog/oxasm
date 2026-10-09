//! [`ClassVisitor`]: ASM's `ClassVisitor`.

use crate::attribute::Attribute;
use crate::constant::Constant;
use crate::jstr::JavaStr;
use crate::type_path::TypePath;
use crate::type_reference::TypeReference;
use crate::visitor::{
    AnnotationVisitor, FieldVisitor, MethodVisitor, ModuleVisitor, RecordComponentVisitor,
};
use crate::writer::ClassWriterFlags;

/// A visitor to visit a Java class. The methods of this trait must be called in the following
/// order: `visit` \[ `visit_source` \] \[ `visit_module` \]\[ `visit_nest_host` \]\[
/// `visit_outer_class` \] ( `visit_annotation` | `visit_type_annotation` | `visit_attribute` )* (
/// `visit_nest_member` | \[ `visit_permitted_subclass` \] | `visit_inner_class` |
/// `visit_record_component` | `visit_field` | `visit_method` )* `visit_end`.
///
/// See the [module docs](crate::visitor) for the delegation and child visitor conventions.
pub trait ClassVisitor {
    /// The class visitor to which this visitor must delegate method calls; `None` (the default)
    /// ends the chain. Must return the same visitor every time.
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        None
    }

    /// The flags of this visitor if it is a [`ClassWriter`](crate::ClassWriter), `None`
    /// otherwise: ASM's `instanceof ClassWriter` test, with which a `CheckClassAdapter` finds
    /// out whether its downstream computes the maximum stack sizes and frames (ADR-08).
    ///
    /// Like [`MethodVisitor::try_copy_method`], this method is deliberately **not** forwarded to
    /// [`ClassVisitor::delegate`]: ASM only looks at the visitor itself. (The `&mut V` and
    /// `Box<V>` impls do forward it, since they are the same visitor.)
    #[doc(hidden)]
    fn class_writer_flags(&self) -> Option<ClassWriterFlags> {
        None
    }

    /// Visits the header of the class.
    ///
    /// - `version`: the class version. The minor version is stored in the 16 most significant
    ///   bits, and the major version in the 16 least significant bits.
    /// - `access`: the class's access flags (see [`crate::opcodes`]). This parameter also
    ///   indicates if the class is deprecated (`ACC_DEPRECATED`) or a record (`ACC_RECORD`).
    /// - `name`: the internal name of the class (see
    ///   [`Type::get_internal_name`](crate::Type::get_internal_name)).
    /// - `signature`: the signature of this class. `None` if the class is not a generic one, and
    ///   does not extend or implement generic classes or interfaces.
    /// - `super_name`: the internal name of the super class. For interfaces, the super class is
    ///   `java/lang/Object`. `None` only for the `java/lang/Object` class.
    /// - `interfaces`: the internal names of the class's interfaces.
    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        if let Some(d) = self.delegate() {
            d.visit(version, access, name, signature, super_name, interfaces);
        }
    }

    /// Visits the number of fields and methods of the class: an oxasm extension, which ASM does
    /// not have. A class reader calls it right after [`visit`](Self::visit), so that a visitor
    /// which stores the members can size its storage once. It is only a hint: other sources of
    /// events need not call it, and the members visited may be fewer or more.
    fn visit_member_counts(&mut self, fields_count: u16, methods_count: u16) {
        if let Some(d) = self.delegate() {
            d.visit_member_counts(fields_count, methods_count);
        }
    }

    /// Visits the source of the class.
    ///
    /// - `source`: the name of the source file from which the class was compiled, or `None`.
    /// - `debug`: additional debug information to compute the correspondence between source and
    ///   compiled elements of the class, or `None`.
    fn visit_source(&mut self, source: Option<&JavaStr>, debug: Option<&JavaStr>) {
        if let Some(d) = self.delegate() {
            d.visit_source(source, debug);
        }
    }

    /// Visit the module corresponding to the class.
    ///
    /// - `name`: the fully qualified name (using dots) of the module.
    /// - `access`: the module access flags, among `ACC_OPEN`, `ACC_SYNTHETIC` and
    ///   `ACC_MANDATED`.
    /// - `version`: the module version, or `None`.
    ///
    /// Returns a visitor to visit the module values, or `None` if this visitor is not interested
    /// in visiting this module.
    fn visit_module<'a>(
        &'a mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        self.delegate()?.visit_module(name, access, version)
    }

    /// Visits the nest host class of the class. A nest is a set of classes of the same package
    /// that share access to their private members. One of these classes, called the host, lists
    /// the other members of the nest, which in turn should link to the host of their nest. This
    /// method must be called only once and only if the visited class is a non-host member of a
    /// nest. A class is implicitly its own nest, so it's invalid to call this method with the
    /// visited class name as argument.
    ///
    /// - `nest_host`: the internal name of the host class of the nest.
    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_nest_host(nest_host);
        }
    }

    /// Visits the enclosing class of the class. This method must be called only if this class is
    /// a local or anonymous class. See the JVMS 4.7.7 section for more details.
    ///
    /// - `owner`: internal name of the enclosing class of the class.
    /// - `name`: the name of the method that contains the class, or `None` if the class is not
    ///   enclosed in a method or constructor of its enclosing class (e.g. if it is enclosed in an
    ///   instance initializer, static initializer, instance variable initializer, or class
    ///   variable initializer).
    /// - `descriptor`: the descriptor of the method that contains the class, or `None` (in the
    ///   same cases as `name`).
    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        if let Some(d) = self.delegate() {
            d.visit_outer_class(owner, name, descriptor);
        }
    }

    /// Visits an annotation of the class.
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

    /// Visits an annotation on a type in the class signature.
    ///
    /// - `type_ref`: a reference to the annotated type. The sort of this type reference must be
    ///   [`TypeReference::CLASS_TYPE_PARAMETER`], [`TypeReference::CLASS_TYPE_PARAMETER_BOUND`]
    ///   or [`TypeReference::CLASS_EXTENDS`].
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

    /// Visits a non standard attribute of the class.
    fn visit_attribute(&mut self, attribute: &Attribute) {
        if let Some(d) = self.delegate() {
            d.visit_attribute(attribute);
        }
    }

    /// Visits a member of the nest. A nest is a set of classes of the same package that share
    /// access to their private members. One of these classes, called the host, lists the other
    /// members of the nest, which in turn should link to the host of their nest. This method must
    /// be called only if the visited class is the host of a nest. A nest host is implicitly a
    /// member of its own nest, so it's invalid to call this method with the visited class name as
    /// argument.
    ///
    /// - `nest_member`: the internal name of a nest member.
    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_nest_member(nest_member);
        }
    }

    /// Visits a permitted subclasses. A permitted subclass is one of the allowed subclasses of
    /// the current class.
    ///
    /// - `permitted_subclass`: the internal name of a permitted subclass.
    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_permitted_subclass(permitted_subclass);
        }
    }

    /// Visits information about an inner class. This inner class is not necessarily a member of
    /// the class being visited. More precisely, every class or interface C which is referenced by
    /// this class and which is not a package member must be visited with this method. This class
    /// must reference its nested class or interface members, and its enclosing class, if any.
    /// See the JVMS 4.7.6 section for more details.
    ///
    /// - `name`: the internal name of C.
    /// - `outer_name`: the internal name of the class or interface C is a member of. `None` if C
    ///   is not the member of a class or interface (e.g. for local or anonymous classes).
    /// - `inner_name`: the (simple) name of C. `None` for anonymous inner classes.
    /// - `access`: the access flags of C originally declared in the source code from which this
    ///   class was compiled.
    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    ) {
        if let Some(d) = self.delegate() {
            d.visit_inner_class(name, outer_name, inner_name, access);
        }
    }

    /// Visits a record component of the class.
    ///
    /// - `name`: the record component name.
    /// - `descriptor`: the record component descriptor (see [`Type`](crate::Type)).
    /// - `signature`: the record component signature. `None` if the record component type does
    ///   not use generic types.
    ///
    /// Returns a visitor to visit this record component annotations and attributes, or `None` if
    /// this class visitor is not interested in visiting these annotations and attributes.
    fn visit_record_component<'a>(
        &'a mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        self.delegate()?
            .visit_record_component(name, descriptor, signature)
    }

    /// Visits a field of the class.
    ///
    /// - `access`: the field's access flags (see [`crate::opcodes`]). This parameter also
    ///   indicates if the field is synthetic and/or deprecated.
    /// - `name`: the field's name.
    /// - `descriptor`: the field's descriptor (see [`Type`](crate::Type)).
    /// - `signature`: the field's signature. `None` if the field's type does not use generic
    ///   types.
    /// - `value`: the field's initial value, or `None` if the field does not have one. Must be an
    ///   `Integer`, a `Float`, a `Long`, a `Double` or a `String` constant (for `int`, `float`,
    ///   `long` or `String` fields respectively). *This parameter is only used for static
    ///   fields*. Its value is ignored for non static fields, which must be initialized through
    ///   bytecode instructions in constructors or methods.
    ///
    /// Returns a visitor to visit field annotations and attributes, or `None` if this class
    /// visitor is not interested in visiting these annotations and attributes.
    fn visit_field<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        self.delegate()?
            .visit_field(access, name, descriptor, signature, value)
    }

    /// Visits a method of the class. This method *must* return a new [`MethodVisitor`] (or
    /// `None`) each time it is called, i.e., it should not return a previously returned visitor.
    ///
    /// - `access`: the method's access flags (see [`crate::opcodes`]). This parameter also
    ///   indicates if the method is synthetic and/or deprecated.
    /// - `name`: the method's name.
    /// - `descriptor`: the method's descriptor (see [`Type`](crate::Type)).
    /// - `signature`: the method's signature. `None` if the method parameters, return type and
    ///   exceptions do not use generic types.
    /// - `exceptions`: the internal names of the method's exception classes.
    ///
    /// Returns an object to visit the byte code of the method, or `None` if this class visitor is
    /// not interested in visiting the code of this method.
    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        self.delegate()?
            .visit_method(access, name, descriptor, signature, exceptions)
    }

    /// Visits the end of the class. This method, which is the last one to be called, is used to
    /// inform the visitor that all the fields and methods of the class have been visited.
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

macro_rules! forward_class_visitor {
    ($($ty:ty),*) => {$(
        impl<T: ClassVisitor + ?Sized> ClassVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
                (**self).delegate()
            }
            fn class_writer_flags(&self) -> Option<ClassWriterFlags> {
                (**self).class_writer_flags()
            }
            fn visit(
                &mut self,
                version: u32,
                access: u32,
                name: &JavaStr,
                signature: Option<&JavaStr>,
                super_name: Option<&JavaStr>,
                interfaces: &[&JavaStr],
            ) {
                (**self).visit(version, access, name, signature, super_name, interfaces);
            }
            fn visit_member_counts(&mut self, fields_count: u16, methods_count: u16) {
                (**self).visit_member_counts(fields_count, methods_count);
            }
            fn visit_source(&mut self, source: Option<&JavaStr>, debug: Option<&JavaStr>) {
                (**self).visit_source(source, debug);
            }
            fn visit_module<'a>(
                &'a mut self,
                name: &JavaStr,
                access: u32,
                version: Option<&JavaStr>,
            ) -> Option<Box<dyn ModuleVisitor + 'a>> {
                (**self).visit_module(name, access, version)
            }
            fn visit_nest_host(&mut self, nest_host: &JavaStr) {
                (**self).visit_nest_host(nest_host);
            }
            fn visit_outer_class(
                &mut self,
                owner: &JavaStr,
                name: Option<&JavaStr>,
                descriptor: Option<&JavaStr>,
            ) {
                (**self).visit_outer_class(owner, name, descriptor);
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
            fn visit_nest_member(&mut self, nest_member: &JavaStr) {
                (**self).visit_nest_member(nest_member);
            }
            fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
                (**self).visit_permitted_subclass(permitted_subclass);
            }
            fn visit_inner_class(
                &mut self,
                name: &JavaStr,
                outer_name: Option<&JavaStr>,
                inner_name: Option<&JavaStr>,
                access: u32,
            ) {
                (**self).visit_inner_class(name, outer_name, inner_name, access);
            }
            fn visit_record_component<'a>(
                &'a mut self,
                name: &JavaStr,
                descriptor: &JavaStr,
                signature: Option<&JavaStr>,
            ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
                (**self).visit_record_component(name, descriptor, signature)
            }
            fn visit_field<'a>(
                &'a mut self,
                access: u32,
                name: &JavaStr,
                descriptor: &JavaStr,
                signature: Option<&JavaStr>,
                value: Option<&Constant<'_>>,
            ) -> Option<Box<dyn FieldVisitor + 'a>> {
                (**self).visit_field(access, name, descriptor, signature, value)
            }
            fn visit_method<'a>(
                &'a mut self,
                access: u32,
                name: &JavaStr,
                descriptor: &JavaStr,
                signature: Option<&JavaStr>,
                exceptions: &[&JavaStr],
            ) -> Option<Box<dyn MethodVisitor + 'a>> {
                (**self).visit_method(access, name, descriptor, signature, exceptions)
            }
            fn visit_end(&mut self) {
                (**self).visit_end();
            }
        }
    )*};
}

forward_class_visitor!(&mut T, Box<T>);
