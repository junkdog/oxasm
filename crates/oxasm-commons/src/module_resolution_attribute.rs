//! [`ModuleResolutionAttribute`]: ASM's `ModuleResolutionAttribute`.

use oxasm::error::ReadError;
use oxasm::reader::AttributeReadContext;
use oxasm::writer::AttributeWriteContext;
use oxasm::{AttributeCodec, CustomAttribute, JavaStr, jstr};

/// A ModuleResolution attribute. This attribute is specific to the OpenJDK and may change in the
/// future.
///
/// It is also its own codec (ASM's attribute "prototype"): pass
/// `&ModuleResolutionAttribute::default()` to `ClassReader::accept_with_codecs` to decode these
/// attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleResolutionAttribute {
    /// The resolution state of a module: zero or more of
    /// [`Self::RESOLUTION_DO_NOT_RESOLVE_BY_DEFAULT`], [`Self::RESOLUTION_WARN_DEPRECATED`],
    /// [`Self::RESOLUTION_WARN_DEPRECATED_FOR_REMOVAL`] and
    /// [`Self::RESOLUTION_WARN_INCUBATING`].
    pub resolution: u16,
}

impl ModuleResolutionAttribute {
    /// The resolution state of a module meaning "do not resolve this module by default".
    pub const RESOLUTION_DO_NOT_RESOLVE_BY_DEFAULT: u16 = 1;
    /// The resolution state of a module meaning "warn if this module is resolved because it is
    /// deprecated".
    pub const RESOLUTION_WARN_DEPRECATED: u16 = 2;
    /// The resolution state of a module meaning "warn if this module is resolved because it is
    /// deprecated and will be removed".
    pub const RESOLUTION_WARN_DEPRECATED_FOR_REMOVAL: u16 = 4;
    /// The resolution state of a module meaning "warn if this module is resolved because it is
    /// incubating".
    pub const RESOLUTION_WARN_INCUBATING: u16 = 8;

    /// Constructs a new ModuleResolution attribute.
    pub fn new(resolution: u16) -> ModuleResolutionAttribute {
        ModuleResolutionAttribute { resolution }
    }
}

impl CustomAttribute for ModuleResolutionAttribute {
    fn name(&self) -> &JavaStr {
        jstr!("ModuleResolution")
    }

    fn write(&self, _cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        self.resolution.to_be_bytes().to_vec()
    }

    fn codec(&self) -> Option<std::sync::Arc<dyn AttributeCodec>> {
        Some(std::sync::Arc::new(ModuleResolutionAttribute::default()))
    }
}

impl AttributeCodec for ModuleResolutionAttribute {
    fn name(&self) -> &JavaStr {
        jstr!("ModuleResolution")
    }

    fn read(
        &self,
        cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        let resolution = cx.reader().read_unsigned_short(cx.offset())?;
        Ok(Box::new(ModuleResolutionAttribute::new(resolution)))
    }
}
