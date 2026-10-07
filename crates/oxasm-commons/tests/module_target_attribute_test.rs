//! Port of ASM's `ModuleTargetAttributeTest`.

use oxasm::jstr;
use oxasm::opcodes::V1_8;
use oxasm::{Attribute, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, ParsingOptions};
use oxasm_commons::ModuleTargetAttribute;

/// Keeps the ModuleTarget attribute it visits.
#[derive(Default)]
struct Collect(Option<ModuleTargetAttribute>);

impl ClassVisitor for Collect {
    fn visit_attribute(&mut self, attribute: &Attribute) {
        if let Attribute::Custom(custom) = attribute
            && let Some(attribute) = custom.downcast_ref::<ModuleTargetAttribute>()
        {
            self.0 = Some(attribute.clone());
        }
    }
}

#[test]
fn test_write_and_read() {
    let mut class_writer = ClassWriter::new(ClassWriterFlags::empty());
    // ASM's test does not call visit, so this_class is 0, a null name that ASM's reader passes on
    // and oxasm's rejects (ADR-06).
    class_writer.visit(
        V1_8,
        0,
        jstr!("C"),
        None,
        Some(jstr!("java/lang/Object")),
        &[],
    );
    class_writer.visit_attribute(&Attribute::Custom(Box::new(ModuleTargetAttribute::new(
        Some("platform".into()),
    ))));
    let class_file = class_writer.to_bytes().unwrap();
    let mut collect = Collect::default();
    ClassReader::new(&class_file)
        .unwrap()
        .accept_with_codecs(
            &mut collect,
            &[&ModuleTargetAttribute::default()],
            ParsingOptions::empty(),
        )
        .unwrap();
    assert_eq!(
        collect
            .0
            .expect("a ModuleTarget attribute")
            .platform
            .as_deref(),
        Some(oxasm::jstr!("platform"))
    );
}
