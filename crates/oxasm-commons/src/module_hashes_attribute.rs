//! [`ModuleHashesAttribute`]: ASM's `ModuleHashesAttribute`.

use oxasm::error::{ReadError, ReadErrorKind};
use oxasm::reader::AttributeReadContext;
use oxasm::writer::{AttributeWriteContext, ByteVector};
use oxasm::{AttributeCodec, CustomAttribute, JavaStr, JavaString, jstr};

/// A ModuleHashes attribute. This attribute is specific to the OpenJDK and may change in the
/// future.
///
/// It is also its own codec (ASM's attribute "prototype"): pass
/// `&ModuleHashesAttribute::default()` to `ClassReader::accept_with_codecs` to decode these
/// attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleHashesAttribute {
    /// The name of the hashing algorithm.
    pub algorithm: JavaString,
    /// A list of module names.
    pub modules: Vec<JavaString>,
    /// The hash of the modules in `modules`. The two lists must have the same size.
    pub hashes: Vec<Vec<u8>>,
}

impl ModuleHashesAttribute {
    /// Constructs a new ModuleHashes attribute.
    pub fn new(
        algorithm: impl Into<JavaString>,
        modules: Vec<JavaString>,
        hashes: Vec<Vec<u8>>,
    ) -> ModuleHashesAttribute {
        ModuleHashesAttribute {
            algorithm: algorithm.into(),
            modules,
            hashes,
        }
    }
}

/// A `null` string where ASM's reader would return one (constant pool index 0).
fn required(value: Option<&JavaStr>, offset: usize) -> Result<JavaString, ReadError> {
    value.map(JavaStr::to_owned).ok_or(ReadError {
        offset,
        kind: ReadErrorKind::InvalidConstantPoolIndex(0),
    })
}

impl CustomAttribute for ModuleHashesAttribute {
    fn name(&self) -> &JavaStr {
        jstr!("ModuleHashes")
    }

    /// # Panics
    ///
    /// If there are fewer hashes than modules (ASM's `IndexOutOfBoundsException`).
    fn write(&self, cx: &mut AttributeWriteContext<'_>) -> Vec<u8> {
        assert!(
            self.hashes.len() >= self.modules.len(),
            "Index {} out of bounds for length {}",
            self.hashes.len(),
            self.hashes.len()
        );
        let mut byte_vector = ByteVector::new();
        byte_vector.put_short(i32::from(cx.new_utf8(&self.algorithm)));
        byte_vector.put_short(i32::try_from(self.modules.len()).unwrap_or(i32::MAX));
        for (module, hash) in self.modules.iter().zip(&self.hashes) {
            byte_vector
                .put_short(i32::from(cx.new_module(module)))
                .put_short(i32::try_from(hash.len()).unwrap_or(i32::MAX))
                .put_byte_array(hash);
        }
        byte_vector.into_vec()
    }

    fn codec(&self) -> Option<std::sync::Arc<dyn AttributeCodec>> {
        Some(std::sync::Arc::new(ModuleHashesAttribute::default()))
    }
}

impl AttributeCodec for ModuleHashesAttribute {
    fn name(&self) -> &JavaStr {
        jstr!("ModuleHashes")
    }

    fn read(
        &self,
        cx: &mut AttributeReadContext<'_, '_>,
    ) -> Result<Box<dyn CustomAttribute>, ReadError> {
        let reader = cx.reader();
        let mut current_offset = cx.offset();
        let algorithm = required(reader.read_utf8(current_offset)?, current_offset)?;
        current_offset += 2;
        let num_modules = reader.read_unsigned_short(current_offset)?;
        current_offset += 2;
        let mut modules = Vec::new();
        let mut hashes = Vec::new();
        for _ in 0..num_modules {
            modules.push(required(
                reader.read_module(current_offset)?,
                current_offset,
            )?);
            current_offset += 2;
            let hash_length = usize::from(reader.read_unsigned_short(current_offset)?);
            current_offset += 2;
            hashes.push(reader.read_bytes(current_offset, hash_length)?.to_vec());
            current_offset += hash_length;
        }
        Ok(Box::new(ModuleHashesAttribute {
            algorithm,
            modules,
            hashes,
        }))
    }
}
