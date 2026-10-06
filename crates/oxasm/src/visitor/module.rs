//! [`ModuleVisitor`]: ASM's `ModuleVisitor`.

use crate::jstr::JavaStr;

/// A visitor to visit a Java module. The methods of this trait must be called in the following
/// order: ( `visit_main_class` | ( `visit_package` | `visit_require` | `visit_export` |
/// `visit_open` | `visit_use` | `visit_provide` )* ) `visit_end`.
///
/// See the [module docs](crate::visitor) for the delegation conventions.
pub trait ModuleVisitor {
    /// The module visitor to which this visitor must delegate method calls; `None` (the default)
    /// ends the chain. Must return the same visitor every time.
    fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
        None
    }

    /// Visit the main class of the current module.
    ///
    /// - `main_class`: the internal name of the main class of the current module.
    fn visit_main_class(&mut self, main_class: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_main_class(main_class);
        }
    }

    /// Visit a package of the current module.
    ///
    /// - `packaze`: the internal name of a package.
    fn visit_package(&mut self, packaze: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_package(packaze);
        }
    }

    /// Visits a dependence of the current module.
    ///
    /// - `module`: the fully qualified name (using dots) of the dependence.
    /// - `access`: the access flag of the dependence among `ACC_TRANSITIVE`, `ACC_STATIC_PHASE`,
    ///   `ACC_SYNTHETIC` and `ACC_MANDATED`.
    /// - `version`: the module version at compile time, or `None`.
    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        if let Some(d) = self.delegate() {
            d.visit_require(module, access, version);
        }
    }

    /// Visit an exported package of the current module.
    ///
    /// - `packaze`: the internal name of the exported package.
    /// - `access`: the access flag of the exported package, valid values are among
    ///   `ACC_SYNTHETIC` and `ACC_MANDATED`.
    /// - `modules`: the fully qualified names (using dots) of the modules that can access the
    ///   public classes of the exported package, or empty.
    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        if let Some(d) = self.delegate() {
            d.visit_export(packaze, access, modules);
        }
    }

    /// Visit an open package of the current module.
    ///
    /// - `packaze`: the internal name of the opened package.
    /// - `access`: the access flag of the opened package, valid values are among `ACC_SYNTHETIC`
    ///   and `ACC_MANDATED`.
    /// - `modules`: the fully qualified names (using dots) of the modules that can use deep
    ///   reflection to the classes of the open package, or empty.
    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        if let Some(d) = self.delegate() {
            d.visit_open(packaze, access, modules);
        }
    }

    /// Visit a service used by the current module. The name must be the internal name of an
    /// interface or a class.
    ///
    /// - `service`: the internal name of the service.
    fn visit_use(&mut self, service: &JavaStr) {
        if let Some(d) = self.delegate() {
            d.visit_use(service);
        }
    }

    /// Visit an implementation of a service.
    ///
    /// - `service`: the internal name of the service.
    /// - `providers`: the internal names of the implementations of the service (there is at least
    ///   one provider).
    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        if let Some(d) = self.delegate() {
            d.visit_provide(service, providers);
        }
    }

    /// Visits the end of the module. This method, which is the last one to be called, is used to
    /// inform the visitor that everything have been visited.
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

macro_rules! forward_module_visitor {
    ($($ty:ty),*) => {$(
        impl<T: ModuleVisitor + ?Sized> ModuleVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn ModuleVisitor> {
                (**self).delegate()
            }
            fn visit_main_class(&mut self, main_class: &JavaStr) {
                (**self).visit_main_class(main_class);
            }
            fn visit_package(&mut self, packaze: &JavaStr) {
                (**self).visit_package(packaze);
            }
            fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
                (**self).visit_require(module, access, version);
            }
            fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
                (**self).visit_export(packaze, access, modules);
            }
            fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
                (**self).visit_open(packaze, access, modules);
            }
            fn visit_use(&mut self, service: &JavaStr) {
                (**self).visit_use(service);
            }
            fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
                (**self).visit_provide(service, providers);
            }
            fn visit_end(&mut self) {
                (**self).visit_end();
            }
        }
    )*};
}

forward_module_visitor!(&mut T, Box<T>);
