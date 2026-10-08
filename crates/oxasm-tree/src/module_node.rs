//! [`ModuleNode`] and its parts: ASM's `ModuleNode`, `ModuleRequireNode`, `ModuleExportNode`,
//! `ModuleOpenNode` and `ModuleProvideNode`.

use std::sync::Arc;

use oxasm::{ClassVisitor, Interner, JavaStr, ModuleVisitor, Sym};

use crate::strings::resolve_all;

/// A node that represents a module declaration.
///
/// Its strings are [`Sym`]s of its interner. `ModuleNode::default()` makes a new interner.
#[derive(Clone, Debug, Default)]
pub struct ModuleNode {
    /// The interner of this node's strings.
    interner: Arc<Interner>,
    /// The fully qualified name (using dots) of this module.
    pub name: Sym,
    /// The module's access flags, among `ACC_OPEN`, `ACC_SYNTHETIC` and `ACC_MANDATED`.
    pub access: u32,
    /// The version of this module, or `None`.
    pub version: Option<Sym>,
    /// The internal name of the main class of this module, or `None`.
    pub main_class: Option<Sym>,
    /// The internal names of the packages declared by this module.
    pub packages: Vec<Sym>,
    /// The dependencies of this module.
    pub requires: Vec<ModuleRequireNode>,
    /// The packages exported by this module.
    pub exports: Vec<ModuleExportNode>,
    /// The packages opened by this module.
    pub opens: Vec<ModuleOpenNode>,
    /// The internal names of the services used by this module.
    pub uses: Vec<Sym>,
    /// The services provided by this module.
    pub provides: Vec<ModuleProvideNode>,
}

impl ModuleNode {
    /// Constructs a module node with no content.
    /// Constructs a module node with no content, whose strings are symbols of the given
    /// interner.
    pub fn new(
        interner: Arc<Interner>,
        name: Sym,
        access: u32,
        version: Option<Sym>,
    ) -> ModuleNode {
        ModuleNode {
            interner,
            name,
            access,
            version,
            main_class: None,
            packages: Vec::new(),
            requires: Vec::new(),
            exports: Vec::new(),
            opens: Vec::new(),
            uses: Vec::new(),
            provides: Vec::new(),
        }
    }

    /// The interner of this node's strings.
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }

    /// The string of the given symbol of this node.
    pub fn resolve(&self, sym: Sym) -> &JavaStr {
        self.interner.resolve(sym)
    }

    /// The symbol of the given string in this node's interner.
    pub fn intern(&self, s: &(impl AsRef<JavaStr> + ?Sized)) -> Sym {
        self.interner.intern(s)
    }

    /// Makes the given class visitor visit this module.
    ///
    /// Like ASM, this does not call the module visitor's `visit_end`.
    pub fn accept(&self, class_visitor: &mut dyn ClassVisitor) {
        let interner = &*self.interner;
        let Some(mut module_visitor) = class_visitor.visit_module(
            interner.resolve(self.name),
            self.access,
            self.version.map(|s| interner.resolve(s)),
        ) else {
            return;
        };
        if let Some(main_class) = self.main_class {
            module_visitor.visit_main_class(interner.resolve(main_class));
        }
        for &packaze in &self.packages {
            module_visitor.visit_package(interner.resolve(packaze));
        }
        for require in &self.requires {
            require.accept(interner, &mut *module_visitor);
        }
        for export in &self.exports {
            export.accept(interner, &mut *module_visitor);
        }
        for open in &self.opens {
            open.accept(interner, &mut *module_visitor);
        }
        for &service in &self.uses {
            module_visitor.visit_use(interner.resolve(service));
        }
        for provide in &self.provides {
            provide.accept(interner, &mut *module_visitor);
        }
    }
}

/// Two module nodes are equal if their content is: the same symbols, which are only comparable
/// when the nodes share an interner.
impl PartialEq for ModuleNode {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.access == other.access
            && self.version == other.version
            && self.main_class == other.main_class
            && self.packages == other.packages
            && self.requires == other.requires
            && self.exports == other.exports
            && self.opens == other.opens
            && self.uses == other.uses
            && self.provides == other.provides
    }
}

impl Eq for ModuleNode {}

impl ModuleVisitor for ModuleNode {
    fn visit_main_class(&mut self, main_class: &JavaStr) {
        self.main_class = Some(self.interner.intern(main_class));
    }

    fn visit_package(&mut self, packaze: &JavaStr) {
        self.packages.push(self.interner.intern(packaze));
    }

