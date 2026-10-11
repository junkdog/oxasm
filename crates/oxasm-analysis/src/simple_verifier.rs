//! [`SimpleVerifier`]: ASM's `SimpleVerifier`, over a [`ClassHierarchy`].

use std::borrow::Cow;

use oxasm::hierarchy::HierarchyError;
use oxasm::{ClassHierarchy, JavaString, Type, TypeSort};

use crate::basic_interpreter::{BasicInterpreter, assertion_error, get_type};
use crate::basic_value::BasicValue;
use crate::basic_verifier::{BasicVerifier, Verifier};
use crate::error::AnalyzerError;
use crate::interpreter::{InsnRef, Interpreter};

/// The type of the `Object` class.
const OBJECT_TYPE: Type = Type::static_object_type(oxasm::hierarchy::OBJECT);

/// The longest chain of super classes followed: it bounds cyclic hierarchies (where ASM
/// recurses or loops forever).
const MAX_DEPTH: usize = 1 << 16;

/// The deepest recursion of `is_assignable_from` through the class being verified, which only a
/// cyclic hierarchy makes deeper than 2 (ADR-06's bound for input-controlled recursion).
const MAX_RECURSION: usize = 256;

/// An extended [`BasicVerifier`] that performs more precise verifications: it computes exact
/// class types, instead of a single "object reference" type (ASM's `SimpleVerifier`).
///
/// ASM's `SimpleVerifier` loads the classes it needs to know about; this one asks a
/// [`ClassHierarchy`] (ADR-07), except for the class being verified, whose facts are given to
/// [`SimpleVerifier::for_class`]. The hierarchy knows super classes and interfaces, not the
/// interfaces a class implements, so ASM's `getClass(type1).isAssignableFrom(getClass(type2))`
/// is [`ClassHierarchy::is_assignable_from`]: this is the parity oracle's `SimpleVerifier`. A
/// class the hierarchy does not know fails the analysis with ASM's `TypeNotPresentException`
/// message.
#[derive(Clone, Debug)]
pub struct SimpleVerifier<H> {
    /// The type of the class that is verified.
    current_class: Option<Type<'static>>,
    /// The type of the super class of the class that is verified.
    current_super_class: Option<Type<'static>>,
    /// The types of the interfaces directly implemented by the class that is verified.
    current_class_interfaces: Vec<Type<'static>>,
    /// Whether the class that is verified is an interface.
    is_interface: bool,
    hierarchy: H,
}

impl<H: ClassHierarchy> SimpleVerifier<H> {
    /// Constructs a new verifier, which knows classes through `hierarchy` only (ASM's
    /// `SimpleVerifier()`).
    pub fn new(hierarchy: H) -> SimpleVerifier<H> {
        SimpleVerifier {
            current_class: None,
            current_super_class: None,
            current_class_interfaces: Vec::new(),
            is_interface: false,
            hierarchy,
        }
    }

    /// Constructs a new verifier for a specific class, whose facts are given here rather than
    /// asked to `hierarchy`, since it may be incorrect (ASM's `SimpleVerifier(Type, Type, List,
    /// boolean)`).
    pub fn for_class(
        current_class: Type<'_>,
        current_super_class: Option<Type<'_>>,
        current_class_interfaces: Vec<Type<'_>>,
        is_interface: bool,
        hierarchy: H,
    ) -> SimpleVerifier<H> {
        SimpleVerifier {
            current_class: Some(current_class.into_owned()),
            current_super_class: current_super_class.map(Type::into_owned),
            current_class_interfaces: current_class_interfaces
                .into_iter()
                .map(Type::into_owned)
                .collect(),
            is_interface,
            hierarchy,
        }
    }

    /// The class hierarchy.
    pub fn hierarchy(&self) -> &H {
        &self.hierarchy
    }

    fn is_current_class(&self, ty: &Type) -> bool {
        self.current_class.as_ref() == Some(ty)
    }

    /// Whether the given object type is an interface (ASM's `isInterface`).
    ///
    /// # Errors
    ///
    /// If the hierarchy does not know it.
    pub fn is_interface(&self, ty: &Type) -> Result<bool, AnalyzerError> {
        if self.is_current_class(ty) {
            return Ok(self.is_interface);
        }
        self.hierarchy
            .is_interface(ty.get_internal_name())
            .map_err(hierarchy_error)
    }

