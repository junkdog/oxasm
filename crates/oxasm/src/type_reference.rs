//! [`TypeReference`]: a reference to a type appearing in a class, field or method declaration,
//! or on an instruction.

/// A reference to a type appearing in a class, field or method declaration, or on an instruction.
/// Such a reference designates the part of the class where the referenced type is appearing (e.g.
/// an 'extends', 'implements' or 'throws' clause, a 'new' instruction, a 'catch' clause, a type
/// cast, a local variable declaration, etc).
///
/// The value is the `target_type` and `target_info` of a `type_annotation` structure, packed as
/// in ASM: the sort (`target_type`) in the most significant byte, the target info below it.
///
/// See [JVMS 4.7.20](https://docs.oracle.com/javase/specs/jvms/se25/html/jvms-4.html#jvms-4.7.20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeReference(u32);

impl TypeReference {
    /// The sort of type references that target a type parameter of a generic class. See
    /// [`TypeReference::get_sort`].
    pub const CLASS_TYPE_PARAMETER: u8 = 0x00;
    /// The sort of type references that target a type parameter of a generic method.
    pub const METHOD_TYPE_PARAMETER: u8 = 0x01;
    /// The sort of type references that target the super class of a class or one of the
    /// interfaces it implements.
    pub const CLASS_EXTENDS: u8 = 0x10;
    /// The sort of type references that target a bound of a type parameter of a generic class.
    pub const CLASS_TYPE_PARAMETER_BOUND: u8 = 0x11;
    /// The sort of type references that target a bound of a type parameter of a generic method.
    pub const METHOD_TYPE_PARAMETER_BOUND: u8 = 0x12;
    /// The sort of type references that target the type of a field.
    pub const FIELD: u8 = 0x13;
    /// The sort of type references that target the return type of a method.
    pub const METHOD_RETURN: u8 = 0x14;
    /// The sort of type references that target the receiver type of a method.
    pub const METHOD_RECEIVER: u8 = 0x15;
    /// The sort of type references that target the type of a formal parameter of a method.
    pub const METHOD_FORMAL_PARAMETER: u8 = 0x16;
    /// The sort of type references that target the type of an exception declared in the throws
    /// clause of a method.
    pub const THROWS: u8 = 0x17;
    /// The sort of type references that target the type of a local variable in a method.
    pub const LOCAL_VARIABLE: u8 = 0x40;
    /// The sort of type references that target the type of a resource variable in a method.
    pub const RESOURCE_VARIABLE: u8 = 0x41;
    /// The sort of type references that target the type of the exception of a 'catch' clause in
    /// a method.
    pub const EXCEPTION_PARAMETER: u8 = 0x42;
    /// The sort of type references that target the type declared in an 'instanceof'
    /// instruction.
    pub const INSTANCEOF: u8 = 0x43;
    /// The sort of type references that target the type of the object created by a 'new'
    /// instruction.
    pub const NEW: u8 = 0x44;
    /// The sort of type references that target the receiver type of a constructor reference.
    pub const CONSTRUCTOR_REFERENCE: u8 = 0x45;
    /// The sort of type references that target the receiver type of a method reference.
    pub const METHOD_REFERENCE: u8 = 0x46;
    /// The sort of type references that target the type declared in an explicit or implicit cast
    /// instruction.
    pub const CAST: u8 = 0x47;
    /// The sort of type references that target a type parameter of a generic constructor in a
    /// constructor call.
    pub const CONSTRUCTOR_INVOCATION_TYPE_ARGUMENT: u8 = 0x48;
    /// The sort of type references that target a type parameter of a generic method in a method
    /// call.
    pub const METHOD_INVOCATION_TYPE_ARGUMENT: u8 = 0x49;
    /// The sort of type references that target a type parameter of a generic constructor in a
    /// constructor reference.
    pub const CONSTRUCTOR_REFERENCE_TYPE_ARGUMENT: u8 = 0x4a;
    /// The sort of type references that target a type parameter of a generic method in a method
    /// reference.
    pub const METHOD_REFERENCE_TYPE_ARGUMENT: u8 = 0x4b;

    /// Constructs a type reference from its packed value (ASM's `new TypeReference(int)`), e.g.
    /// one previously returned by [`TypeReference::get_value`].
    #[inline]
    #[must_use]
    pub const fn new(type_ref: u32) -> TypeReference {
        TypeReference(type_ref)
    }

    /// Returns a type reference of the given sort: [`TypeReference::FIELD`],
    /// [`TypeReference::METHOD_RETURN`], [`TypeReference::METHOD_RECEIVER`],
    /// [`TypeReference::LOCAL_VARIABLE`], [`TypeReference::RESOURCE_VARIABLE`],
    /// [`TypeReference::INSTANCEOF`], [`TypeReference::NEW`],
    /// [`TypeReference::CONSTRUCTOR_REFERENCE`] or [`TypeReference::METHOD_REFERENCE`].
    #[must_use]
    pub const fn new_type_reference(sort: u8) -> TypeReference {
        TypeReference((sort as u32) << 24)
    }

    /// Returns a reference to a type parameter of a generic class or method
    /// ([`TypeReference::CLASS_TYPE_PARAMETER`] or [`TypeReference::METHOD_TYPE_PARAMETER`]).
    #[must_use]
    pub const fn new_type_parameter_reference(sort: u8, param_index: u8) -> TypeReference {
        TypeReference(((sort as u32) << 24) | ((param_index as u32) << 16))
    }