    fn visit_require(&mut self, module: &JavaStr, access: u32, version: Option<&JavaStr>) {
        let interner = &self.interner;
        self.requires.push(ModuleRequireNode::new(
            interner.intern(module),
            access,
            version.map(|v| interner.intern(v)),
        ));
    }

    fn visit_export(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        let interner = &self.interner;
        self.exports.push(ModuleExportNode::new(
            interner.intern(packaze),
            access,
            modules.iter().map(|&m| interner.intern(m)).collect(),
        ));
    }

    fn visit_open(&mut self, packaze: &JavaStr, access: u32, modules: &[&JavaStr]) {
        let interner = &self.interner;
        self.opens.push(ModuleOpenNode::new(
            interner.intern(packaze),
            access,
            modules.iter().map(|&m| interner.intern(m)).collect(),
        ));
    }

    fn visit_use(&mut self, service: &JavaStr) {
        self.uses.push(self.interner.intern(service));
    }

    fn visit_provide(&mut self, service: &JavaStr, providers: &[&JavaStr]) {
        let interner = &self.interner;
        self.provides.push(ModuleProvideNode::new(
            interner.intern(service),
            providers.iter().map(|&p| interner.intern(p)).collect(),
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
    pub module: Sym,
    /// The access flag of the dependence among `ACC_TRANSITIVE`, `ACC_STATIC_PHASE`,
    /// `ACC_SYNTHETIC` and `ACC_MANDATED`.
    pub access: u32,
    /// The module version at compile time, or `None`.
    pub version: Option<Sym>,
}

impl ModuleRequireNode {
    /// Constructs a new module require node.
    pub fn new(module: Sym, access: u32, version: Option<Sym>) -> ModuleRequireNode {
        ModuleRequireNode {
            module,
            access,
            version,
        }
    }

    /// Makes the given module visitor visit this require directive.
    pub fn accept(&self, interner: &Interner, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_require(
            interner.resolve(self.module),
            self.access,
            self.version.map(|s| interner.resolve(s)),
        );
    }
}

/// A node that represents an exported package with its name and the module that can access to
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleExportNode {
    /// The internal name of the exported package.
    pub packaze: Sym,
    /// The access flags of the exported package, valid values are among `ACC_SYNTHETIC` and
    /// `ACC_MANDATED`.
    pub access: u32,
    /// The list of modules that can access this exported package, specified with fully
    /// qualified names (using dots). Empty if the package is exported to all modules.
    pub modules: Vec<Sym>,
}

impl ModuleExportNode {
    /// Constructs a new module export node.
    pub fn new(packaze: Sym, access: u32, modules: Vec<Sym>) -> ModuleExportNode {
        ModuleExportNode {
            packaze,
            access,
            modules,
        }
    }

    /// Makes the given module visitor visit this export declaration.
    pub fn accept(&self, interner: &Interner, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_export(
            interner.resolve(self.packaze),
            self.access,
            &resolve_all(interner, &self.modules),
        );
    }
}

/// A node that represents an opened package with its name and the module that can access it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleOpenNode {
    /// The internal name of the opened package.
    pub packaze: Sym,
    /// The access flag of the opened package, valid values are among `ACC_SYNTHETIC` and
    /// `ACC_MANDATED`.
    pub access: u32,
    /// The fully qualified names (using dots) of the modules that can use deep reflection to
    /// the classes of the open package. Empty if the package is open to all modules.
    pub modules: Vec<Sym>,
}

impl ModuleOpenNode {
    /// Constructs a new module open node.
    pub fn new(packaze: Sym, access: u32, modules: Vec<Sym>) -> ModuleOpenNode {
        ModuleOpenNode {
            packaze,
            access,
            modules,
        }
    }

    /// Makes the given module visitor visit this opened package.
    pub fn accept(&self, interner: &Interner, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_open(
            interner.resolve(self.packaze),
            self.access,
            &resolve_all(interner, &self.modules),
        );
    }
}

/// A node that represents a service and its implementation provided by the current module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleProvideNode {
    /// The internal name of the service.
    pub service: Sym,
    /// The internal names of the implementations of the service (there is at least one
    /// provider).
    pub providers: Vec<Sym>,
}

impl ModuleProvideNode {
    /// Constructs a new module provide node.
    pub fn new(service: Sym, providers: Vec<Sym>) -> ModuleProvideNode {
        ModuleProvideNode { service, providers }
    }

    /// Makes the given module visitor visit this require declaration.
    pub fn accept(&self, interner: &Interner, module_visitor: &mut dyn ModuleVisitor) {
        module_visitor.visit_provide(
            interner.resolve(self.service),
            &resolve_all(interner, &self.providers),
        );
    }
}
