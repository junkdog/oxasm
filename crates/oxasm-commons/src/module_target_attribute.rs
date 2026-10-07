//! [`ModuleTargetAttribute`]: ASM's `ModuleTargetAttribute`.

use oxasm::error::ReadError;
use oxasm::reader::AttributeReadContext;
use oxasm::writer::AttributeWriteContext;
use oxasm::{AttributeCodec, CustomAttribute, JavaStr, JavaString, jstr};

/// A ModuleTarget attribute. This attribute is specific to the OpenJDK and may change in the
/// future.
///
/// It is also its own codec (ASM's attribute "prototype"): pass
/// `&ModuleTargetAttribute::default()` to `ClassReader::accept_with_codecs` to decode these
/// attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleTargetAttribute {
    /// The name of the platform on which the module can run.
    pub platform: Option<JavaString>,
}

impl ModuleTargetAttribute {
    /// Constructs a new ModuleTarget attribute.
    pub fn new(platform: Option<JavaString>) -> ModuleTargetAttribute {
        ModuleTargetAttribute { platform }
    }
}

impl CustomAttribute for ModuleTargetAttribute {
    fn name(&self) -> &JavaStr {
        jstr!("ModuleTarget")
    }

    fn write(&self, cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        let platform_index = self.platform.as_deref().map_or(0, |p| cx.new_utf8(p));
        platform_index.to_be_bytes().to_vec()
    }

    fn codec(&self) -> Option<std::sync::Arc<dyn AttributeCodec>> {
        Some(std::sync::Arc::new(ModuleTargetAttribute::default()))
    }
}

impl AttributeCodec for ModuleTargetAttribute {
    fn name(&self) -> &JavaStr {
        jstr!("ModuleTarget")
    }

    fn read(
        &self,
        cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        let platform = cx.reader().read_utf8(cx.offset())?.map(JavaStr::to_owned);
        Ok(Box::new(ModuleTargetAttribute::new(platform)))
    }
}
