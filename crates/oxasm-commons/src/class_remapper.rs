//! [`ClassRemapper`]: ASM's `ClassRemapper`.

use std::borrow::Cow;

use oxasm::{
    AnnotationVisitor, Attribute, ClassVisitor, Constant, FieldVisitor, JavaCompactStr, JavaStr,
    MethodVisitor, ModuleVisitor, RecordComponentVisitor, TypePath, TypeReference, jstr,
};

use crate::ModuleHashesAttribute;
use crate::annotation_remapper::AnnotationRemapper;
use crate::field_remapper::FieldRemapper;
use crate::method_remapper::MethodRemapper;
use crate::module_remapper::ModuleRemapper;
use crate::record_component_remapper::RecordComponentRemapper;
use crate::remapper::Remapper;

/// A [`ClassVisitor`] that remaps types with a [`Remapper`].
///
/// Its member visitors are a [`FieldRemapper`], a [`MethodRemapper`], a [`ModuleRemapper`], a
/// [`RecordComponentRemapper`] and [`AnnotationRemapper`]s wrapping the delegate's. ASM lets a
/// subclass replace them by overriding its `createXxxRemapper` factory methods; here a
/// transformation that needs other member visitors wraps them itself, in its own class visitor
/// (the member remappers are public).
pub struct ClassRemapper<V, R> {
    cv: V,
    /// The remapper used to remap the types in the visited class.
    remapper: R,
    /// The internal name of the visited class (empty before `visit`).
    class_name: JavaCompactStr,
}

impl<V: ClassVisitor, R: Remapper> ClassRemapper<V, R> {
    /// Constructs a new class remapper.
    ///
    /// - `class_visitor`: the class visitor this remapper must delegate to.
    /// - `remapper`: the remapper to use to remap the types in the visited class. Pass a
    ///   reference (`&remapper`) to share one remapper between several class remappers.
    pub fn new(class_visitor: V, remapper: R) -> Self {
        ClassRemapper {
            cv: class_visitor,
            remapper,
            class_name: JavaCompactStr::default(),
        }
    }

    /// The downstream class visitor.
    pub fn class_visitor(&self) -> &V {
        &self.cv
    }

    /// The downstream class visitor, mutably.
    pub fn class_visitor_mut(&mut self) -> &mut V {
        &mut self.cv
    }

    /// The remapper.
    pub fn remapper(&self) -> &R {
        &self.remapper
    }

    /// Returns the downstream class visitor.
    pub fn into_class_visitor(self) -> V {
        self.cv
    }
}

impl<V, R> std::fmt::Debug for ClassRemapper<V, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClassRemapper")
            .field("class_name", &self.class_name)
            .finish_non_exhaustive()
    }
}

