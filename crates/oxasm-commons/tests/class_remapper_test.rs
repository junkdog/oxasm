//! Port of ASM's `ClassRemapperTest`.
//!
//! Dropped (ADR-09, no `api` versions): `testVisitLdcInsn_constantDynamic_deprecated` and
//! `testInvokeDynamicInsn_field_deprecated`, which test the deprecated two argument
//! `mapInvokeDynamicMethodName` that ASM only calls for remappers created with its deprecated
//! constructors (`api == 0`). The `Api` dimension of the parameterized tests is gone too.
//!
//! The JVM checks (`newInstance()`) of `testAllMethods_precompiledClass*` are the
//! `*_new_instance` twins, which need the JVM oracle.

mod common;

use std::borrow::Cow;

use oxasm::opcodes::*;
use oxasm::{
    AnnotationValue, Attribute, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, Constant,
    ConstantDynamic, Handle, HandleKind, JavaStr, JavaString, ParsingOptions, jstr,
};
use oxasm_commons::remapper::defaults;
use oxasm_commons::{ClassRemapper, ModuleHashesAttribute, Remapper, SimpleRemapper};
use oxasm_testkit::PrecompiledClass;
use oxasm_tree::{ClassNode, Insn, SymConstant};
use oxasm_util::check::CheckMethodAdapter;

use common::{CodeComment, assert_precompiled_new_instance, assert_same_class_file};

