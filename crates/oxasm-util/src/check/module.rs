//! [`CheckModuleAdapter`]: ASM's `CheckModuleAdapter`.

use std::collections::HashSet;

use oxasm::opcodes::*;
use oxasm::{JavaStr, JavaString, ModuleVisitor};

use crate::check::support::{
    check_access, check_fully_qualified_name, check_internal_name, illegal_argument, illegal_state,
    unsupported_operation,
};

/// A [`ModuleVisitor`] that checks that its methods are properly used, and forwards them to an
/// optional downstream visitor.
///
/// # Panics
///
/// Its methods panic where ASM's throw (see the [module docs](crate::check)).
pub struct CheckModuleAdapter<'a> {
    mv: Option<Box<dyn ModuleVisitor + 'a>>,
    /// Whether the visited module is open.
    is_open: bool,
    /// The fully qualified names of the dependencies of the visited module.
    required_modules: NameSet,
    /// The internal names of the packages exported by the visited module.
    exported_packages: NameSet,
    /// The internal names of the packages opened by the visited module.
    opened_packages: NameSet,
    /// The internal names of the services used by the visited module.
    used_services: NameSet,
    /// The internal names of the services provided by the visited module.
    provided_services: NameSet,
    /// The class version number (set by a `CheckClassAdapter`; 0 otherwise).
    pub(crate) class_version: u32,
    /// Whether `visit_end` has been called.
    visit_end_called: bool,
}

impl<'a> CheckModuleAdapter<'a> {
    /// Constructs a new adapter that forwards the events to `module_visitor`. `is_open` is
    /// whether the visited module is open (has the `ACC_OPEN` flag in
    /// [`ClassVisitor::visit_module`](oxasm::ClassVisitor::visit_module)).
    pub fn new(module_visitor: Option<Box<dyn ModuleVisitor + 'a>>, is_open: bool) -> Self {
        CheckModuleAdapter {
            mv: module_visitor,
            is_open,
            required_modules: NameSet::new("Modules requires"),
            exported_packages: NameSet::new("Module exports"),
            opened_packages: NameSet::new("Module opens"),
            used_services: NameSet::new("Module uses"),
            provided_services: NameSet::new("Module provides"),
            class_version: 0,
            visit_end_called: false,
        }
    }

    fn check_visit_end_not_called(&self) {
        if self.visit_end_called {
            illegal_state("Cannot call a visit method after visitEnd has been called");
        }
    }
}

impl std::fmt::Debug for CheckModuleAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckModuleAdapter")
            .field("is_open", &self.is_open)
            .field("class_version", &self.class_version)
            .field("visit_end_called", &self.visit_end_called)
            .finish_non_exhaustive()
    }
}

impl ModuleVisitor for CheckModuleAdapter<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
        match &mut self.mv {
            Some(mv) => Some(&mut **mv),
            None => None,
        }
    }

    fn visit_main_class(&mut self, main_class: &JavaStr) {
        // Modules can only appear in V9 or more classes.
        check_internal_name(V9, main_class, "module main class");
        if let Some(mv) = &mut self.mv {
            mv.visit_main_class(main_class);
        }
    }

    fn visit_package(&mut self, packaze: &JavaStr) {
        check_internal_name(V9, packaze, "module package");
        if let Some(mv) = &mut self.mv {
            mv.visit_package(packaze);
        }
    }

    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        self.check_visit_end_not_called();
        check_fully_qualified_name(V9, module, "required module");
        self.required_modules
            .check_name_not_already_declared(module);
        check_access(
            access,
            ACC_STATIC_PHASE | ACC_TRANSITIVE | ACC_SYNTHETIC | ACC_MANDATED,
        );
        if self.class_version >= V10
            && module == "java.base"
            && access & (ACC_STATIC_PHASE | ACC_TRANSITIVE) != 0
        {
            illegal_argument(format_args!(
                "Invalid access flags: {} java.base can not be declared ACC_TRANSITIVE or \
                 ACC_STATIC_PHASE",
                access.cast_signed()
            ));
        }
        if let Some(mv) = &mut self.mv {
            mv.visit_require(module, access, version);
        }
    }

    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.check_visit_end_not_called();
        check_internal_name(V9, packaze, "package name");
        self.exported_packages
            .check_name_not_already_declared(packaze);
        check_access(access, ACC_SYNTHETIC | ACC_MANDATED);
        for module in modules {
            check_fully_qualified_name(V9, module, "module export to");
        }
        if let Some(mv) = &mut self.mv {
            mv.visit_export(packaze, access, modules);
        }
    }

    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.check_visit_end_not_called();
        if self.is_open {
            unsupported_operation("An open module can not use open directive");
        }
        check_internal_name(V9, packaze, "package name");
        self.opened_packages
            .check_name_not_already_declared(packaze);
        check_access(access, ACC_SYNTHETIC | ACC_MANDATED);
        for module in modules {
            check_fully_qualified_name(V9, module, "module open to");
        }
        if let Some(mv) = &mut self.mv {
            mv.visit_open(packaze, access, modules);
        }
    }

    fn visit_use(&mut self, service: &JavaStr) {
        self.check_visit_end_not_called();
        check_internal_name(V9, service, "service");
        self.used_services.check_name_not_already_declared(service);
        if let Some(mv) = &mut self.mv {
            mv.visit_use(service);
        }
    }

    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        self.check_visit_end_not_called();
        check_internal_name(V9, service, "service");
        self.provided_services
            .check_name_not_already_declared(service);
        if providers.is_empty() {
            illegal_argument("Providers cannot be null or empty");
        }
        for provider in providers {
            check_internal_name(V9, provider, "provider");
        }
        if let Some(mv) = &mut self.mv {
            mv.visit_provide(service, providers);
        }
    }

    fn visit_end(&mut self) {
        self.check_visit_end_not_called();
        self.visit_end_called = true;
        if let Some(mv) = &mut self.mv {
            mv.visit_end();
        }
    }
}

/// A set of names, of a given kind, that must be declared at most once.
struct NameSet {
    kind: &'static str,
    names: HashSet<JavaString>,
}

impl NameSet {
    fn new(kind: &'static str) -> NameSet {
        NameSet {
            kind,
            names: HashSet::new(),
        }
    }

    fn check_name_not_already_declared(&mut self, name: &JavaStr) {
        if !self.names.insert(name.to_owned()) {
            illegal_argument(format_args!("{} '{name}' already declared", self.kind));
        }
    }
}
