//! [`ModuleNode`] and its parts: ASM's `ModuleNode`, `ModuleRequireNode`, `ModuleExportNode`,
//! `ModuleOpenNode` and `ModuleProvideNode`.

use oxasm::{ClassVisitor, JavaStr, JavaString, ModuleVisitor};

/// The elements of a list of strings, as the `&[&JavaStr]` a visitor takes.
fn as_strs(strings: &[JavaString]) -> Vec<&JavaStr> {
    strings.iter().map(|s| &**s).collect()
}

/// A node that represents a module declaration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleNode {
    /// The fully qualified name (using dots) of this module.
    pub name: JavaString,
    /// The module's access flags, among `ACC_OPEN`, `ACC_SYNTHETIC` and `ACC_MANDATED`.
    pub access: u32,
    /// The version of this module, or `None`.
    pub version: Option<JavaString>,
    /// The internal name of the main class of this module, or `None`.
    pub main_class: Option<JavaString>,
    /// The internal names of the packages declared by this module.
    pub packages: Vec<JavaString>,
    /// The dependencies of this module.
    pub requires: Vec<ModuleRequireNode>,
    /// The packages exported by this module.
    pub exports: Vec<ModuleExportNode>,
    /// The packages opened by this module.
    pub opens: Vec<ModuleOpenNode>,
    /// The internal names of the services used by this module.
    pub uses: Vec<JavaString>,
    /// The services provided by this module.
    pub provides: Vec<ModuleProvideNode>,
}

impl ModuleNode {
    /// Constructs a module node with no content.
    pub fn new(
        name: impl Into<JavaString>,
        access: u32,
        version: Option<JavaString>,
    ) -> ModuleNode {
        ModuleNode {
            name: name.into(),
            access,
            version,
            ..ModuleNode::default()
        }
    }

    /// Makes the given class visitor visit this module.
    ///
    /// Like ASM, this does not call the module visitor's `visit_end`.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let Some(mut module_visitor) =
            class_visitor.visit_module(&self.name, self.access, self.version.as_deref())
        else {
            return;
        };
        if let Some(main_class) = &self.main_class {
            module_visitor.visit_main_class(main_class);
        }
        for packaze in &self.packages {
            module_visitor.visit_package(packaze);
        }
        for require in &self.requires {
            require.accept(&mut *module_visitor);
        }
        for export in &self.exports {
            export.accept(&mut *module_visitor);
        }
        for open in &self.opens {
            open.accept(&mut *module_visitor);
        }
        for service in &self.uses {
            module_visitor.visit_use(service);
        }
        for provide in &self.provides {
            provide.accept(&mut *module_visitor);
        }
    }
}

impl ModuleVisitor for ModuleNode {
    fn visit_main_class(&mut self, main_class: &JavaStr) {
        self.main_class = Some(main_class.to_owned());
    }

    fn visit_package(&mut self, packaze: &JavaStr) {
        self.packages.push(packaze.to_owned());
    }

    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        self.requires.push(ModuleRequireNode::new(
            module,
            access,
            version.map(JavaStr::to_owned),
        ));
    }

    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.exports.push(ModuleExportNode::new(
            packaze,
            access,
            modules.iter().map(|&m| m.to_owned()).collect(),
        ));
    }

    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        self.opens.push(ModuleOpenNode::new(
            packaze,
            access,
            modules.iter().map(|&m| m.to_owned()).collect(),
        ));
    }

    fn visit_use(&mut self, service: &JavaStr) {
        self.uses.push(service.to_owned());
    }

    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        self.provides.push(ModuleProvideNode::new(
            service,
            providers.iter().map(|&p| p.to_owned()).collect(),
        ));
    }

    fn visit_end(&mut self) {
        // Nothing to do.
    }
}

/// A node that represents a required module with its name and access of a module descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRequireNode {
    /// The fully qualified name (using dots) of the dependence.
    pub module: JavaString,
    /// The access flag of the dependence among `ACC_TRANSITIVE`, `ACC_STATIC_PHASE`,
    /// `ACC_SYNTHETIC` and `ACC_MANDATED`.
    pub access: u32,
    /// The module version at compile time, or `None`.
    pub version: Option<JavaString>,
}

impl ModuleRequireNode {
    /// Constructs a new module require node.
    pub fn new(
        module: impl Into<JavaString>,
        access: u32,
        version: Option<JavaString>,
    ) -> ModuleRequireNode {
        ModuleRequireNode {
            module: module.into(),
            access,
            version,
        }
    }

    /// Makes the given module visitor visit this require directive.
    pub fn accept(&self, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_require(&self.module, self.access, self.version.as_deref());
    }
}

/// A node that represents an exported package with its name and the module that can access to
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleExportNode {
    /// The internal name of the exported package.
    pub packaze: JavaString,
    /// The access flags of the exported package, valid values are among `ACC_SYNTHETIC` and
    /// `ACC_MANDATED`.
    pub access: u32,
    /// The list of modules that can access this exported package, specified with fully
    /// qualified names (using dots). Empty if the package is exported to all modules.
    pub modules: Vec<JavaString>,
}

impl ModuleExportNode {
    /// Constructs a new module export node.
    pub fn new(
        packaze: impl Into<JavaString>,
        access: u32,
        modules: Vec<JavaString>,
    ) -> ModuleExportNode {
        ModuleExportNode {
            packaze: packaze.into(),
            access,
            modules,
        }
    }

    /// Makes the given module visitor visit this export declaration.
    pub fn accept(&self, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_export(&self.packaze, self.access, &as_strs(&self.modules));
    }
}

/// A node that represents an opened package with its name and the module that can access it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleOpenNode {
    /// The internal name of the opened package.
    pub packaze: JavaString,
    /// The access flag of the opened package, valid values are among `ACC_SYNTHETIC` and
    /// `ACC_MANDATED`.
    pub access: u32,
    /// The fully qualified names (using dots) of the modules that can use deep reflection to
    /// the classes of the open package. Empty if the package is open to all modules.
    pub modules: Vec<JavaString>,
}

impl ModuleOpenNode {
    /// Constructs a new module open node.
    pub fn new(
        packaze: impl Into<JavaString>,
        access: u32,
        modules: Vec<JavaString>,
    ) -> ModuleOpenNode {
        ModuleOpenNode {
            packaze: packaze.into(),
            access,
            modules,
        }
    }

    /// Makes the given module visitor visit this opened package.
    pub fn accept(&self, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_open(&self.packaze, self.access, &as_strs(&self.modules));
    }
}

/// A node that represents a service and its implementation provided by the current module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleProvideNode {
    /// The internal name of the service.
    pub service: JavaString,
    /// The internal names of the implementations of the service (there is at least one
    /// provider).
    pub providers: Vec<JavaString>,
}

impl ModuleProvideNode {
    /// Constructs a new module provide node.
    pub fn new(service: impl Into<JavaString>, providers: Vec<JavaString>) -> ModuleProvideNode {
        ModuleProvideNode {
            service: service.into(),
            providers,
        }
    }

    /// Makes the given module visitor visit this require declaration.
    pub fn accept(&self, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_provide(&self.service, &as_strs(&self.providers));
    }
}
