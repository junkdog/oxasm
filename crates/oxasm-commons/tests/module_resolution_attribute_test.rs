//! Port of ASM's `ModuleResolutionAttributeTest`.

use oxasm::jstr;
use oxasm::opcodes::V1_8;
use oxasm::{Attribute, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, ParsingOptions};
use oxasm_commons::ModuleResolutionAttribute;

/// Keeps the ModuleResolution attribute it visits.
#[derive(Default)]
struct Collect(Option<ModuleResolutionAttribute>);

impl ClassVisitor for Collect {
    fn visit_attribute(&mut self, attribute: &Attribute) {
        if let Attribute::Custom(custom) = attribute
            && let Some(attribute) = custom.downcast_ref::<ModuleResolutionAttribute>()
        {
            self.0 = Some(*attribute);
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
    class_writer.visit_attribute(&Attribute::Custom(Box::new(
        ModuleResolutionAttribute::new(123),
    )));
    let class_file = class_writer.to_bytes().unwrap();
    let mut collect = Collect::default();
    ClassReader::new(&class_file)
        .unwrap()
        .accept_with_codecs(
            &mut collect,
            &[&ModuleResolutionAttribute::default()],
            ParsingOptions::empty(),
        )
        .unwrap();
    assert_eq!(
        collect.0.expect("a ModuleResolution attribute").resolution,
        123
    );
}