    /// Returns a reference to a type parameter bound of a generic class or method
    /// ([`TypeReference::CLASS_TYPE_PARAMETER_BOUND`] or
    /// [`TypeReference::METHOD_TYPE_PARAMETER_BOUND`]).
    #[must_use]
    pub const fn new_type_parameter_bound_reference(
        sort: u8,
        param_index: u8,
        bound_index: u8,
    ) -> TypeReference {
        TypeReference(
            ((sort as u32) << 24) | ((param_index as u32) << 16) | ((bound_index as u32) << 8),
        )
    }

    /// Returns a reference to the super class or to an interface of the 'implements' clause of a
    /// class: `itf_index` is the index of an interface in the 'implements' clause of a class, or
    /// -1 to reference the super class of the class.
    #[must_use]
    pub const fn new_super_type_reference(itf_index: i32) -> TypeReference {
        #[allow(clippy::cast_sign_loss)] // the low 16 bits, as Java's `itfIndex & 0xFFFF`
        let itf = (itf_index & 0xffff) as u32;
        TypeReference(((TypeReference::CLASS_EXTENDS as u32) << 24) | (itf << 8))
    }

    /// Returns a reference to the type of a formal parameter of a method.
    #[must_use]
    pub const fn new_formal_parameter_reference(param_index: u8) -> TypeReference {
        TypeReference(
            ((TypeReference::METHOD_FORMAL_PARAMETER as u32) << 24) | ((param_index as u32) << 16),
        )
    }

    /// Returns a reference to the type of an exception, in a 'throws' clause of a method.
    #[must_use]
    pub const fn new_exception_reference(exception_index: u16) -> TypeReference {
        TypeReference(((TypeReference::THROWS as u32) << 24) | ((exception_index as u32) << 8))
    }

    /// Returns a reference to the type of the exception declared in a 'catch' clause of a method.
    #[must_use]
    pub const fn new_try_catch_reference(try_catch_block_index: u16) -> TypeReference {
        TypeReference(
            ((TypeReference::EXCEPTION_PARAMETER as u32) << 24)
                | ((try_catch_block_index as u32) << 8),
        )
    }

    /// Returns a reference to the type of a type argument in a constructor or method call or
    /// reference ([`TypeReference::CAST`] to
    /// [`TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT`]).
    #[must_use]
    pub const fn new_type_argument_reference(sort: u8, arg_index: u8) -> TypeReference {
        TypeReference(((sort as u32) << 24) | arg_index as u32)
    }

    /// Returns the sort of this type reference (one of the constants of this type).
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // the top byte
    pub const fn get_sort(self) -> u8 {
        (self.0 >> 24) as u8
    }

    /// Returns the index of the type parameter referenced by this type reference. This method
    /// must only be used for type references whose sort is a `*_TYPE_PARAMETER` or
    /// `*_TYPE_PARAMETER_BOUND`.
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to a byte
    pub const fn get_type_parameter_index(self) -> u8 {
        ((self.0 & 0x00ff_0000) >> 16) as u8
    }

    /// Returns the index of the type parameter bound, within the type parameter
    /// [`TypeReference::get_type_parameter_index`], referenced by this type reference. This
    /// method must only be used for type references whose sort is a `*_TYPE_PARAMETER_BOUND`.
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to a byte
    pub const fn get_type_parameter_bound_index(self) -> u8 {
        ((self.0 & 0x0000_ff00) >> 8) as u8
    }

    /// Returns the index of the "super type" of a class that is referenced by this type
    /// reference: the index of an interface in the 'implements' clause, or -1 for the super
    /// class. This method must only be used for type references whose sort is
    /// [`TypeReference::CLASS_EXTENDS`].
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // Java's `(short)`
    pub const fn get_super_type_index(self) -> i32 {
        ((self.0 & 0x00ff_ff00) >> 8) as u16 as i16 as i32
    }

    /// Returns the index of the formal parameter whose type is referenced by this type reference.
    /// This method must only be used for type references whose sort is
    /// [`TypeReference::METHOD_FORMAL_PARAMETER`].
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to a byte
    pub const fn get_formal_parameter_index(self) -> u8 {
        ((self.0 & 0x00ff_0000) >> 16) as u8
    }

    /// Returns the index of the exception, in a 'throws' clause of a method, whose type is
    /// referenced by this type reference. This method must only be used for type references whose
    /// sort is [`TypeReference::THROWS`].
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to 16 bits
    pub const fn get_exception_index(self) -> u16 {
        ((self.0 & 0x00ff_ff00) >> 8) as u16
    }

    /// Returns the index of the try catch block (using the order in which they are visited with
    /// `visit_try_catch_block`), whose 'catch' type is referenced by this type reference. This
    /// method must only be used for type references whose sort is
    /// [`TypeReference::EXCEPTION_PARAMETER`].
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to 16 bits
    pub const fn get_try_catch_block_index(self) -> u16 {
        ((self.0 & 0x00ff_ff00) >> 8) as u16
    }

    /// Returns the index of the type argument referenced by this type reference. This method must
    /// only be used for type references whose sort is [`TypeReference::CAST`] to
    /// [`TypeReference::METHOD_REFERENCE_TYPE_ARGUMENT`].
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to a byte
    pub const fn get_type_argument_index(self) -> u8 {
        (self.0 & 0xff) as u8
    }

    /// Returns the int encoded value of this type reference, suitable for use in visit methods
    /// related to type annotations, like `visit_type_annotation`.
    #[inline]
    #[must_use]
    pub const fn get_value(self) -> u32 {
        self.0
    }
}

impl From<u32> for TypeReference {
    fn from(type_ref: u32) -> Self {
        TypeReference(type_ref)
    }
}

impl From<TypeReference> for u32 {
    fn from(type_ref: TypeReference) -> Self {
        type_ref.0
    }
}