/// The given strings as a slice of `&JavaStr`, for the visitor methods.
fn as_refs<'a>(strings: &'a [Cow<'_, JavaStr>]) -> Vec<&'a JavaStr> {
    strings.iter().map(|s| &**s).collect()
}

impl<V: ClassVisitor, R: Remapper> ClassVisitor for ClassRemapper<V, R> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.cv)
    }

    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        signature: Option<&JavaStr>,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        self.class_name = JavaCompactStr::from(name);
        let remapper = &self.remapper;
        let remapped_super_name = super_name.map(|super_name| remapper.map_type(super_name));
        let remapped_interfaces = remapper.map_types(interfaces);
        self.cv.visit(
            version,
            access,
            &remapper.map_type(name),
            remapper.map_signature(signature, false).as_deref(),
            remapped_super_name.as_deref(),
            &as_refs(&remapped_interfaces),
        );
    }

    fn visit_module<'a>(
        &'a mut self,
        name: &JavaStr,
        access: u32,
        version: Option<&JavaStr>,
    ) -> Option<Box<dyn ModuleVisitor + 'a>> {
        let remapper = &self.remapper;
        let module_visitor =
            self.cv
                .visit_module(&remapper.map_module_name(name), access, version)?;
        Some(Box::new(ModuleRemapper::new(module_visitor, remapper)))
    }

    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let remapper = &self.remapper;
        let annotation_visitor = self
            .cv
            .visit_annotation(&remapper.map_desc(descriptor), visible)?;
        Some(Box::new(AnnotationRemapper::new(
            Some(descriptor),
            annotation_visitor,
            remapper,
        )))
    }

    fn visit_type_annotation<'a>(
        &'a mut self,
        type_ref: TypeReference,
        type_path: Option<&TypePath>,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let remapper = &self.remapper;
        let annotation_visitor = self.cv.visit_type_annotation(
            type_ref,
            type_path,
            &remapper.map_desc(descriptor),
            visible,
        )?;
        Some(Box::new(AnnotationRemapper::new(
            Some(descriptor),
            annotation_visitor,
            remapper,
        )))
    }

    /// Remaps the module names of a [`ModuleHashesAttribute`]. ASM changes the attribute in
    /// place; here the delegate gets a remapped copy.
    fn visit_attribute(&mut self, attribute: &Attribute) {
        if let Attribute::Custom(custom) = attribute
            && let Some(module_hashes_attribute) = custom.downcast_ref::<ModuleHashesAttribute>()
        {
            let mut module_hashes_attribute = module_hashes_attribute.clone();
            for module in &mut module_hashes_attribute.modules {
                *module = self.remapper.map_module_name(module).into_owned();
            }
            self.cv
                .visit_attribute(&Attribute::Custom(Box::new(module_hashes_attribute)));
            return;
        }
        self.cv.visit_attribute(attribute);
    }

    fn visit_record_component<'a>(
        &'a mut self,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
    ) -> Option<Box<dyn RecordComponentVisitor + 'a>> {
        let remapper = &self.remapper;
        let record_component_visitor = self.cv.visit_record_component(
            &remapper.map_record_component_name(&self.class_name, name, descriptor),
            &remapper.map_desc(descriptor),
            remapper.map_signature(signature, true).as_deref(),
        )?;
        Some(Box::new(RecordComponentRemapper::new(
            record_component_visitor,
            remapper,
        )))
    }

    fn visit_field<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        let remapper = &self.remapper;
        let remapped_value = value.map(|value| remapper.map_value(value));
        let field_visitor = self.cv.visit_field(
            access,
            &remapper.map_field_name(&self.class_name, name, descriptor),
            &remapper.map_desc(descriptor),
            remapper.map_signature(signature, true).as_deref(),
            remapped_value.as_ref(),
        )?;
        Some(Box::new(FieldRemapper::new(field_visitor, remapper)))
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let remapper = &self.remapper;
        let remapped_descriptor = remapper.map_method_desc(descriptor);
        let remapped_exceptions = remapper.map_types(exceptions);
        let method_visitor = self.cv.visit_method(
            access,
            &remapper.map_method_name(&self.class_name, name, descriptor),
            &remapped_descriptor,
            remapper.map_signature(signature, false).as_deref(),
            &as_refs(&remapped_exceptions),
        )?;
        Some(Box::new(MethodRemapper::new(method_visitor, remapper)))
    }

    fn visit_inner_class(
        &mut self,
        name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        access: u32,
    ) {
        let remapper = &self.remapper;
        // In the order of Java's evaluation of the arguments (the first exception wins).
        let remapped_name = remapper.map_type(name);
        let remapped_outer_name = outer_name.map(|outer_name| remapper.map_type(outer_name));
        let remapped_inner_name = inner_name
            .map(|inner_name| remapper.map_inner_class_name(name, outer_name, inner_name));
        self.cv.visit_inner_class(
            &remapped_name,
            remapped_outer_name.as_deref(),
            remapped_inner_name.as_deref(),
            access,
        );
    }

    fn visit_outer_class(
        &mut self,
        owner: &JavaStr,
        name: Option<&JavaStr>,
        descriptor: Option<&JavaStr>,
    ) {
        let remapper = &self.remapper;
        // ASM passes a null descriptor on, which its string concatenations print as "null".
        let remapped_name = name
            .map(|name| remapper.map_method_name(owner, name, descriptor.unwrap_or(jstr!("null"))));
        let remapped_descriptor = descriptor.map(|descriptor| remapper.map_method_desc(descriptor));
        self.cv.visit_outer_class(
            &remapper.map_type(owner),
            remapped_name.as_deref(),
            remapped_descriptor.as_deref(),
        );
    }

    fn visit_nest_host(&mut self, nest_host: &JavaStr) {
        self.cv.visit_nest_host(&self.remapper.map_type(nest_host));
    }

    fn visit_nest_member(&mut self, nest_member: &JavaStr) {
        self.cv
            .visit_nest_member(&self.remapper.map_type(nest_member));
    }

    fn visit_permitted_subclass(&mut self, permitted_subclass: &JavaStr) {
        self.cv
            .visit_permitted_subclass(&self.remapper.map_type(permitted_subclass));
    }
}