#[test]
fn test_visit() {
    let mut class_node = ClassNode::new();
    let mut class_remapper = ClassRemapper::new(
        &mut class_node,
        SimpleRemapper::with_single_mapping("pkg/C", "new/pkg/C"),
    );

    class_remapper.visit(
        V1_5,
        ACC_PUBLIC,
        jstr!("pkg/C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );

    drop(class_remapper);
    assert_eq!(class_node.resolve(class_node.name), "new/pkg/C");
}

#[test]
fn test_visit_annotation() {
    struct AnnotationAttributeRemapper;
    impl Remapper for AnnotationAttributeRemapper {
        fn map_annotation_attribute_name<'a>(
            &'a self,
            descriptor: &'a JavaStr,
            name: &'a JavaStr,
        ) -> Cow<'a, JavaStr> {
            if descriptor == "Lpkg/A;" {
                let mut new_name = JavaString::from("new.");
                new_name.push_java_str(name);
                return Cow::Owned(new_name);
            }
            Cow::Borrowed(name)
        }
    }
    let mut class_node = ClassNode::new();
    let mut remapper = ClassRemapper::new(&mut class_node, AnnotationAttributeRemapper);
    remapper.visit(
        V1_5,
        ACC_PUBLIC,
        jstr!("pkg/C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    let mut annotation_visitor = remapper.visit_annotation(jstr!("Lpkg/A;"), true).unwrap();
    annotation_visitor.visit(
        Some(jstr!("attribute")),
        &AnnotationValue::String(Cow::Borrowed(jstr!("value"))),
    );
    drop(annotation_visitor);
    drop(remapper);

    assert_eq!(
        class_node.visible_annotations[0].values[0]
            .0
            .map(|name| class_node.resolve(name)),
        Some(jstr!("new.attribute"))
    );
}

/// A remapper that maps the given internal names (ASM's anonymous `Remapper`s overriding `map`).
struct MapRemapper(&'static [(&'static str, &'static str)]);

impl Remapper for MapRemapper {
    fn map<'a>(&'a self, internal_name: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
        for (from, to) in self.0 {
            if internal_name == *from {
                return Some(Cow::Borrowed(jstr!(to)));
            }
        }
        Some(Cow::Borrowed(internal_name))
    }
}

fn remap_inner_class(
    mapping: &'static [(&'static str, &'static str)],
    name: &str,
    outer_name: &str,
    inner_name: &str,
) -> ClassNode {
    let mut class_node = ClassNode::new();
    let mut remapper = ClassRemapper::new(&mut class_node, MapRemapper(mapping));
    remapper.visit(
        V1_5,
        ACC_PUBLIC,
        jstr!("pkg/C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );

    remapper.visit_inner_class(
        jstr!(name),
        Some(jstr!(outer_name)),
        Some(jstr!(inner_name)),
        ACC_PUBLIC,
    );

    drop(remapper);
    class_node
}

#[test]
fn test_visit_inner_class() {
    let class_node = remap_inner_class(
        &[("pkg/C", "a"), ("pkg/C$Inner", "a$b")],
        "pkg/C$Inner",
        "pkg/C",
        "Inner",
    );

    let inner_class = &class_node.inner_classes[0];
    assert_eq!(class_node.resolve(inner_class.name), "a$b");
    assert_eq!(
        inner_class.outer_name.map(|s| class_node.resolve(s)),
        Some(jstr!("a"))
    );
    assert_eq!(
        inner_class.inner_name.map(|s| class_node.resolve(s)),
        Some(jstr!("b"))
    );
}

#[test]
fn test_visit_inner_class_local_inner_class() {
    let class_node = remap_inner_class(
        &[("pkg/C", "a"), ("pkg/C$1Inner", "a$1b")],
        "pkg/C$1Inner",
        "pkg/C",
        "Inner",
    );

    let inner_class = &class_node.inner_classes[0];
    assert_eq!(class_node.resolve(inner_class.name), "a$1b");
    assert_eq!(
        inner_class.outer_name.map(|s| class_node.resolve(s)),
        Some(jstr!("a"))
    );
    assert_eq!(
        inner_class.inner_name.map(|s| class_node.resolve(s)),
        Some(jstr!("b"))
    );
}

#[test]
fn test_visit_inner_class_special_remap() {
    let class_node = remap_inner_class(
        &[("pkg/C", "pkg2/C"), ("pkg/C$$a", "pkg2/C$$a")],
        "pkg/C$$a",
        "pkg/C",
        "$a",
    );

    let inner_class = &class_node.inner_classes[0];
    assert_eq!(class_node.resolve(inner_class.name), "pkg2/C$$a");
    assert_eq!(
        inner_class.outer_name.map(|s| class_node.resolve(s)),
        Some(jstr!("pkg2/C"))
    );
    assert_eq!(
        inner_class.inner_name.map(|s| class_node.resolve(s)),
        Some(jstr!("$a"))
    );
}

#[test]
fn test_visit_attribute_module_hashes() {
    struct ModuleNameRemapper;
    impl Remapper for ModuleNameRemapper {
        fn map_module_name<'a>(&'a self, name: &'a JavaStr) -> Cow<'a, JavaStr> {
            let mut new_name = JavaString::from("new.");
            new_name.push_java_str(name);
            Cow::Owned(new_name)
        }
    }
    let mut class_node = ClassNode::new();
    let mut class_remapper = ClassRemapper::new(&mut class_node, ModuleNameRemapper);
    class_remapper.visit(
        V1_5,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );

    class_remapper.visit_attribute(&Attribute::Custom(Box::new(ModuleHashesAttribute::new(
        "algorithm",
        vec![JavaString::from("pkg.C")],
        vec![Vec::new()],
    ))));

    drop(class_remapper);
    assert_eq!(class_node.resolve(class_node.name), "C");
    let Attribute::Custom(attribute) = &class_node.attrs[0] else {
        panic!("a custom attribute");
    };
    let module_hashes_attribute = attribute.downcast_ref::<ModuleHashesAttribute>().unwrap();
    assert_eq!(module_hashes_attribute.modules[0], "new.pkg.C");
}

/// The remapper of `testVisitLdcInsn_constantDynamic`.
struct ConstantDynamicRemapper;

impl Remapper for ConstantDynamicRemapper {
    fn map_basic_invoke_dynamic_method_name<'a>(
        &'a self,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
        _bootstrap_method_handle: &'a Handle,
        _bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Cow<'a, JavaStr> {
        let mut new_name = JavaString::from("new.");
        new_name.push_java_str(name);
        Cow::Owned(new_name)
    }

    fn map<'a>(&'a self, internal_name: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
        if internal_name == "java/lang/String" {
            return Some(Cow::Borrowed(jstr!("java/lang/Integer")));
        }
        Some(Cow::Borrowed(internal_name))
    }
}

#[test]
fn test_visit_ldc_insn_constant_dynamic() {
    let mut class_node = ClassNode::new();
    let mut class_remapper = ClassRemapper::new(&mut class_node, ConstantDynamicRemapper);
    class_remapper.visit(
        V11,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    let mut method_visitor = class_remapper
        .visit_method(ACC_PUBLIC, jstr!("hello"), jstr!("()V"), None, &[])
        .unwrap();
    method_visitor.visit_code();

    method_visitor.visit_ldc_insn(&Constant::Dynamic(Box::new(ConstantDynamic::new(
        "foo",
        "Ljava/lang/String;",
        Handle::new(
            HandleKind::InvokeStatic,
            "BSMHost",
            "bsm",
            "()Ljava/lang/String;",
            false,
        ),
        Vec::new(),
    ))));
    drop(method_visitor);
    drop(class_remapper);

    let Insn::Ldc(SymConstant::Dynamic(constant_dynamic)) =
        &class_node.methods[0].instructions.get(0).unwrap().insn
    else {
        panic!("an LDC of a constant dynamic");
    };
    assert_eq!(class_node.resolve(constant_dynamic.name), "new.foo");
    assert_eq!(
        class_node.resolve(constant_dynamic.desc),
        "Ljava/lang/Integer;"
    );
    assert_eq!(
        class_node.resolve(constant_dynamic.bsm.desc),
        "()Ljava/lang/Integer;"
    );
}

#[test]
fn test_invoke_dynamic_insn_field() {
    struct FieldHandleRemapper;
    impl Remapper for FieldHandleRemapper {
        fn map_basic_invoke_dynamic_method_name<'a>(
            &'a self,
            name: &'a JavaStr,
            _descriptor: &'a JavaStr,
            _bootstrap_method_handle: &'a Handle,
            _bootstrap_method_arguments: &'a [Constant<'_>],
        ) -> Cow<'a, JavaStr> {
            let mut new_name = JavaString::from("new.");
            new_name.push_java_str(name);
            Cow::Owned(new_name)
        }

        fn map_field_name<'a>(
            &'a self,
            _owner: &'a JavaStr,
            name: &'a JavaStr,
            _descriptor: &'a JavaStr,
        ) -> Cow<'a, JavaStr> {
            if name == "a" {
                return Cow::Borrowed(jstr!("demo"));
            }
            Cow::Borrowed(name)
        }
    }
    let mut class_node = ClassNode::new();
    let mut class_remapper = ClassRemapper::new(&mut class_node, FieldHandleRemapper);
    class_remapper.visit(
        V11,
        ACC_PUBLIC,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    let mut method_visitor = class_remapper
        .visit_method(ACC_PUBLIC, jstr!("hello"), jstr!("()V"), None, &[])
        .unwrap();
    method_visitor.visit_code();

    method_visitor.visit_invoke_dynamic_insn(
        jstr!("foo"),
        jstr!("()Ljava/lang/String;"),
        &Handle::new(
            HandleKind::GetField,
            "pkg/B",
            "a",
            "Ljava/lang/String;",
            false,
        ),
        &[],
    );
    drop(method_visitor);
    drop(class_remapper);

    let Insn::InvokeDynamic(invoke_dynamic) =
        &class_node.methods[0].instructions.get(0).unwrap().insn
    else {
        panic!("an invokedynamic instruction");
    };
    assert_eq!(class_node.resolve(invoke_dynamic.name), "new.foo");
    assert_eq!(class_node.resolve(invoke_dynamic.bsm.name), "demo");
}

