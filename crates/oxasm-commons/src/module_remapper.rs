//! [`ModuleRemapper`]: ASM's `ModuleRemapper`.

use std::borrow::Cow;

use oxasm::{JavaStr, ModuleVisitor};

use crate::remapper::Remapper;

/// A [`ModuleVisitor`] that remaps types with a [`Remapper`].
pub struct ModuleRemapper<'r, M, R: ?Sized> {
    mv: M,
    remapper: &'r R,
}

impl<'r, M: ModuleVisitor, R: Remapper + ?Sized> ModuleRemapper<'r, M, R> {
    /// Constructs a new module remapper.
    ///
    /// - `module_visitor`: the module visitor this remapper must delegate to.
    /// - `remapper`: the remapper to use to remap the types in the visited module.
    pub fn new(module_visitor: M, remapper: &'r R) -> Self {
        ModuleRemapper {
            mv: module_visitor,
            remapper,
        }
    }
}

impl<M, R: ?Sized> std::fmt::Debug for ModuleRemapper<'_, M, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleRemapper").finish_non_exhaustive()
    }
}

impl<M: ModuleVisitor, R: Remapper + ?Sized> ModuleVisitor for ModuleRemapper<'_, M, R> {
    fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
        Some(&mut self.mv)
    }

    fn visit_main_class(&mut self, main_class: &JavaStr) {
        self.mv
            .visit_main_class(&self.remapper.map_type(main_class));
    }

    fn visit_package(&mut self, packaze: &JavaStr) {
        self.mv
            .visit_package(&self.remapper.map_package_name(packaze));
    }

    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        self.mv
            .visit_require(&self.remapper.map_module_name(module), access, version);
    }

    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        let remapped_modules: Vec<Cow<'_, JavaStr>> = modules
            .iter()
            .map(|module| self.remapper.map_module_name(module))
            .collect();
        let remapped_modules: Vec<&JavaStr> = remapped_modules.iter().map(|m| &**m).collect();
        self.mv.visit_export(
            &self.remapper.map_package_name(packaze),
            access,
            &remapped_modules,
        );
    }

    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        let remapped_modules: Vec<Cow<'_, JavaStr>> = modules
            .iter()
            .map(|module| self.remapper.map_module_name(module))
            .collect();
        let remapped_modules: Vec<&JavaStr> = remapped_modules.iter().map(|m| &**m).collect();
        self.mv.visit_open(
            &self.remapper.map_package_name(packaze),
            access,
            &remapped_modules,
        );
    }

    fn visit_use(&mut self, service: &JavaStr) {
        self.mv.visit_use(&self.remapper.map_type(service));
    }

    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        let remapped_providers: Vec<Cow<'_, JavaStr>> = providers
            .iter()
            .map(|provider| self.remapper.map_type(provider))
            .collect();
        let remapped_providers: Vec<&JavaStr> = remapped_providers.iter().map(|p| &**p).collect();
        self.mv
            .visit_provide(&self.remapper.map_type(service), &remapped_providers);
    }
}
