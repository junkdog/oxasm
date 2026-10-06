//! `ClassReader::new` + `accept` of a visitor that visits everything (and does nothing) never
//! panics, hangs or runs out of memory (oxasm-verification, "Fuzzing").

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxasm::{
    AnnotationVisitor, ClassReader, ClassVisitor, Constant, FieldVisitor, JavaStr, Label,
    MethodVisitor, ModuleVisitor, ParsingOptions, RecordComponentVisitor, TypePath, TypeReference,
};

/// Visits every child: annotations (nested ones included), modules, record components, fields
/// and methods, so that the reader parses everything.
struct Everything;

macro_rules! annotations {
    ($($method:ident($($arg:ty),* $(,)?)),* $(,)?) => {$(
        fn $method<'a>(&'a mut self, $(_: $arg),*) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            Some(Box::new(Everything))
        }
    )*};
}

impl AnnotationVisitor for Everything {
    annotations!(
        visit_annotation(Option<&JavaStr>, &JavaStr),
        visit_array(Option<&JavaStr>)
    );
}

impl ModuleVisitor for Everything {}

impl FieldVisitor for Everything {
    annotations!(
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
    );
}

impl RecordComponentVisitor for Everything {
    annotations!(
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
    );
}

impl MethodVisitor for Everything {
    annotations!(
        visit_annotation_default(),
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_parameter_annotation(u8, &JavaStr, bool),
        visit_insn_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_try_catch_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
        visit_local_variable_annotation(
            TypeReference,
            Option<&TypePath>,
            &[Label],
            &[Label],
            &[u16],
            &JavaStr,
            bool,
        ),
    );
}

impl ClassVisitor for Everything {
    annotations!(
        visit_annotation(&JavaStr, bool),
        visit_type_annotation(TypeReference, Option<&TypePath>, &JavaStr, bool),
    );

    fn visit_module<'a>(
        &'a mut self,
        _: &JavaStr,
        _: u32,
        _: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        Some(Box::new(Everything))
    }

    fn visit_record_component<'a>(
        &'a mut self,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        Some(Box::new(Everything))
    }

    fn visit_field<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        Some(Box::new(Everything))
    }

    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        _: &JavaStr,
        _: &JavaStr,
        _: Option<&JavaStr>,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        Some(Box::new(Everything))
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(reader) = ClassReader::new(data) else {
        return;
    };
    let _ = reader.get_access();
    let _ = reader.get_class_name();
    let _ = reader.get_super_name();
    let _ = reader.get_interfaces();
    for options in [ParsingOptions::empty(), ParsingOptions::EXPAND_FRAMES] {
        let _ = reader.accept(&mut Everything, options);
    }
});
