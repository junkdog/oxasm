//! [`StaticInitMerger`]: ASM's `StaticInitMerger`.

use std::fmt;

use oxasm::opcodes::*;
use oxasm::{ClassVisitor, JavaStr, JavaString, MethodVisitor, jstr};

/// A [`ClassVisitor`] that merges `<clinit>` methods into a single one. All the existing
/// `<clinit>` methods are renamed, and a new one is created, which calls all the renamed methods.
///
/// # Difference with ASM
///
/// ASM's merger visits the merged `<clinit>` method (with the downstream's `visitMethod`) right
/// after the first renamed one, and adds a call to it each time it renames a `<clinit>` method,
/// while the downstream visits the renamed methods and the next ones. Two child visitors of the
/// same class visitor cannot be alive at once here (ADR-02), so this merger records the calls,
/// and visits the merged method at [`visit_end`](ClassVisitor::visit_end), after all the other
/// methods. The merged method has the same content as ASM's, but it is the last method of the
/// class instead of the one after the first renamed method, and a class writer adds the
/// constants of its instructions to the constant pool after those of the other methods.
pub struct StaticInitMerger<V> {
    /// The class visitor to which this visitor delegates calls.
    cv: V,
    /// The internal name of the visited class.
    owner: Option<JavaString>,
    /// The prefix to use to rename the existing `<clinit>` methods.
    renamed_clinit_method_prefix: JavaString,
    /// The number of `<clinit>` methods visited so far.
    num_clinit_methods: i32,
    /// The merged `<clinit>` method, once a first `<clinit>` method has been visited: its
    /// descriptor (that of the first `<clinit>` method), and the owner, name and descriptor of
    /// the renamed methods it calls (ASM's `mergedClinitVisitor`, which visits these calls as the
    /// `<clinit>` methods are renamed).
    merged_clinit: Option<MergedClinit>,
}

/// The content of the merged `<clinit>` method.
#[derive(Debug)]
struct MergedClinit {
    descriptor: JavaString,
    /// The `INVOKESTATIC` instructions of the method: owner, name and descriptor.
    calls: Vec<(JavaString, JavaString, JavaString)>,
}

impl<V: ClassVisitor> StaticInitMerger<V> {
    /// Constructs a new [`StaticInitMerger`].
    ///
    /// - `prefix`: the prefix to use to rename the existing `<clinit>` methods.
    /// - `class_visitor`: the class visitor to which this visitor must delegate method calls.
    pub fn new(prefix: impl Into<JavaString>, class_visitor: V) -> Self {
        StaticInitMerger {
            cv: class_visitor,
            owner: None,
            renamed_clinit_method_prefix: prefix.into(),
            num_clinit_methods: 0,
            merged_clinit: None,
        }
    }

    /// The class visitor to which this visitor delegates calls.
    pub fn get_delegate(&mut self) -> &mut V {
        &mut self.cv
    }

    /// Returns this merger's class visitor.
    pub fn into_inner(self) -> V {
        self.cv
    }
}

impl<V> fmt::Debug for StaticInitMerger<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticInitMerger")
            .field("owner", &self.owner)
            .field(
                "renamed_clinit_method_prefix",
                &self.renamed_clinit_method_prefix,
            )
            .field("num_clinit_methods", &self.num_clinit_methods)
            .field("merged_clinit", &self.merged_clinit)
            .finish_non_exhaustive()
    }
}

impl<V: ClassVisitor> ClassVisitor for StaticInitMerger<V> {
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
        self.cv
            .visit(version, access, name, signature, super_name, interfaces);
        self.owner = Some(name.to_owned());
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        if name == "<clinit>" {
            let new_access = ACC_PRIVATE + ACC_STATIC;
            let mut new_name = self.renamed_clinit_method_prefix.clone();
            new_name.push_str(&self.num_clinit_methods.to_string());
            self.num_clinit_methods = self.num_clinit_methods.wrapping_add(1);

            let merged_clinit = self.merged_clinit.get_or_insert_with(|| MergedClinit {
                descriptor: descriptor.to_owned(),
                calls: Vec::new(),
            });
            let owner = self
                .owner
                .clone()
                .unwrap_or_else(|| jstr!("null").to_owned());
            merged_clinit
                .calls
                .push((owner, new_name.clone(), descriptor.to_owned()));
            self.cv
                .visit_method(new_access, &new_name, descriptor, signature, exceptions)
        } else {
            self.cv
                .visit_method(access, name, descriptor, signature, exceptions)
        }
    }

    fn visit_end(&mut self) {
        if let Some(merged_clinit) = self.merged_clinit.take() {
            // Like ASM, visit neither the code start nor the end of the merged method.
            if let Some(mut merged_clinit_visitor) = self.cv.visit_method(
                ACC_PRIVATE + ACC_STATIC,
                jstr!("<clinit>"),
                &merged_clinit.descriptor,
                None,
                &[],
            ) {
                for (owner, name, descriptor) in &merged_clinit.calls {
                    merged_clinit_visitor.visit_method_insn(
                        INVOKESTATIC,
                        owner,
                        name,
                        descriptor,
                        false,
                    );
                }
                merged_clinit_visitor.visit_insn(RETURN);
                merged_clinit_visitor.visit_maxs(0, 0);
            }
        }
        self.cv.visit_end();
    }
}
