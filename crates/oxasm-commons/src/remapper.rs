//! [`Remapper`]: ASM's `Remapper`.

use std::borrow::Cow;

use oxasm::opcodes::HandleKind;
use oxasm::signature::{SignatureReader, SignatureVisitor, SignatureWriter};
use oxasm::{
    AnnotationValue, Constant, ConstantDynamic, Handle, JavaStr, JavaString, Type, TypeSort, jstr,
};

use crate::signature_remapper::SignatureRemapper;
use crate::support::illegal_argument;

/// The class name of LambdaMetafactory.
const LAMBDA_FACTORY_CLASSNAME: &JavaStr = jstr!("java/lang/invoke/LambdaMetafactory");

/// The method signature of `LambdaMetafactory.metafactory(...)`.
const LAMBDA_FACTORY_METAFACTORY: &JavaStr = jstr!(
    "(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/invoke/MethodType;\
     Ljava/lang/invoke/MethodType;Ljava/lang/invoke/MethodHandle;\
     Ljava/lang/invoke/MethodType;)Ljava/lang/invoke/CallSite;"
);

/// The method signature of `LambdaMetafactory.altMetafactory(...)`.
const LAMBDA_FACTORY_ALTMETAFACTORY: &JavaStr = jstr!(
    "(Ljava/lang/invoke/MethodHandles$Lookup;Ljava/lang/String;Ljava/lang/invoke/MethodType;\
     [Ljava/lang/Object;)Ljava/lang/invoke/CallSite;"
);

/// A class responsible for remapping types and names (ADR-13: ASM's abstract class, as a trait
/// whose methods have ASM's implementations as defaults).
///
/// Every method returns a [`Cow`] that borrows its arguments (or the remapper) when nothing
/// changes, so that an identity mapping allocates little. Where ASM's methods return `null` (only
/// [`map`](Remapper::map), for "no mapping"), these return `None`.
///
/// A method overriding a default can call that default, as Java calls `super.mapDesc(...)`,
/// through the functions of [`defaults`].
///
/// ASM's `api` versions are gone (ADR-09): this is ASM's remapper for the latest api, whose
/// invokedynamic and constant dynamic names go through the four argument
/// [`map_invoke_dynamic_method_name`](Remapper::map_invoke_dynamic_method_name). ASM's
/// deprecated two argument overload, used only by remappers created with ASM's deprecated
/// constructors, has no counterpart.
///
/// # Panics
///
/// The default methods panic where ASM throws: on malformed descriptors and signatures (ASM's
/// `StringIndexOutOfBoundsException`s and `IllegalArgumentException`s).
pub trait Remapper {
    /// Returns the given descriptor, remapped with [`map`](Remapper::map).
    fn map_desc<'a>(&'a self, descriptor: &'a JavaStr) -> Cow<'a, JavaStr> {
        defaults::map_desc(self, descriptor)
    }

    /// Returns the given internal name, remapped with [`map`](Remapper::map) (ASM's
    /// `mapType(String)`). The name of an array type (starting with `[`) has its element type
    /// remapped. ASM returns `null` for `null`: callers map `Option`s with this method.
    fn map_type<'a>(&'a self, internal_name: &'a JavaStr) -> Cow<'a, JavaStr> {
        defaults::map_type(self, internal_name)
    }

