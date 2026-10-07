//! Port of ASM's `CheckModuleAdapterTest`.
//!
//! ASM's tests set the package-private `classVersion` field of the adapter; here the adapter
//! gets it from a `CheckClassAdapter`, as in ASM's own use (`with_class_version`).

use oxasm::opcodes::*;
use oxasm::{ClassVisitor, ModuleVisitor, jstr};
use oxasm_testkit::{panic_message, panics};
use oxasm_util::check::{CheckClassAdapter, CheckModuleAdapter};
use oxasm_util::trace::NoDelegate;

/// Runs `f` with the module visitor of a `CheckClassAdapter` for a class of the given version
/// (a `CheckModuleAdapter` whose `classVersion` is `version`).
fn with_class_version<R>(version: u32, f: impl FnOnce(&mut dyn ModuleVisitor) -> R) -> R {
    let mut check_class_adapter = CheckClassAdapter::new(NoDelegate);
    check_class_adapter.visit(version, ACC_MODULE, jstr!("module-info"), None, None, &[]);
    let mut module_visitor = check_class_adapter
        .visit_module(jstr!("module"), 0, None)
        .expect("a check adapter");
    f(&mut *module_visitor)
}

#[test]
fn test_constructor() {
    assert!(!panics(|| CheckModuleAdapter::new(None, false)));
    // The subclass half is dropped: there is no subclassing (ADR-13).
}

#[test]
fn test_visit_require_java_base_transitive() {
    let message = panic_message(|| {
        with_class_version(V10, |mv| {
            mv.visit_require(jstr!("java.base"), ACC_TRANSITIVE, None)
        });
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid access flags: 32 java.base can not be \
         declared ACC_TRANSITIVE or ACC_STATIC_PHASE"
    );
}

#[test]
fn test_visit_require_java_base_static_phase() {
    let message = panic_message(|| {
        with_class_version(V10, |mv| {
            mv.visit_require(jstr!("java.base"), ACC_STATIC_PHASE, None);
        });
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid access flags: 64 java.base can not be \
         declared ACC_TRANSITIVE or ACC_STATIC_PHASE"
    );
}

#[test]
fn test_visit_require_java_base_transitive_and_static_phase() {
    let message = panic_message(|| {
        with_class_version(V10, |mv| {
            mv.visit_require(jstr!("java.base"), ACC_TRANSITIVE | ACC_STATIC_PHASE, None);
        });
    });

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid access flags: 96 java.base can not be \
         declared ACC_TRANSITIVE or ACC_STATIC_PHASE"
    );
}

#[test]
fn test_visit_require_java_base_transitive_or_static_phase_are_ignored_under_jvms9() {
    assert!(!panics(|| {
        with_class_version(V9, |mv| {
            mv.visit_require(jstr!("java.base"), ACC_TRANSITIVE | ACC_STATIC_PHASE, None);
        });
    }));
}

// The `null` arrays of the next tests are empty slices (ADR-02).

#[test]
fn test_visit_export_null_array() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, false);

    assert!(!panics(|| check_module_adapter.visit_export(
        jstr!("package"),
        0,
        &[]
    )));
}

#[test]
fn test_visit_open_null_array() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, false);

    assert!(!panics(|| check_module_adapter.visit_open(
        jstr!("package"),
        0,
        &[]
    )));
}

#[test]
fn test_visit_open_open_module() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, true);

    let message = panic_message(|| check_module_adapter.visit_open(jstr!("package"), 0, &[]));

    assert_eq!(
        message,
        "java.lang.UnsupportedOperationException: An open module can not use open directive"
    );
}

#[test]
fn test_visit_use_name_already_declared() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, false);
    check_module_adapter.visit_use(jstr!("service"));

    let message = panic_message(|| check_module_adapter.visit_use(jstr!("service")));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Module uses 'service' already declared"
    );
}

#[test]
fn test_visit_use_after_end() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, false);
    check_module_adapter.visit_end();

    let message = panic_message(|| check_module_adapter.visit_use(jstr!("service")));

    assert_eq!(
        message,
        "java.lang.IllegalStateException: Cannot call a visit method after visitEnd has been called"
    );
}

#[test]
fn test_visit_provide_null_provider_list() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, false);

    let message = panic_message(|| check_module_adapter.visit_provide(jstr!("service2"), &[]));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Providers cannot be null or empty"
    );
}

#[test]
fn test_visit_provide_empty_provider_list() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, false);

    let message = panic_message(|| check_module_adapter.visit_provide(jstr!("service1"), &[]));

    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Providers cannot be null or empty"
    );
}

// -------------------------------------------------------------------------------------------------
// oxasm-specific
// -------------------------------------------------------------------------------------------------

#[test]
fn names_are_checked() {
    let mut check_module_adapter = CheckModuleAdapter::new(None, false);
    check_module_adapter.visit_require(jstr!("java.base"), ACC_MANDATED, None);
    check_module_adapter.visit_export(jstr!("pkg/a"), 0, &[jstr!("other.module")]);

    let message = panic_message(|| check_module_adapter.visit_require(jstr!("java.base"), 0, None));
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Modules requires 'java.base' already declared"
    );
    let message = panic_message(|| check_module_adapter.visit_require(jstr!("a..b"), 0, None));
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid required module (must be a fully qualified \
         name): a..b"
    );
    let message = panic_message(|| {
        check_module_adapter.visit_export(jstr!("pkg/b"), 0, &[jstr!("x;y")]);
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid module export to (must be a fully \
         qualified name): x;y"
    );
    let message = panic_message(|| {
        check_module_adapter.visit_provide(jstr!("svc"), &[jstr!("")]);
    });
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid provider (must not be null or empty)"
    );
    let message =
        panic_message(|| check_module_adapter.visit_export(jstr!("pkg/c"), ACC_FINAL, &[]));
    assert_eq!(
        message,
        "java.lang.IllegalArgumentException: Invalid access flags: 16"
    );
}