/// Tests that classes transformed with an empty ClassRemapper are unchanged.
#[test]
fn test_empty_class_remapper_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let class_file = class_parameter.bytes();
        let class_reader = ClassReader::new(class_file).unwrap();
        let mut class_remapper = ClassRemapper::new(
            ClassWriter::new(ClassWriterFlags::empty()),
            SimpleRemapper::default(),
        );

        class_reader
            .accept_with_codecs(
                &mut class_remapper,
                &[&CodeComment],
                ParsingOptions::empty(),
            )
            .unwrap();

        let new_class_file = class_remapper.into_class_visitor().to_bytes().unwrap();
        assert_same_class_file(class_file, &new_class_file, &class_parameter);
    }
}

/// Tests that inner class names are unchanged with by an empty ClassRemapper.
#[test]
fn test_empty_class_remapper_inner_class_names() {
    let mut class_file_writer = ClassWriter::new(ClassWriterFlags::empty());
    class_file_writer.visit(
        V1_8,
        ACC_ABSTRACT | ACC_INTERFACE,
        jstr!("Outer"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    class_file_writer.visit_inner_class(
        jstr!("Outer$$Inner"),
        Some(jstr!("Outer")),
        Some(jstr!("$Inner")),
        ACC_PUBLIC | ACC_STATIC | ACC_ABSTRACT | ACC_INTERFACE,
    );
    class_file_writer.visit_end();
    let class_file = class_file_writer.to_bytes().unwrap();
    let class_reader = ClassReader::new(&class_file).unwrap();
    let mut class_remapper = ClassRemapper::new(
        ClassWriter::new(ClassWriterFlags::empty()),
        SimpleRemapper::default(),
    );

    class_reader
        .accept_with_codecs(
            &mut class_remapper,
            &[&CodeComment],
            ParsingOptions::empty(),
        )
        .unwrap();

    let new_class_file = class_remapper.into_class_visitor().to_bytes().unwrap();
    assert_same_class_file(&class_file, &new_class_file, &"Outer");
}

/// The class of `class_parameter`, remapped with an [`UpperCaseRemapper`] by a class remapper
/// fed by a class reader (ASM's `testAllMethods_precompiledClass`, less the JVM check).
fn remap_precompiled_class(class_parameter: PrecompiledClass) -> Vec<u8> {
    let class_reader = ClassReader::new(class_parameter.bytes()).unwrap();
    let upper_case_remapper = UpperCaseRemapper::new(&class_parameter.internal_name());
    let mut class_remapper = ClassRemapper::new(
        ClassWriter::new(ClassWriterFlags::empty()),
        upper_case_remapper,
    );

    class_reader
        .accept(&mut class_remapper, ParsingOptions::empty())
        .unwrap();

    class_remapper.into_class_visitor().to_bytes().unwrap()
}

/// The class of `class_parameter`, remapped with an [`UpperCaseRemapper`] by a class remapper
/// fed by a class node (ASM's `testAllMethods_precompiledClass_fromClassNode`, less the JVM
/// check).
fn remap_precompiled_class_from_class_node(class_parameter: PrecompiledClass) -> Vec<u8> {
    let mut class_node = ClassNode::new();
    ClassReader::new(class_parameter.bytes())
        .unwrap()
        .accept(&mut class_node, ParsingOptions::empty())
        .unwrap();
    let upper_case_remapper = UpperCaseRemapper::new(&class_parameter.internal_name());
    let mut class_remapper = ClassRemapper::new(
        ClassWriter::new(ClassWriterFlags::empty()),
        upper_case_remapper,
    );

    class_node.accept(&mut class_remapper);

    class_remapper.into_class_visitor().to_bytes().unwrap()
}

/// Tests that classes can be transformed with a ClassRemapper (the checks of the
/// `UpperCaseRemapper` pass); the JVM check is
/// [`test_all_methods_precompiled_class_new_instance`].
#[test]
fn test_all_methods_precompiled_class() {
    for &class_parameter in PrecompiledClass::ALL {
        let new_class_file = remap_precompiled_class(class_parameter);
        let new_class_reader = ClassReader::new(&new_class_file).unwrap();
        let expected_name =
            UpperCaseRemapper::new(&class_parameter.internal_name()).remapped_internal_class_name;
        assert_eq!(new_class_reader.get_class_name().unwrap(), &*expected_name);
    }
}

/// Tests that classes transformed with a ClassRemapper can be loaded and instantiated.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_precompiled_class_new_instance() {
    for &class_parameter in PrecompiledClass::ALL {
        let new_class_file = remap_precompiled_class(class_parameter);
        assert_precompiled_new_instance(class_parameter, &new_class_file);
    }
}

/// Tests that classes can be transformed with a ClassNode and ClassRemapper (the checks of the
/// `UpperCaseRemapper` pass); the JVM check is
/// [`test_all_methods_precompiled_class_from_class_node_new_instance`].
#[test]
fn test_all_methods_precompiled_class_from_class_node() {
    for &class_parameter in PrecompiledClass::ALL {
        let new_class_file = remap_precompiled_class_from_class_node(class_parameter);
        // The class node path writes the same class as the reader path.
        assert_same_class_file(
            &remap_precompiled_class(class_parameter),
            &new_class_file,
            &class_parameter,
        );
    }
}

/// Tests that classes transformed with a ClassNode and ClassRemapper can be loaded and
/// instantiated.
#[test]
#[ignore = "needs the JVM oracle"]
fn test_all_methods_precompiled_class_from_class_node_new_instance() {
    for &class_parameter in PrecompiledClass::ALL {
        let new_class_file = remap_precompiled_class_from_class_node(class_parameter);
        assert_precompiled_new_instance(class_parameter, &new_class_file);
    }
}

/// ASM's `checkDescriptor`: panics if `descriptor` is not a valid field descriptor.
fn check_descriptor(descriptor: &JavaStr) {
    let mut check_method_adapter = CheckMethodAdapter::new(None);
    check_method_adapter.visit_code();
    check_method_adapter.visit_field_insn(GETFIELD, jstr!("Owner"), jstr!("name"), descriptor);
}

/// ASM's `checkInternalName`: panics if `internal_name` is not a valid internal name.
fn check_internal_name(internal_name: &JavaStr) {
    let mut check_method_adapter = CheckMethodAdapter::new(None);
    check_method_adapter.version = V1_5;
    check_method_adapter.visit_code();
    check_method_adapter.visit_field_insn(GETFIELD, internal_name, jstr!("name"), jstr!("I"));
}

use oxasm::MethodVisitor;

/// Java's `toUpperCase(Locale.ENGLISH)`, on the code points that are characters (unpaired
/// surrogates are kept).
fn to_upper_case(s: &JavaStr) -> JavaString {
    let mut upper_case = JavaString::with_capacity(s.len());
    for c in s.chars() {
        match c.as_char() {
            Some(c) => c.to_uppercase().for_each(|c| upper_case.push(c)),
            None => upper_case.push_java(c),
        }
    }
    upper_case
}

/// ASM's `UpperCaseRemapper`: remaps the class, its fields and methods (except constructors) and
/// the invokedynamic names to upper case, and checks the descriptors and internal names it maps.
struct UpperCaseRemapper {
    internal_class_name: JavaString,
    remapped_internal_class_name: JavaString,
}

impl UpperCaseRemapper {
    fn new(internal_class_name: &str) -> UpperCaseRemapper {
        let internal_class_name = JavaString::from(internal_class_name);
        let remapped_internal_class_name = if internal_class_name == "module-info" {
            internal_class_name.clone()
        } else {
            to_upper_case(&internal_class_name)
        };
        UpperCaseRemapper {
            internal_class_name,
            remapped_internal_class_name,
        }
    }
}

impl Remapper for UpperCaseRemapper {
    fn map_desc<'a>(&'a self, descriptor: &'a JavaStr) -> Cow<'a, JavaStr> {
        check_descriptor(descriptor);
        defaults::map_desc(self, descriptor)
    }

    fn map_type<'a>(&'a self, type_: &'a JavaStr) -> Cow<'a, JavaStr> {
        if type_ != "module-info" {
            check_internal_name(type_);
        }
        defaults::map_type(self, type_)
    }

    fn map_method_name<'a>(
        &'a self,
        owner: &'a JavaStr,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        if name == "<init>" || name == "<clinit>" {
            return Cow::Borrowed(name);
        }
        if owner == &*self.internal_class_name {
            Cow::Owned(to_upper_case(name))
        } else {
            Cow::Borrowed(name)
        }
    }

    fn map_basic_invoke_dynamic_method_name<'a>(
        &'a self,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
        _bootstrap_method_handle: &'a Handle,
        _bootstrap_method_arguments: &'a [Constant<'_>],
    ) -> Cow<'a, JavaStr> {
        Cow::Owned(to_upper_case(name))
    }

    fn map_field_name<'a>(
        &'a self,
        owner: &'a JavaStr,
        name: &'a JavaStr,
        _descriptor: &'a JavaStr,
    ) -> Cow<'a, JavaStr> {
        if owner == &*self.internal_class_name {
            Cow::Owned(to_upper_case(name))
        } else {
            Cow::Borrowed(name)
        }
    }

    fn map<'a>(&'a self, type_name: &'a JavaStr) -> Option<Cow<'a, JavaStr>> {
        Some(if type_name == &*self.internal_class_name {
            Cow::Borrowed(&self.remapped_internal_class_name)
        } else {
            Cow::Borrowed(type_name)
        })
    }

    // ASM's override of mapValue checks that the value has a supported type, which `Constant`
    // guarantees.
}