    /// Returns the given internal names, remapped with [`map_type`](Remapper::map_type).
    fn map_types<'a>(&'a self, internal_names: &[&'a JavaStr]) -> Vec<Cow<'a, JavaStr>> {
        internal_names
            .iter()
            .map(|internal_name| self.map_type(internal_name))
            .collect()
    }

    /// Returns the given method descriptor, with its argument and return type descriptors
    /// remapped with [`map_desc`](Remapper::map_desc).
    fn map_method_desc<'a>(&'a self, method_descriptor: &'a JavaStr) -> Cow<'a, JavaStr> {
        defaults::map_method_desc(self, method_descriptor)
    }

    /// Returns the given value, remapped with this remapper. Possible values are integers,
    /// floats, longs, doubles, strings, [`Type`]s, [`Handle`]s and [`ConstantDynamic`]s: the
    /// first five are returned unchanged by default.
    ///
    /// The annotation values that are constants (`Int`, `Long`, `Float`, `Double`, `String` and
    /// `Type`) also go through this method; the others (bytes, characters, shorts, booleans and
    /// primitive arrays), which ASM passes to `mapValue` boxed, are not remapped.
    fn map_value<'v>(&self, value: &Constant<'v>) -> Constant<'v> {
        defaults::map_value(self, value)
    }

    /// Returns the given signature, remapped with the [`SignatureVisitor`] returned by
    /// [`create_signature_remapper`](Remapper::create_signature_remapper), or `None` if
    /// `signature` is `None`. `type_signature` tells whether the signature is a field signature
    /// (`JavaTypeSignature`), or a class or method one.
    fn map_signature<'a>(
        &'a self,
        signature: Option<&'a JavaStr>,
        type_signature: bool,
    ) -> Option<Cow<'a, JavaStr>> {
        defaults::map_signature(self, signature, type_signature)
    }

    /// Constructs a new remapper for signatures, which forwards the remapped signature events to
    /// `signature_visitor`. The default is a [`SignatureRemapper`].
    fn create_signature_remapper<'a>(
        &'a self,
        signature_visitor: &'a mut dyn SignatureVisitor,
    ) -> Box<dyn SignatureVisitor + 'a> {
        Box::new(SignatureRemapper::new(signature_visitor, self))
    }

    /// Maps an annotation attribute name. The default implementation returns the given name
    /// unchanged. Subclasses can override.
    ///
    /// - `descriptor`: the descriptor of the annotation class.
    /// - `name`: the name of the annotation attribute.
    fn map_annotation_attribute_name<'a>(
        &'a self,
        _descriptor: &'a JavaStr,
        name: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        Cow::Borrowed(name)
    }

    /// Maps an inner class name to its new name. The default implementation uses
    /// [`map_type`](Remapper::map_type) on the full name of the inner class, and returns the
    /// part after the last `$` (and its following digits, for local classes), or `inner_name`
    /// if the class name does not change or has no `$`.
    ///
    /// - `name`: the fully-qualified internal name of the inner class.
    /// - `owner_name`: the internal name of the owner class of the inner class.
    /// - `inner_name`: the internal name of the inner class.
    fn map_inner_class_name<'a>(
        &'a self,
        name: &'a JavaStr,
        owner_name: Option<&'a JavaStr>,
        inner_name: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        defaults::map_inner_class_name(self, name, owner_name, inner_name)
    }

    /// Maps a method name to its new name. The default implementation returns the given name
    /// unchanged.
    ///
    /// - `owner`: the internal name of the owner class of the method.
    /// - `name`: the name of the method.
    /// - `descriptor`: the descriptor of the method.
    fn map_method_name<'a>(
        &'a self,
        _owner: &'a JavaStr,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        Cow::Borrowed(name)
    }

    /// Maps an invokedynamic or a constant dynamic method name to its new name. The default
    /// implementation tries [`map_well_known_invoke_dynamic_method_name`] first, and then
    /// [`map_basic_invoke_dynamic_method_name`] if it returns `None`. Subclasses can override.
    ///
    /// - `name`: the name of the method.
    /// - `descriptor`: the descriptor of the method.
    /// - `bootstrap_method_handle`: the bootstrap method.
    /// - `bootstrap_method_arguments`: the bootstrap method constant arguments.
    ///
    /// [`map_well_known_invoke_dynamic_method_name`]: Remapper::map_well_known_invoke_dynamic_method_name
    /// [`map_basic_invoke_dynamic_method_name`]: Remapper::map_basic_invoke_dynamic_method_name
    fn map_invoke_dynamic_method_name<'a>(
        &'a self,
        name: &'a JavaStr,
        descriptor: &'a JavaStr,
        bootstrap_method_handle: &'a Handle,
        bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Cow<'a, JavaStr> {
        defaults::map_invoke_dynamic_method_name(
            self,
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        )
    }

    /// Maps an invokedynamic or a constant dynamic method name to its new name, for well-known
    /// bootstrap methods: those of `java.lang.invoke.LambdaMetafactory`, whose name is the
    /// implemented interface method's, mapped with [`map_method_name`](Remapper::map_method_name).
    /// Returns `None` for other bootstrap methods.
    ///
    /// # Panics
    ///
    /// For a `LambdaMetafactory` bootstrap method without arguments (ASM's
    /// `ArrayIndexOutOfBoundsException`).
    fn map_well_known_invoke_dynamic_method_name<'a>(
        &'a self,
        name: &'a JavaStr,
        descriptor: &'a JavaStr,
        bootstrap_method_handle: &'a Handle,
        bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Option<Cow<'a, JavaStr>> {
        defaults::map_well_known_invoke_dynamic_method_name(
            self,
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        )
    }

    /// Maps an invokedynamic or a constant dynamic method name to its new name, for the
    /// bootstrap methods that are not well known. The default implementation returns the given
    /// name unchanged. Subclasses can override.
    fn map_basic_invoke_dynamic_method_name<'a>(
        &'a self,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
        _bootstrap_method_handle: &'a Handle,
        _bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Cow<'a, JavaStr> {
        Cow::Borrowed(name)
    }

    /// Maps a record component name to its new name. The default implementation returns the
    /// given name unchanged.
    ///
    /// - `owner`: the internal name of the owner class of the record component.
    /// - `name`: the name of the record component.
    /// - `descriptor`: the descriptor of the record component.
    fn map_record_component_name<'a>(
        &'a self,
        _owner: &'a JavaStr,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        Cow::Borrowed(name)
    }

    /// Maps a field name to its new name. The default implementation returns the given name
    /// unchanged.
    ///
    /// - `owner`: the internal name of the owner class of the field.
    /// - `name`: the name of the field.
    /// - `descriptor`: the descriptor of the field.
    fn map_field_name<'a>(
        &'a self,
        _owner: &'a JavaStr,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        Cow::Borrowed(name)
    }

    /// Maps a package name to its new name. The default implementation returns the given name
    /// unchanged.
    fn map_package_name<'a>(&'a self, name: &'a JavaStr) -> Cow<'a, JavaStr> {
        Cow::Borrowed(name)
    }

    /// Maps a module name to its new name. The default implementation returns the given name
    /// unchanged.
    fn map_module_name<'a>(&'a self, name: &'a JavaStr) -> Cow<'a, JavaStr> {
        Cow::Borrowed(name)
    }

    /// Maps the internal name of a class to its new name. The default implementation returns
    /// the given name unchanged. Returns `None` (ASM's `null`) to keep the name unchanged too:
    /// what [`SimpleRemapper`](crate::SimpleRemapper) returns for names it has no mapping for.
    fn map<'a>(&'a self, internal_name: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
        Some(Cow::Borrowed(internal_name))
    }
}