    /// The super class of the given object type, or `None` for `java/lang/Object` (ASM's
    /// `getSuperClass`).
    ///
    /// # Errors
    ///
    /// If the hierarchy does not know it.
    pub fn get_super_class(&self, ty: &Type<'_>) -> Result<Option<Type<'static>>, AnalyzerError> {
        if self.is_current_class(ty) {
            return Ok(self.current_super_class.clone());
        }
        Ok(self
            .hierarchy
            .super_class(ty.get_internal_name())
            .map_err(hierarchy_error)?
            .map(|name| Type::get_object_type(&name).into_owned()))
    }

    /// Whether the class of `type1` is the same as, or a super class or interface of, the class
    /// of `type2` (ASM's `isAssignableFrom`). Both are object types.
    ///
    /// # Errors
    ///
    /// If the hierarchy does not know a class.
    pub fn is_assignable_from(&self, type1: &Type, type2: &Type) -> Result<bool, AnalyzerError> {
        self.is_assignable_from_at(type1, Some(type2), 0)
    }

    fn is_assignable_from_at(
        &self,
        type1: &Type,
        type2: Option<&Type>,
        depth: usize,
    ) -> Result<bool, AnalyzerError> {
        // ASM recurses here; through the class being verified, a cyclic hierarchy recurses
        // until a StackOverflowError.
        if depth > MAX_RECURSION {
            return Err(AnalyzerError::runtime("java.lang.StackOverflowError"));
        }
        let Some(type2) = type2 else {
            return Err(null_pointer("type2"));
        };
        let mut type2 = Cow::Borrowed(type2);
        // ASM's tail recursion over the super classes of type2, as a loop.
        let mut steps = 0;
        loop {
            if *type1 == *type2 {
                return Ok(true);
            }
            if !self.is_current_class(type1) {
                break;
            }
            let Some(super_type2) = self.get_super_class(&type2)? else {
                return Ok(false);
            };
            if self.is_interface {
                // This should always be true, given the preconditions of this method, but is
                // kept for backward compatibility.
                return Ok(matches!(
                    type2.get_sort(),
                    TypeSort::Object | TypeSort::Array
                ));
            }
            steps += 1;
            if steps > MAX_DEPTH {
                return Err(AnalyzerError::runtime("java.lang.StackOverflowError"));
            }
            type2 = Cow::Owned(super_type2);
        }
        let type2 = &*type2;
        if self.is_current_class(type2) {
            if self.is_assignable_from_at(type1, self.current_super_class.as_ref(), depth + 1)? {
                return Ok(true);
            }
            for current_class_interface in &self.current_class_interfaces {
                if self.is_assignable_from_at(type1, Some(current_class_interface), depth + 1)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        self.hierarchy
            .is_assignable_from(type1.get_internal_name(), type2.get_internal_name())
            .map_err(hierarchy_error)
    }

    /// ASM's `SimpleVerifier.newValue`.
    ///
    /// # Errors
    ///
    /// On a malformed array type.
    pub fn new_value(&self, ty: Option<&Type>) -> Result<Option<BasicValue>, AnalyzerError> {
        let Some(ty) = ty else {
            return Ok(Some(BasicValue::UNINITIALIZED_VALUE));
        };
        let is_array = ty.get_sort() == TypeSort::Array;
        if is_array
            && matches!(
                element_type(ty)?.get_sort(),
                TypeSort::Boolean | TypeSort::Char | TypeSort::Byte | TypeSort::Short
            )
        {
            return Ok(Some(BasicValue::new(Some(ty.clone()))));
        }

        let value = BasicInterpreter::new_value(Some(ty))?;
        if value.as_ref() != Some(&BasicValue::REFERENCE_VALUE) {
            return Ok(value);
        }
        if is_array {
            let element_value = self.new_value(Some(&element_type(ty)?))?;
            let element_type = element_value
                .as_ref()
                .and_then(BasicValue::get_type)
                .ok_or_else(|| null_pointer("value"))?;
            let mut descriptor = JavaString::new();
            for _ in 0..ty.get_dimensions() {
                descriptor.push('[');
            }
            element_type.append_descriptor(&mut descriptor);
            Ok(Some(BasicValue::new(Some(get_type(&descriptor)?))))
        } else {
            Ok(Some(BasicValue::new(Some(ty.clone()))))
        }
    }

    /// ASM's `SimpleVerifier.merge` (`None` if the merge is `value1`).
    ///
    /// # Errors
    ///
    /// If the hierarchy does not know a class.
    pub fn merge(
        &self,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        let (Some(type1), Some(type2)) = (value1.get_type(), value2.get_type()) else {
            // Null types correspond to BasicValue.UNINITIALIZED_VALUE.
            return Ok(Some(BasicValue::UNINITIALIZED_VALUE));
        };
        if type1 == type2 {
            return Ok(None);
        }
        // The merge of a primitive type with a different type is the type of uninitialized
        // values.
        if !is_reference_type(type1) || !is_reference_type(type2) {
            return Ok(Some(BasicValue::UNINITIALIZED_VALUE));
        }
        // Special case for the type of the "null" literal.
        if *type1 == BasicInterpreter::NULL_TYPE {
            return Ok(Some(*value2));
        }
        if *type2 == BasicInterpreter::NULL_TYPE {
            return Ok(None);
        }
        // Convert type1 to its element type and array dimension. Arrays of primitive values
        // are seen as Object arrays with one dimension less. Hence the element type is always
        // of Type.OBJECT sort.
        let (mut type1, dim1) = element_and_dimensions(type1)?;
        // Do the same for type2.
        let (type2, dim2) = element_and_dimensions(type2)?;
        // The merge of array types of different dimensions is an Object array type.
        if dim1 != dim2 {
            return self.new_array_value(&OBJECT_TYPE, dim1.min(dim2)).map(Some);
        }
        // Type1 and type2 have a Type.OBJECT sort by construction (see above), as expected by
        // isAssignableFrom.
        if self.is_assignable_from(&type1, &type2)? {
            return self.new_array_value(&type1, dim1).map(Some);
        }
        if self.is_assignable_from(&type2, &type1)? {
            return self.new_array_value(&type2, dim1).map(Some);
        }
        if !self.is_interface(&type1)? {
            let mut depth = 0;
            while type1 != OBJECT_TYPE {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(AnalyzerError::runtime(format!(
                        "broken super class chain from {}",
                        type1.get_internal_name()
                    )));
                }
                type1 = self
                    .get_super_class(&type1)?
                    .ok_or_else(|| null_pointer("type1"))?;
                if self.is_assignable_from(&type1, &type2)? {
                    return self.new_array_value(&type1, dim1).map(Some);
                }
            }
        }
        self.new_array_value(&OBJECT_TYPE, dim1).map(Some)
    }

    fn new_array_value(&self, ty: &Type, dimensions: usize) -> Result<BasicValue, AnalyzerError> {
        let value = if dimensions == 0 {
            self.new_value(Some(ty))?
        } else {
            let mut descriptor = JavaString::new();
            for _ in 0..dimensions {
                descriptor.push('[');
            }
            ty.append_descriptor(&mut descriptor);
            self.new_value(Some(&get_type(&descriptor)?))?
        };
        value.ok_or_else(|| null_pointer("value"))
    }
}

impl<H: ClassHierarchy> Verifier for SimpleVerifier<H> {
    fn is_array_value(&mut self, value: &BasicValue) -> bool {
        value.get_type().is_some_and(|ty| {
            ty.get_sort() == TypeSort::Array || *ty == BasicInterpreter::NULL_TYPE
        })
    }

    fn get_element_value(
        &mut self,
        object_array_value: &BasicValue,
    ) -> Result<BasicValue, AnalyzerError> {
        if let Some(array_type) = object_array_value.get_type() {
            if array_type.get_sort() == TypeSort::Array {
                let descriptor = array_type.get_descriptor();
                let element = get_type(descriptor.get(1..descriptor.len()).unwrap_or_default())?;
                return self
                    .new_value(Some(&element))?
                    .ok_or_else(|| null_pointer("value"));
            } else if *array_type == BasicInterpreter::NULL_TYPE {
                return Ok(*object_array_value);
            }
        }
        Err(assertion_error())
    }

    fn is_sub_type_of(
        &mut self,
        value: &BasicValue,
        expected: &BasicValue,
    ) -> Result<bool, AnalyzerError> {
        let (Some(ty), Some(expected_type)) = (value.get_type(), expected.get_type()) else {
            // Null types correspond to BasicValue.UNINITIALIZED_VALUE.
            return Ok(value.get_type().is_none() && expected.get_type().is_none());
        };
        if ty == expected_type {
            return Ok(true);
        }
        match expected_type.get_sort() {
            TypeSort::Int | TypeSort::Float | TypeSort::Long | TypeSort::Double => Ok(false),
            TypeSort::Array | TypeSort::Object => {
                if *ty == BasicInterpreter::NULL_TYPE {
                    return Ok(true);
                }
                // Convert 'type' to its element type and array dimension. Arrays of primitive
                // values are seen as Object arrays with one dimension less. Hence the element
                // type is always of Type.OBJECT sort.
                let (mut ty, dim) = element_and_dimensions(ty)?;
                // Do the same for expectedType.
                let mut expected_dim = 0;
                let mut expected_type = expected_type.clone();
                if expected_type.get_sort() == TypeSort::Array {
                    expected_dim = expected_type.get_dimensions();
                    expected_type = element_type(&expected_type)?;
                    if expected_type.get_sort() != TypeSort::Object {
                        // If the expected type is an array of some primitive type, it does not
                        // have any subtype other than itself. And 'type' is different by
                        // hypothesis.
                        return Ok(false);
                    }
                }
                // A type with less dimensions than expected can't be a subtype of the expected
                // type.
                if dim < expected_dim {
                    return Ok(false);
                }
                // A type with more dimensions than expected is seen as an array with the
                // expected dimensions but with an Object element type. For instance an array of
                // arrays of Integer is seen as an array of Object if the expected type is an
                // array of Serializable.
                if dim > expected_dim {
                    ty = OBJECT_TYPE;
                }
                // type and expectedType have a Type.OBJECT sort by construction (see above), as
                // expected by isAssignableFrom.
                if self.is_assignable_from(&expected_type, &ty)? {
                    return Ok(true);
                }
                // ASM: getClass(expectedType).isInterface(). The merge of class or interface
                // types can only yield class types (because it is not possible in general to
                // find an unambiguous common super interface, due to multiple inheritance).
                // Because of this limitation, we need to relax the subtyping check here if
                // 'value' is an interface.
                if self.class_is_interface(&expected_type)? {
                    // ASM: Object.class.isAssignableFrom(getClass(type)), which is true once
                    // the class is found.
                    self.class_is_interface(&ty)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => Err(assertion_error()),
        }
    }
}

impl<H: ClassHierarchy> SimpleVerifier<H> {
    /// ASM's `getClass(type).isInterface()`, which loads the class even if it is the class
    /// being verified.
    fn class_is_interface(&self, ty: &Type) -> Result<bool, AnalyzerError> {
        self.hierarchy
            .is_interface(ty.get_internal_name())
            .map_err(hierarchy_error)
    }
}

impl<H: ClassHierarchy> Interpreter for SimpleVerifier<H> {
    type V = BasicValue;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<BasicValue>, AnalyzerError> {
        SimpleVerifier::new_value(self, ty)
    }

    fn new_operation(&mut self, insn: InsnRef<'_>) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicInterpreter::new_operation(self, insn)
    }

    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<BasicValue, AnalyzerError> {
        BasicVerifier::copy_operation(insn, value)
    }

    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::unary_operation(self, insn, value)
    }

    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::binary_operation(self, insn, value1, value2)
    }

    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &BasicValue,
        value2: &BasicValue,
        value3: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::ternary_operation(self, insn, value1, value2, value3)
    }

    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[BasicValue],
        return_type: Option<&Type<'_>>,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        BasicVerifier::nary_operation(self, insn, values, return_type)
    }

