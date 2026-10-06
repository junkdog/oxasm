//! [`TraceModuleVisitor`]: ASM's `TraceModuleVisitor`.

use oxasm::{JavaStr, ModuleVisitor};

use crate::printer::Printer;

/// A [`ModuleVisitor`] that prints the modules it visits with a [`Printer`].
pub struct TraceModuleVisitor<'a, P> {
    mv: Option<Box<dyn ModuleVisitor + 'a>>,
    /// The printer to convert the visited module into text.
    pub p: P,
}

impl<P: Printer> TraceModuleVisitor<'_, P> {
    /// Constructs a new visitor, without a downstream visitor.
    pub fn new(printer: P) -> Self {
        TraceModuleVisitor {
            mv: None,
            p: printer,
        }
    }
}

impl<'a, P: Printer> TraceModuleVisitor<'a, P> {
    /// Constructs a new visitor that also forwards the events to `module_visitor`.
    pub fn with_delegate(module_visitor: Option<Box<dyn ModuleVisitor + 'a>>, printer: P) -> Self {
        TraceModuleVisitor {
            mv: module_visitor,
            p: printer,
        }
    }
}

impl<P> std::fmt::Debug for TraceModuleVisitor<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceModuleVisitor").finish_non_exhaustive()
    }
}

impl<P: Printer> ModuleVisitor for TraceModuleVisitor<'_, P> {
    fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
        match &mut self.mv {
            Some(mv) => Some(&mut **mv),
            None => None,
        }
    }

    fn visit_main_class(&mut self, main_class: &JavaStr) {
        self.p.visit_main_class(main_class);
        if let Some(mv) = &mut self.mv {
            mv.visit_main_class(main_class);
        }
    }

    fn visit_package(&mut self, packaze: &JavaStr) {
        self.p.visit_package(packaze);
        if let Some(mv) = &mut self.mv {
            mv.visit_package(packaze);
        }
    }

    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        self.p.visit_require(module, access, version);
        if let Some(mv) = &mut self.mv {
            mv.visit_require(module, access, version);
        }
    }

    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.p.visit_export(packaze, access, modules);
        if let Some(mv) = &mut self.mv {
            mv.visit_export(packaze, access, modules);
        }
    }

    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.p.visit_open(packaze, access, modules);
        if let Some(mv) = &mut self.mv {
            mv.visit_open(packaze, access, modules);
        }
    }

    fn visit_use(&mut self, service: &JavaStr) {
        self.p.visit_use(service);
        if let Some(mv) = &mut self.mv {
            mv.visit_use(service);
        }
    }

    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        self.p.visit_provide(service, providers);
        if let Some(mv) = &mut self.mv {
            mv.visit_provide(service, providers);
        }
    }

    fn visit_end(&mut self) {
        self.p.visit_module_end();
        if let Some(mv) = &mut self.mv {
            mv.visit_end();
        }
    }
}
