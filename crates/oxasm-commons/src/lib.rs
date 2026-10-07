//! Common transformations and adapters for [`oxasm`]: a Rust port of ASM's `asm-commons` module
//! (`org.objectweb.asm.commons`).
//!
//! [`oxasm`]: https://docs.rs/oxasm

mod advice_adapter;
mod analyzer_adapter;
pub mod annotation_remapper;
pub mod class_remapper;
mod code_size_evaluator;
pub mod field_remapper;
pub mod generator_adapter;
pub mod instruction_adapter;
mod jsr_inliner_adapter;
mod local_variables_sorter;
mod method;
pub mod method_remapper;
mod module_hashes_attribute;
pub mod module_remapper;
mod module_resolution_attribute;
mod module_target_attribute;
pub mod record_component_remapper;
pub mod remapper;
mod serial_version_uid_adder;
pub mod signature_remapper;
pub mod simple_remapper;
mod static_init_merger;
mod support;
mod try_catch_block_sorter;

pub use advice_adapter::{Advice, AdviceAdapter};
pub use analyzer_adapter::AnalyzerAdapter;
pub use annotation_remapper::AnnotationRemapper;
pub use class_remapper::ClassRemapper;
pub use code_size_evaluator::CodeSizeEvaluator;
pub use field_remapper::FieldRemapper;
pub use generator_adapter::{GeneratorAdapter, TableSwitchGenerator};
pub use instruction_adapter::{InstructionAdapter, InstructionDispatcher, InstructionVisitor};
pub use jsr_inliner_adapter::JsrInlinerAdapter;
pub use local_variables_sorter::{LocalVariablesSorter, LocalVariablesSorterHooks};
pub use method::Method;
pub use method_remapper::MethodRemapper;
pub use module_hashes_attribute::ModuleHashesAttribute;
pub use module_remapper::ModuleRemapper;
pub use module_resolution_attribute::ModuleResolutionAttribute;
pub use module_target_attribute::ModuleTargetAttribute;
pub use record_component_remapper::RecordComponentRemapper;
pub use remapper::Remapper;
pub use serial_version_uid_adder::{EncodedStringTooLong, SerialVersionUidAdder};
pub use signature_remapper::SignatureRemapper;
pub use simple_remapper::SimpleRemapper;
pub use static_init_merger::StaticInitMerger;
pub use try_catch_block_sorter::TryCatchBlockSorter;
