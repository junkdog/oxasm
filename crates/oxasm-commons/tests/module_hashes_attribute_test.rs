//! Port of ASM's `ModuleHashesAttributeTest`.

use oxasm::jstr;
use oxasm::opcodes::V1_8;
use oxasm::{Attribute, ClassReader, ClassVisitor, ClassWriter, ClassWriterFlags, ParsingOptions};
use oxasm_commons::ModuleHashesAttribute;

const HASH1: [u8; 3] = [0x1, 0x2, 0x3];
const HASH2: [u8; 3] = [0x4, 0x5, 0x6];

/// Keeps the ModuleHashes attribute it visits.
#[derive(Default)]
struct Collect(Option<ModuleHashesAttribute>);

impl ClassVisitor for Collect {
    fn visit_attribute(&mut self, attribute: &Attribute) {
        if let Attribute::Custom(custom) = attribute
            && let Some(attribute) = custom.downcast_ref::<ModuleHashesAttribute>()
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
    class_writer.visit_attribute(&Attribute::Custom(Box::new(ModuleHashesAttribute::new(
        "algorithm",
        vec!["module1".into(), "module2".into()],
        vec![HASH1.to_vec(), HASH2.to_vec()],
    ))));
    let class_file = class_writer.to_bytes().unwrap();
    let mut collect = Collect::default();
    ClassReader::new(&class_file)
        .unwrap()
        .accept_with_codecs(
            &mut collect,
            &[&ModuleHashesAttribute::default()],
            ParsingOptions::empty(),
        )
        .unwrap();
    let module_hashes_attribute = collect.0.expect("a ModuleHashes attribute");
    assert_eq!(module_hashes_attribute.algorithm, "algorithm");
    assert_eq!(module_hashes_attribute.modules.len(), 2);
    assert_eq!(module_hashes_attribute.modules[0], "module1");
    assert_eq!(module_hashes_attribute.modules[1], "module2");
    assert_eq!(module_hashes_attribute.hashes.len(), 2);
    assert_eq!(module_hashes_attribute.hashes[0], HASH1);
    assert_eq!(module_hashes_attribute.hashes[1], HASH2);
}