    fn return_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &BasicValue,
        expected: Option<&BasicValue>,
    ) -> Result<(), AnalyzerError> {
        BasicVerifier::return_operation(self, insn, value, expected)
    }

    fn merge(
        &mut self,
        value1: &BasicValue,
        value2: &BasicValue,
    ) -> Result<Option<BasicValue>, AnalyzerError> {
        SimpleVerifier::merge(self, value1, value2)
    }
}

/// Whether a type is an object or array type.
fn is_reference_type(ty: &Type) -> bool {
    matches!(ty.get_sort(), TypeSort::Object | TypeSort::Array)
}

/// The element type of an array type, whose malformed descriptor is a runtime error.
fn element_type<'a>(ty: &Type<'a>) -> Result<Type<'a>, AnalyzerError> {
    ty.try_get_element_type()
        .map_err(|e| AnalyzerError::runtime(e.message()))
}

/// A reference type as an object type and a number of dimensions: an array of primitive values
/// is seen as an `Object` array with one dimension less.
fn element_and_dimensions<'a>(ty: &Type<'a>) -> Result<(Type<'a>, usize), AnalyzerError> {
    if ty.get_sort() != TypeSort::Array {
        return Ok((ty.clone(), 0));
    }
    let dimensions = ty.get_dimensions();
    let element = element_type(ty)?;
    if element.get_sort() != TypeSort::Object {
        Ok((OBJECT_TYPE, dimensions - 1))
    } else {
        Ok((element, dimensions))
    }
}

/// A class hierarchy error, as the runtime exception ASM's class loading throws.
fn hierarchy_error(error: HierarchyError) -> AnalyzerError {
    let mut message = JavaString::new();
    match &error {
        HierarchyError::TypeNotPresent(name) => {
            message.push_str("Type ");
            message.push_java_str(name);
            message.push_str(" not present");
        }
        HierarchyError::BrokenSuperClassChain(_) | HierarchyError::NoHierarchy(..) => {
            message.push_str(&error.to_string());
        }
    }
    AnalyzerError::runtime(message)
}

/// The `NullPointerException` of a `null` reference that ASM dereferences.
fn null_pointer(variable: &str) -> AnalyzerError {
    AnalyzerError::runtime(format!(
        "java.lang.NullPointerException: {variable} is null"
    ))
}