macro_rules! forward_remapper {
    ($($ty:ty),*) => {$(
        impl<T: Remapper + ?Sized> Remapper for $ty {
            fn map_desc<'a>(&'a self, descriptor: &'a JavaStr) -> Cow<'a, JavaStr> {
                (**self).map_desc(descriptor)
            }
            fn map_type<'a>(&'a self, internal_name: &'a JavaStr) -> Cow<'a, JavaStr> {
                (**self).map_type(internal_name)
            }
            fn map_types<'a>(&'a self, internal_names: &[&'a JavaStr]) -> Vec<Cow<'a, JavaStr>> {
                (**self).map_types(internal_names)
            }
            fn map_method_desc<'a>(&'a self, method_descriptor: &'a JavaStr) -> Cow<'a, JavaStr> {
                (**self).map_method_desc(method_descriptor)
            }
            fn map_value<'v>(&self, value: &Constant<'v>) -> Constant<'v> {
                (**self).map_value(value)
            }
            fn map_signature<'a>(
                &'a self,
                signature: Option<&'a JavaStr>,
                type_signature: bool,
            ) -> Option<Cow<'a, JavaStr>> {
                (**self).map_signature(signature, type_signature)
            }
            fn create_signature_remapper<'a>(
                &'a self,
                signature_visitor: &'a mut dyn SignatureVisitor,
            ) -> Box<dyn SignatureVisitor + 'a> {
                (**self).create_signature_remapper(signature_visitor)
            }
            fn map_annotation_attribute_name<'a>(
                &'a self,
                descriptor: &'a JavaStr,
                name: &'a JavaStr,
            ) -> Cow<'a, JavaStr> {
                (**self).map_annotation_attribute_name(descriptor, name)
            }
            fn map_inner_class_name<'a>(
                &'a self,
                name: &'a JavaStr,
                owner_name: Option<&'a JavaStr>,
                inner_name: &'a JavaStr,
            ) -> Cow<'a, JavaStr> {
                (**self).map_inner_class_name(name, owner_name, inner_name)
            }
            fn map_method_name<'a>(
                &'a self,
                owner: &'a JavaStr,
                name: &'a JavaStr,
                descriptor: &'a JavaStr,
            ) -> Cow<'a, JavaStr> {
                (**self).map_method_name(owner, name, descriptor)
            }
            fn map_invoke_dynamic_method_name<'a>(
                &'a self,
                name: &'a JavaStr,
                descriptor: &'a JavaStr,
                bootstrap_method_handle: &'a Handle,
                bootstrap_method_arguments: &'a [Constant<'_>],
            ) -> Cow<'a, JavaStr> {
                (**self).map_invoke_dynamic_method_name(
                    name,
                    descriptor,
                    bootstrap_method_handle,
                    bootstrap_method_arguments,
                )
            }
            fn map_well_known_invoke_dynamic_method_name<'a>(
                &'a self,
                name: &'a JavaStr,
                descriptor: &'a JavaStr,
                bootstrap_method_handle: &'a Handle,
                bootstrap_method_arguments: &'a [Constant<'_>],
            ) -> Option<Cow<'a, JavaStr>> {
                (**self).map_well_known_invoke_dynamic_method_name(
                    name,
                    descriptor,
                    bootstrap_method_handle,
                    bootstrap_method_arguments,
                )
            }
            fn map_basic_invoke_dynamic_method_name<'a>(
                &'a self,
                name: &'a JavaStr,
                descriptor: &'a JavaStr,
                bootstrap_method_handle: &'a Handle,
                bootstrap_method_arguments: &'a [Constant<'_>],
            ) -> Cow<'a, JavaStr> {
                (**self).map_basic_invoke_dynamic_method_name(
                    name,
                    descriptor,
                    bootstrap_method_handle,
                    bootstrap_method_arguments,
                )
            }
            fn map_record_component_name<'a>(
                &'a self,
                owner: &'a JavaStr,
                name: &'a JavaStr,
                descriptor: &'a JavaStr,
            ) -> Cow<'a, JavaStr> {
                (**self).map_record_component_name(owner, name, descriptor)
            }
            fn map_field_name<'a>(
                &'a self,
                owner: &'a JavaStr,
                name: &'a JavaStr,
                descriptor: &'a JavaStr,
            ) -> Cow<'a, JavaStr> {
                (**self).map_field_name(owner, name, descriptor)
            }
            fn map_package_name<'a>(&'a self, name: &'a JavaStr) -> Cow<'a, JavaStr> {
                (**self).map_package_name(name)
            }
            fn map_module_name<'a>(&'a self, name: &'a JavaStr) -> Cow<'a, JavaStr> {
                (**self).map_module_name(name)
            }
            fn map<'a>(&'a self, internal_name: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
                (**self).map(internal_name)
            }
        }
    )*};
}

