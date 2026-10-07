//! Common transformations and adapters for [`oxasm`]: a Rust port of ASM's `asm-commons` module
//! (`org.objectweb.asm.commons`).
//!
//! [`oxasm`]: https://docs.rs/oxasm

mod module_hashes_attribute;
mod module_resolution_attribute;
mod module_target_attribute;

pub use module_hashes_attribute::ModuleHashesAttribute;
pub use module_resolution_attribute::ModuleResolutionAttribute;
pub use module_target_attribute::ModuleTargetAttribute;