forward_remapper!(&T, Box<T>);

/// The default implementations of the [`Remapper`] methods (ASM's `Remapper` methods), which an
/// overriding method can call as Java calls `super.mapXxx(...)`.
pub mod defaults {
    use super::*;

    /// ASM's `mapDesc`.
    pub fn map_desc<'a, R: Remapper + ?Sized>(
        remapper: &'a R,
        descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        // A primitive type maps to itself, without building a Type.
        if let [b'V' | b'Z' | b'C' | b'B' | b'S' | b'I' | b'F' | b'J' | b'D'] =
            descriptor.as_bytes()
        {
            return Cow::Borrowed(descriptor);
        }
        let remapped = map_type_value(remapper, &Type::get_type(descriptor));
        unchanged_or_owned(descriptor, &remapped.get_descriptor())
    }

    /// ASM's private `mapType(Type)`: the given type, remapped with [`Remapper::map`] (object
    /// types and array element types) and [`Remapper::map_method_desc`] (method types).
    pub fn map_type_value<'a, R: Remapper + ?Sized>(remapper: &R, type_: &Type<'a>) -> Type<'a> {
        match type_.get_sort() {
            TypeSort::Array => {
                let mut remapped_descriptor = JavaString::new();
                for _ in 0..type_.get_dimensions() {
                    remapped_descriptor.push('[');
                }
                remapped_descriptor.push_java_str(
                    &map_type_value(remapper, &type_.get_element_type()).get_descriptor(),
                );
                Type::get_type(&remapped_descriptor).into_owned()
            }
            TypeSort::Object => match remapper.map(type_.get_internal_name()) {
                Some(remapped_internal_name) => {
                    crate::support::object_type(&remapped_internal_name).into_owned()
                }
                None => type_.clone(),
            },
            TypeSort::Method => {
                Type::get_method_type(&*remapper.map_method_desc(&type_.get_descriptor()))
                    .into_owned()
            }
            _ => type_.clone(),
        }
    }

    /// ASM's `mapType(String)`.
    pub fn map_type<'a, R: Remapper + ?Sized>(
        remapper: &'a R,
        internal_name: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        if !internal_name.is_empty() && !internal_name.starts_with('[') {
            // The common case, without building a Type: ASM's getObjectType gives an object
            // type, which mapType(Type) maps with map. (An empty name throws in getObjectType.)
            return remapper
                .map(internal_name)
                .unwrap_or(Cow::Borrowed(internal_name));
        }
        let remapped = map_type_value(remapper, &crate::support::object_type(internal_name));
        unchanged_or_owned(internal_name, remapped.get_internal_name())
    }

    /// ASM's `mapMethodDesc`.
    pub fn map_method_desc<'a, R: Remapper + ?Sized>(
        remapper: &'a R,
        method_descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        if method_descriptor == "()V" {
            return Cow::Borrowed(method_descriptor);
        }

        let mut string_builder = JavaString::from("(");
        for argument_type in Type::get_argument_types_of(method_descriptor) {
            string_builder
                .push_java_str(&map_type_value(remapper, &argument_type).get_descriptor());
        }
        let return_type = Type::get_return_type_of(method_descriptor);
        if return_type.get_sort() == TypeSort::Void {
            string_builder.push_str(")V");
        } else {
            string_builder.push(')');
            string_builder.push_java_str(&map_type_value(remapper, &return_type).get_descriptor());
        }
        unchanged_or_owned(method_descriptor, &string_builder)
    }

    /// ASM's `mapValue`.
    pub fn map_value<'v, R: Remapper + ?Sized>(remapper: &R, value: &Constant<'v>) -> Constant<'v> {
        match value {
            Constant::Type(type_) => Constant::Type(map_type_value(remapper, type_)),
            Constant::Handle(handle) => Constant::Handle(map_handle(remapper, handle)),
            Constant::Dynamic(constant_dynamic) => {
                let name = constant_dynamic.get_name();
                let descriptor = constant_dynamic.get_descriptor();
                let bootstrap_method = constant_dynamic.get_bootstrap_method();
                let bootstrap_method_arguments = constant_dynamic.get_bootstrap_method_arguments();
                let remapped_bootstrap_method_arguments: Vec<Constant<'static>> =
                    bootstrap_method_arguments
                        .iter()
                        .map(|argument| remapper.map_value(argument))
                        .collect();
                let remapped_name = remapper.map_invoke_dynamic_method_name(
                    name,
                    descriptor,
                    bootstrap_method,
                    bootstrap_method_arguments,
                );
                Constant::Dynamic(Box::new(ConstantDynamic::new(
                    remapped_name,
                    remapper.map_desc(descriptor),
                    map_handle_value(remapper, bootstrap_method),
                    remapped_bootstrap_method_arguments,
                )))
            }
            _ => value.clone(),
        }
    }

    /// The `Handle` case of ASM's `mapValue`.
    fn map_handle<R: Remapper + ?Sized>(remapper: &R, handle: &Handle) -> Handle {
        let is_field_handle = handle.get_tag() <= HandleKind::PutStatic;
        let (owner, name, descriptor) = (handle.get_owner(), handle.get_name(), handle.get_desc());
        Handle::new(
            handle.get_tag(),
            remapper.map_type(owner),
            if is_field_handle {
                remapper.map_field_name(owner, name, descriptor)
            } else {
                remapper.map_method_name(owner, name, descriptor)
            },
            if is_field_handle {
                remapper.map_desc(descriptor)
            } else {
                remapper.map_method_desc(descriptor)
            },
            handle.is_interface(),
        )
    }

    /// ASM's `(Handle) mapValue(handle)`: a handle remapped with [`Remapper::map_value`].
    ///
    /// # Panics
    ///
    /// If `map_value` does not return a handle (ASM's `ClassCastException`).
    pub fn map_handle_value<R: Remapper + ?Sized>(remapper: &R, handle: &Handle) -> Handle {
        match remapper.map_value(&Constant::Handle(handle.clone())) {
            Constant::Handle(handle) => handle,
            value => panic!("java.lang.ClassCastException: {value} is not a Handle"),
        }
    }

    /// ASM's `mapValue` for an annotation value: the values that are constants go through
    /// [`Remapper::map_value`], the others are returned unchanged (see there).
    ///
    /// # Panics
    ///
    /// If `map_value` returns a handle or a constant dynamic, which annotations cannot hold
    /// (ASM's `AnnotationWriter` throws an `IllegalArgumentException`).
    pub fn map_annotation_value<'v, R: Remapper + ?Sized>(
        remapper: &R,
        value: &AnnotationValue<'v>,
    ) -> AnnotationValue<'v> {
        let constant = match value {
            AnnotationValue::Int(v) => Constant::Integer(*v),
            AnnotationValue::Long(v) => Constant::Long(*v),
            AnnotationValue::Float(v) => Constant::Float(*v),
            AnnotationValue::Double(v) => Constant::Double(*v),
            AnnotationValue::String(s) => Constant::String(s.clone()),
            AnnotationValue::Type(t) => Constant::Type(t.clone()),
            _ => return value.clone(),
        };
        match remapper.map_value(&constant) {
            Constant::Integer(v) => AnnotationValue::Int(v),
            Constant::Long(v) => AnnotationValue::Long(v),
            Constant::Float(v) => AnnotationValue::Float(v),
            Constant::Double(v) => AnnotationValue::Double(v),
            Constant::String(s) => AnnotationValue::String(s),
            Constant::Type(t) => AnnotationValue::Type(t),
            value => illegal_argument(format_args!("Unsupported annotation value: {value}")),
        }
    }

    /// ASM's `mapSignature`.
    ///
    /// # Panics
    ///
    /// If the signature is malformed (ASM throws).
    pub fn map_signature<'a, R: Remapper + ?Sized>(
        remapper: &'a R,
        signature: Option<&'a JavaStr>,
        type_signature: bool,
    ) -> Option<Cow<'a, JavaStr>> {
        let signature = signature?;
        let signature_reader = SignatureReader::new(signature);
        let mut signature_writer = SignatureWriter::new();
        {
            let mut signature_remapper = remapper.create_signature_remapper(&mut signature_writer);
            let result = if type_signature {
                signature_reader.accept_type(&mut *signature_remapper)
            } else {
                signature_reader.accept(&mut *signature_remapper)
            };
            if let Err(error) = result {
                // ASM throws an IllegalArgumentException or, at the end of the signature, a
                // StringIndexOutOfBoundsException, whose message oxasm's reader does not keep
                // (ADR-14): the class of the first, and the reader's message.
                crate::support::illegal_argument(error);
            }
        }
        Some(unchanged_or_owned(
            signature,
            signature_writer.as_java_str(),
        ))
    }

    /// ASM's `mapInnerClassName`.
    pub fn map_inner_class_name<'a, R: Remapper + ?Sized>(
        remapper: &'a R,
        name: &'a JavaStr,
        _owner_name: Option<&'a JavaStr>,
        inner_name: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        let remapped_inner_name = remapper.map_type(name);

        if *remapped_inner_name == *name {
            return Cow::Borrowed(inner_name);
        } else {
            let origin_split = name.rfind('/');
            let remapped_split = remapped_inner_name.rfind('/');
            if let (Some(origin_split), Some(remapped_split)) = (origin_split, remapped_split)
                && name[origin_split..] == remapped_inner_name[remapped_split..]
            {
                // class name not changed
                return Cow::Borrowed(inner_name);
            }
        }

        if let Some(dollar) = remapped_inner_name.rfind('$') {
            let mut index = dollar + 1;
            for (offset, c) in remapped_inner_name[index..].char_indices() {
                if !is_java_digit(c.as_u32()) {
                    index = dollar + 1 + offset;
                    break;
                }
                index = dollar + 1 + offset + c.len_semi_utf8();
            }
            Cow::Owned(remapped_inner_name[index..].to_owned())
        } else {
            Cow::Borrowed(inner_name)
        }
    }

    /// ASM's 4 argument `mapInvokeDynamicMethodName`.
    pub fn map_invoke_dynamic_method_name<'a, R: Remapper + ?Sized>(
        remapper: &'a R,
        name: &'a JavaStr,
        descriptor: &'a JavaStr,
        bootstrap_method_handle: &'a Handle,
        bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Cow<'a, JavaStr> {
        let mapped_well_known_name = remapper.map_well_known_invoke_dynamic_method_name(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        );
        if let Some(mapped_well_known_name) = mapped_well_known_name {
            return mapped_well_known_name;
        }
        remapper.map_basic_invoke_dynamic_method_name(
            name,
            descriptor,
            bootstrap_method_handle,
            bootstrap_method_arguments,
        )
    }

    /// ASM's `mapWellKnownInvokeDynamicMethodName`.
    ///
    /// # Panics
    ///
    /// For a `LambdaMetafactory` bootstrap method without arguments (ASM's
    /// `ArrayIndexOutOfBoundsException`).
    pub fn map_well_known_invoke_dynamic_method_name<'a, R: Remapper + ?Sized>(
        remapper: &'a R,
        name: &'a JavaStr,
        descriptor: &'a JavaStr,
        bootstrap_method_handle: &'a Handle,
        bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Option<Cow<'a, JavaStr>> {
        if bootstrap_method_handle.get_owner() == LAMBDA_FACTORY_CLASSNAME
            && bootstrap_method_handle.get_tag() == HandleKind::InvokeStatic
        {
            // This is a lambda creation.
            // Note: **if** is reserved for future JDK changes.
            let mut is_metafactory = false;
            is_metafactory |= bootstrap_method_handle.get_name() == "metafactory"
                && bootstrap_method_handle.get_desc() == LAMBDA_FACTORY_METAFACTORY;
            is_metafactory |= bootstrap_method_handle.get_name() == "altMetafactory"
                && bootstrap_method_handle.get_desc() == LAMBDA_FACTORY_ALTMETAFACTORY;

            if is_metafactory {
                // Note:
                // Java lambda instances are created by LambdaMetafactory.metafactory() and
                // LambdaMetafactory.altMetafactory().
                // The specification can be found in the LambdaMetafactory javadoc:
                // https://docs.oracle.com/en/java/javase/21/docs/api/java.base/java/lang/invoke/LambdaMetafactory.html
                //
                // In short, all the necessary parameters can be obtained from this
                // invokedynamic, including the following three:
                // - Class name: From return type of method descriptor.
                // - Method name: Same as the name of invokedynamic.
                // - Method descriptor: From the first bootstrap argument.
                let Some(first_argument) = bootstrap_method_arguments.first() else {
                    panic!(
                        "java.lang.ArrayIndexOutOfBoundsException: Index 0 out of bounds for \
                         length 0"
                    );
                };
                let owner = Type::get_return_type_of(descriptor);
                let method_descriptor = JavaString::from(first_argument.to_string());
                return Some(Cow::Owned(
                    remapper
                        .map_method_name(owner.get_internal_name(), name, &method_descriptor)
                        .into_owned(),
                ));
            }
        }

        None
    }

    /// `remapped` borrowed as `original` if they are equal, or owned.
    fn unchanged_or_owned<'a>(original: &'a JavaStr, remapped: &JavaStr) -> Cow<'a, JavaStr> {
        if original == remapped {
            Cow::Borrowed(original)
        } else {
            Cow::Owned(remapped.to_owned())
        }
    }
}

/// Java's `Character.isDigit(char)` (Java 25's tables): whether a UTF-16 code unit is a decimal
/// digit (general category Nd). Every such range of the BMP has 10 digits.
fn is_java_digit(c: u32) -> bool {
    const DIGIT_RANGES: [u32; 37] = [
        0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66,
        0x0CE6, 0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946,
        0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0,
        0xA9F0, 0xAA50, 0xABF0, 0xFF10,
    ];
    DIGIT_RANGES
        .iter()
        .any(|&start| (start..start + 10).contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_digits() {
        assert!(is_java_digit(u32::from('0')) && is_java_digit(u32::from('9')));
        assert!(!is_java_digit(u32::from('a')) && !is_java_digit(u32::from('/')));
        assert!(is_java_digit(0x0669) && !is_java_digit(0x066A));
        assert!(is_java_digit(0xFF19) && !is_java_digit(0xFF1A));
        // A supplementary digit is two UTF-16 units, neither of which is a digit.
        assert!(!is_java_digit(0x104A0));
    }
}
