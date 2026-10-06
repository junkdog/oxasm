//! P1 shape spike (oxasm-port-plan P1, ADR-02/03/04): the visitor, label and string shapes under
//! real borrow-checker pressure, before the bulk port. Self-contained: the mini traits here have
//! the exact shape of the real `oxasm::visitor` traits (delegate + forwarding defaults, boxed
//! children borrowing their parent, the non-forwarding `try_copy_method` hook), but only a few
//! methods each.
//!
//! Exercised shapes:
//!
//! 1. A hand-fed event source (reader stub) driving `&mut dyn ClassVisitor`.
//! 2. A remapping adapter that wraps every child visitor (method, annotation, nested annotation)
//!    and borrows shared state (`&R`) alongside its downstream: disjoint field borrows.
//! 3. A stub writer that owns a symbol table and hands out `MethodWriter<'a>` borrowing the table
//!    *and* its own slot in the method list, with per-method label side tables (ADR-03).
//! 4. A `ClassNode`-like collector, replayed into two writers (the same `Label`s twice).
//! 5. Stateful method adapters (`&'a mut` counters), a tee (fan-out to two downstreams), and an
//!    adapter that emits an extra method at `visit_end`.
//! 6. The non-forwarding `try_copy_method` hook: a copy reaches the writer only when nothing
//!    in between overrides the method visitor.
//! 7. `JavaStr` round trips (lone surrogate, NUL, supplementary) through the whole chain and the
//!    writer's modified UTF-8 pool.
//!
//! Run: `cargo run -p oxasm --example spike_visitors`.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use oxasm::jstr::{JavaStr, JavaString};

// -------------------------------------------------------------------------------------------------
// ADR-03: Label is an opaque Copy id
// -------------------------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Label(NonZeroU64);

impl Label {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Label {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Label(NonZeroU64::new(NEXT.fetch_add(1, Ordering::Relaxed)).expect("label ids overflowed"))
    }
}

// -------------------------------------------------------------------------------------------------
// ADR-02: visitor traits
// -------------------------------------------------------------------------------------------------

pub trait AnnotationVisitor {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        None
    }
    fn visit(&mut self, name: Option<&JavaStr>, value: i32) {
        if let Some(d) = self.delegate() {
            d.visit(name, value);
        }
    }
    fn visit_annotation<'a>(
        &'a mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_annotation(name, descriptor)
    }
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

/// What a reader offers a writer for the method copy optimization (ADR-08).
#[derive(Debug)]
pub struct MethodCopySource<'r> {
    pub reader_id: u64,
    pub bytes: &'r [u8],
}

pub trait MethodVisitor {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        None
    }
    /// ADR-08: deliberately NOT forwarded to `delegate()`.
    #[doc(hidden)]
    fn try_copy_method(&mut self, _src: &MethodCopySource<'_>) -> bool {
        false
    }
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_annotation(descriptor, visible)
    }
    fn visit_code(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_code();
        }
    }
    fn visit_insn(&mut self, opcode: u8) {
        if let Some(d) = self.delegate() {
            d.visit_insn(opcode);
        }
    }
    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        if let Some(d) = self.delegate() {
            d.visit_method_insn(opcode, owner, name, descriptor);
        }
    }
    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        if let Some(d) = self.delegate() {
            d.visit_jump_insn(opcode, label);
        }
    }
    fn visit_label(&mut self, label: Label) {
        if let Some(d) = self.delegate() {
            d.visit_label(label);
        }
    }
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

pub trait ClassVisitor {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        None
    }
    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        if let Some(d) = self.delegate() {
            d.visit(version, access, name, super_name, interfaces);
        }
    }
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        self.delegate()?.visit_annotation(descriptor, visible)
    }
    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        self.delegate()?
            .visit_method(access, name, descriptor, exceptions)
    }
    fn visit_end(&mut self) {
        if let Some(d) = self.delegate() {
            d.visit_end();
        }
    }
}

// The reference/box impls forward EVERY method, `try_copy_method` included: they are the same
// visitor, not a delegate. (A `delegate()`-only impl would silently drop overrides.)
macro_rules! forward_class_visitor {
    ($($ty:ty),*) => {$(
        impl<T: ClassVisitor + ?Sized> ClassVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> { (**self).delegate() }
            fn visit(&mut self, v: u32, a: u32, n: &JavaStr, s: Option<&JavaStr>, i: &[&JavaStr]) { (**self).visit(v, a, n, s, i) }
            fn visit_annotation<'a>(&'a mut self, d: &JavaStr, v: bool) -> Option<Box<dyn AnnotationVisitor + 'a>> { (**self).visit_annotation(d, v) }
            fn visit_method<'a>(&'a mut self, a: u32, n: &JavaStr, d: &JavaStr, e: &[&JavaStr]) -> Option<Box<dyn MethodVisitor + 'a>> { (**self).visit_method(a, n, d, e) }
            fn visit_end(&mut self) { (**self).visit_end() }
        }
    )*};
}
forward_class_visitor!(&mut T, Box<T>);

macro_rules! forward_method_visitor {
    ($($ty:ty),*) => {$(
        impl<T: MethodVisitor + ?Sized> MethodVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> { (**self).delegate() }
            fn try_copy_method(&mut self, src: &MethodCopySource<'_>) -> bool { (**self).try_copy_method(src) }
            fn visit_annotation<'a>(&'a mut self, d: &JavaStr, v: bool) -> Option<Box<dyn AnnotationVisitor + 'a>> { (**self).visit_annotation(d, v) }
            fn visit_code(&mut self) { (**self).visit_code() }
            fn visit_insn(&mut self, o: u8) { (**self).visit_insn(o) }
            fn visit_method_insn(&mut self, o: u8, ow: &JavaStr, n: &JavaStr, d: &JavaStr) { (**self).visit_method_insn(o, ow, n, d) }
            fn visit_jump_insn(&mut self, o: u8, l: Label) { (**self).visit_jump_insn(o, l) }
            fn visit_label(&mut self, l: Label) { (**self).visit_label(l) }
            fn visit_end(&mut self) { (**self).visit_end() }
        }
    )*};
}
forward_method_visitor!(&mut T, Box<T>);

macro_rules! forward_annotation_visitor {
    ($($ty:ty),*) => {$(
        impl<T: AnnotationVisitor + ?Sized> AnnotationVisitor for $ty {
            fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> { (**self).delegate() }
            fn visit(&mut self, n: Option<&JavaStr>, v: i32) { (**self).visit(n, v) }
            fn visit_annotation<'a>(&'a mut self, n: Option<&JavaStr>, d: &JavaStr) -> Option<Box<dyn AnnotationVisitor + 'a>> { (**self).visit_annotation(n, d) }
            fn visit_end(&mut self) { (**self).visit_end() }
        }
    )*};
}
forward_annotation_visitor!(&mut T, Box<T>);

// -------------------------------------------------------------------------------------------------
// 1. Reader stub: a hand-fed event source
// -------------------------------------------------------------------------------------------------

const RETURN: u8 = 0xb1;
const GOTO: u8 = 0xa7;
const INVOKEVIRTUAL: u8 = 0xb6;

struct ReaderStub {
    id: u64,
    /// The awkward strings: a lone surrogate, NUL, a supplementary character.
    weird_name: JavaString,
    raw_method: Vec<u8>,
}

impl ReaderStub {
    fn new() -> Self {
        ReaderStub {
            id: 7,
            weird_name: JavaString::from_utf16(&[0x6d, 0xd800, 0x00, 0xd83d, 0xdca3]),
            raw_method: vec![0xca, 0xfe],
        }
    }

    /// Mirrors ClassReader.accept: one child at a time, each finished (and dropped) before the
    /// next is requested.
    fn accept(&self, cv: &mut dyn ClassVisitor) {
        cv.visit(
            52,
            0x21,
            JavaStr::from_str("a/Foo"),
            Some(JavaStr::from_str("java/lang/Object")),
            &[],
        );
        if let Some(mut av) = cv.visit_annotation(JavaStr::from_str("La/Ann;"), true) {
            av.visit(Some(JavaStr::from_str("x")), 1);
            if let Some(mut nested) = av.visit_annotation(
                Some(JavaStr::from_str("inner")),
                JavaStr::from_str("La/Inner;"),
            ) {
                nested.visit(None, 2);
                nested.visit_end();
            }
            av.visit_end();
        }
        // Method 1: copyable (raw bytes offered first, as ClassReader.readMethod does).
        if let Some(mut mv) = cv.visit_method(1, &self.weird_name, JavaStr::from_str("()V"), &[]) {
            let src = MethodCopySource {
                reader_id: self.id,
                bytes: &self.raw_method,
            };
            if !mv.try_copy_method(&src) {
                self.method_body(&mut *mv);
            }
        }
        // Method 2: annotation + body with a backward jump; the labels are this method's own.
        let exceptions = [JavaStr::from_str("java/io/IOException")];
        if let Some(mut mv) = cv.visit_method(
            1,
            JavaStr::from_str("run"),
            JavaStr::from_str("()V"),
            &exceptions,
        ) {
            if let Some(mut av) = mv.visit_annotation(JavaStr::from_str("La/Ann;"), false) {
                av.visit(Some(JavaStr::from_str("y")), 3);
                av.visit_end();
            }
            self.method_body(&mut *mv);
        }
        cv.visit_end();
    }

    fn method_body(&self, mv: &mut dyn MethodVisitor) {
        let top = Label::new();
        mv.visit_code();
        mv.visit_label(top);
        mv.visit_method_insn(
            INVOKEVIRTUAL,
            JavaStr::from_str("a/Foo"),
            &self.weird_name,
            JavaStr::from_str("()V"),
        );
        mv.visit_jump_insn(GOTO, top);
        mv.visit_insn(RETURN);
        mv.visit_end();
    }
}

// -------------------------------------------------------------------------------------------------
// 2. Remapping adapter (ADR-02, ADR-13)
// -------------------------------------------------------------------------------------------------

trait Remapper {
    fn map<'s>(&self, internal_name: &'s JavaStr) -> std::borrow::Cow<'s, JavaStr>;
}

struct SimpleRemapper(HashMap<JavaString, JavaString>);

impl Remapper for SimpleRemapper {
    fn map<'s>(&self, name: &'s JavaStr) -> std::borrow::Cow<'s, JavaStr> {
        match self.0.get(name) {
            Some(mapped) => std::borrow::Cow::Owned(mapped.clone()),
            None => std::borrow::Cow::Borrowed(name),
        }
    }
}

/// Generic over its downstream and its remapper (static dispatch inside the chain).
struct ClassRemapper<V, R> {
    next: V,
    remapper: R,
}

impl<V: ClassVisitor, R: Remapper> ClassVisitor for ClassRemapper<V, R> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.next)
    }
    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        super_name: Option<&JavaStr>,
        interfaces: &[&JavaStr],
    ) {
        let name = self.remapper.map(name);
        let super_name = super_name.map(|s| self.remapper.map(s));
        let interfaces: Vec<_> = interfaces.iter().map(|i| self.remapper.map(i)).collect();
        let interfaces: Vec<&JavaStr> = interfaces.iter().map(|i| &**i).collect();
        self.next
            .visit(version, access, &name, super_name.as_deref(), &interfaces);
    }
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        // Disjoint borrows: `self.next` mutably for the child, `self.remapper` shared.
        let av = self.next.visit_annotation(descriptor, visible)?;
        Some(Box::new(AnnotationRemapper {
            next: av,
            remapper: &self.remapper,
        }))
    }
    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let name = self.remapper.map(name);
        let exceptions: Vec<_> = exceptions.iter().map(|e| self.remapper.map(e)).collect();
        let exceptions: Vec<&JavaStr> = exceptions.iter().map(|e| &**e).collect();
        let mv = self
            .next
            .visit_method(access, &name, descriptor, &exceptions)?;
        Some(Box::new(MethodRemapper {
            next: mv,
            remapper: &self.remapper,
        }))
    }
}

struct MethodRemapper<'a, R> {
    next: Box<dyn MethodVisitor + 'a>,
    remapper: &'a R,
}

impl<R: Remapper> MethodVisitor for MethodRemapper<'_, R> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        Some(&mut *self.next)
    }
    fn visit_annotation<'b>(
        &'b mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        let av = self.next.visit_annotation(descriptor, visible)?;
        Some(Box::new(AnnotationRemapper {
            next: av,
            remapper: self.remapper,
        }))
    }
    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        let owner = self.remapper.map(owner);
        let name = self.remapper.map(name);
        self.next
            .visit_method_insn(opcode, &owner, &name, descriptor);
    }
}

struct AnnotationRemapper<'a, R> {
    next: Box<dyn AnnotationVisitor + 'a>,
    remapper: &'a R,
}

impl<R: Remapper> AnnotationVisitor for AnnotationRemapper<'_, R> {
    fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
        Some(&mut *self.next)
    }
    fn visit_annotation<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        // The nested child borrows *this* remapper's downstream; the remapper ref is copied.
        let av = self.next.visit_annotation(name, descriptor)?;
        Some(Box::new(AnnotationRemapper {
            next: av,
            remapper: self.remapper,
        }))
    }
}

// -------------------------------------------------------------------------------------------------
// 3. Writer stub (ADR-08 ownership, ADR-03 label side tables)
// -------------------------------------------------------------------------------------------------

#[derive(Default)]
struct SymbolTable {
    source_id: Option<u64>,
    pool: Vec<Vec<u8>>, // modified UTF-8 entries, insertion order
    index: HashMap<Vec<u8>, u16>,
}

impl SymbolTable {
    fn add_utf8(&mut self, s: &JavaStr) -> u16 {
        let bytes = s.to_modified_utf8().into_owned();
        if let Some(&i) = self.index.get(&bytes) {
            return i;
        }
        let i = u16::try_from(self.pool.len() + 1).expect("pool overflow");
        self.pool.push(bytes.clone());
        self.index.insert(bytes, i);
        i
    }
}

#[derive(Default, Debug)]
struct MethodState {
    name_index: u16,
    code: Vec<u8>,
    copied: Option<Vec<u8>>,
    annotations: usize,
}

#[derive(Default)]
struct ClassWriterStub {
    symbols: SymbolTable,
    methods: Vec<MethodState>,
    annotations: Vec<Vec<u8>>,
}

impl ClassWriterStub {
    fn from_reader(reader: &ReaderStub) -> Self {
        let mut cw = ClassWriterStub::default();
        cw.symbols.source_id = Some(reader.id);
        cw
    }
}

impl ClassVisitor for ClassWriterStub {
    fn visit(&mut self, _: u32, _: u32, name: &JavaStr, _: Option<&JavaStr>, _: &[&JavaStr]) {
        self.symbols.add_utf8(name);
    }
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        _: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        let type_index = self.symbols.add_utf8(descriptor);
        self.annotations.push(type_index.to_be_bytes().to_vec());
        // Borrows the symbol table and the annotation's own byte buffer: disjoint fields.
        let buf = self.annotations.last_mut().expect("just pushed");
        Some(Box::new(AnnotationWriterStub {
            symbols: &mut self.symbols,
            buf,
        }))
    }
    fn visit_method<'a>(
        &'a mut self,
        _: u32,
        name: &JavaStr,
        _: &JavaStr,
        _: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        let name_index = self.symbols.add_utf8(name);
        self.methods.push(MethodState {
            name_index,
            ..MethodState::default()
        });
        let state = self.methods.last_mut().expect("just pushed");
        Some(Box::new(MethodWriterStub {
            symbols: &mut self.symbols,
            state,
            labels: HashMap::new(),
            fixups: Vec::new(),
        }))
    }
}

struct AnnotationWriterStub<'a> {
    symbols: &'a mut SymbolTable,
    buf: &'a mut Vec<u8>,
}

impl AnnotationVisitor for AnnotationWriterStub<'_> {
    fn visit(&mut self, name: Option<&JavaStr>, value: i32) {
        if let Some(name) = name {
            let i = self.symbols.add_utf8(name);
            self.buf.extend_from_slice(&i.to_be_bytes());
        }
        self.buf.extend_from_slice(&value.to_be_bytes());
    }
    fn visit_annotation<'b>(
        &'b mut self,
        name: Option<&JavaStr>,
        descriptor: &JavaStr,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        if let Some(name) = name {
            let i = self.symbols.add_utf8(name);
            self.buf.extend_from_slice(&i.to_be_bytes());
        }
        let i = self.symbols.add_utf8(descriptor);
        self.buf.extend_from_slice(&i.to_be_bytes());
        // Reborrow both for the nested writer: it writes into the same buffer, as Java does.
        Some(Box::new(AnnotationWriterStub {
            symbols: &mut *self.symbols,
            buf: &mut *self.buf,
        }))
    }
}

struct MethodWriterStub<'a> {
    symbols: &'a mut SymbolTable,
    state: &'a mut MethodState,
    /// ADR-03: Label → bytecode offset, owned by this method writer.
    labels: HashMap<Label, usize>,
    /// Forward references: (offset of the 2-byte branch operand, opcode offset, target).
    fixups: Vec<(usize, usize, Label)>,
}

impl MethodVisitor for MethodWriterStub<'_> {
    fn try_copy_method(&mut self, src: &MethodCopySource<'_>) -> bool {
        if self.symbols.source_id != Some(src.reader_id) {
            return false;
        }
        // Copy immediately: no lifetime tie between writer and reader.
        self.state.copied = Some(src.bytes.to_vec());
        true
    }
    fn visit_annotation<'b>(
        &'b mut self,
        _: &JavaStr,
        _: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'b>> {
        self.state.annotations += 1;
        None // not interested in the values: prunes the reader's work
    }
    fn visit_insn(&mut self, opcode: u8) {
        self.state.code.push(opcode);
    }
    fn visit_method_insn(&mut self, opcode: u8, owner: &JavaStr, name: &JavaStr, _: &JavaStr) {
        self.symbols.add_utf8(owner);
        let i = self.symbols.add_utf8(name);
        self.state.code.push(opcode);
        self.state.code.extend_from_slice(&i.to_be_bytes());
    }
    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        let at = self.state.code.len();
        self.state.code.extend_from_slice(&[opcode, 0, 0]);
        self.fixups.push((at + 1, at, label));
    }
    fn visit_label(&mut self, label: Label) {
        self.labels.insert(label, self.state.code.len());
    }
    fn visit_end(&mut self) {
        for &(operand, insn, label) in &self.fixups {
            let target = self.labels[&label];
            let delta = i16::try_from(target as isize - insn as isize).expect("short jump");
            self.state.code[operand..operand + 2].copy_from_slice(&delta.to_be_bytes());
        }
    }
}

// -------------------------------------------------------------------------------------------------
// 4. ClassNode-like collector
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Insn {
    Plain(u8),
    Method {
        opcode: u8,
        owner: JavaString,
        name: JavaString,
        descriptor: JavaString,
    },
    Jump(u8, Label),
    Label(Label),
}

#[derive(Debug, Default, Clone, PartialEq)]
struct MethodNode {
    access: u32,
    name: JavaString,
    descriptor: JavaString,
    exceptions: Vec<JavaString>,
    invisible_annotations: Vec<JavaString>,
    insns: Vec<Insn>,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct ClassNode {
    version: u32,
    access: u32,
    name: JavaString,
    super_name: Option<JavaString>,
    methods: Vec<MethodNode>,
}

impl ClassVisitor for ClassNode {
    fn visit(
        &mut self,
        version: u32,
        access: u32,
        name: &JavaStr,
        super_name: Option<&JavaStr>,
        _: &[&JavaStr],
    ) {
        self.version = version;
        self.access = access;
        self.name = name.to_owned();
        self.super_name = super_name.map(JavaStr::to_owned);
    }
    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        self.methods.push(MethodNode {
            access,
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
            exceptions: exceptions.iter().map(|e| (*e).to_owned()).collect(),
            ..MethodNode::default()
        });
        // `&mut MethodNode` is itself a MethodVisitor via the blanket impl.
        Some(Box::new(self.methods.last_mut().expect("just pushed")))
    }
}

impl MethodVisitor for MethodNode {
    fn visit_annotation<'a>(
        &'a mut self,
        descriptor: &JavaStr,
        visible: bool,
    ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
        if !visible {
            self.invisible_annotations.push(descriptor.to_owned());
        }
        None
    }
    fn visit_insn(&mut self, opcode: u8) {
        self.insns.push(Insn::Plain(opcode));
    }
    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        self.insns.push(Insn::Method {
            opcode,
            owner: owner.to_owned(),
            name: name.to_owned(),
            descriptor: descriptor.to_owned(),
        });
    }
    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        self.insns.push(Insn::Jump(opcode, label));
    }
    fn visit_label(&mut self, label: Label) {
        self.insns.push(Insn::Label(label));
    }
}

impl ClassNode {
    /// Replays into any visitor; `&self`, so one node can be accepted any number of times.
    fn accept(&self, cv: &mut dyn ClassVisitor) {
        cv.visit(
            self.version,
            self.access,
            &self.name,
            self.super_name.as_deref(),
            &[],
        );
        for m in &self.methods {
            let exceptions: Vec<&JavaStr> = m.exceptions.iter().map(|e| &**e).collect();
            let Some(mut mv) = cv.visit_method(m.access, &m.name, &m.descriptor, &exceptions)
            else {
                continue;
            };
            for d in &m.invisible_annotations {
                if let Some(mut av) = mv.visit_annotation(d, false) {
                    av.visit_end();
                }
            }
            mv.visit_code();
            for insn in &m.insns {
                match insn {
                    Insn::Plain(op) => mv.visit_insn(*op),
                    Insn::Method {
                        opcode,
                        owner,
                        name,
                        descriptor,
                    } => mv.visit_method_insn(*opcode, owner, name, descriptor),
                    Insn::Jump(op, l) => mv.visit_jump_insn(*op, *l),
                    Insn::Label(l) => mv.visit_label(*l),
                }
            }
            mv.visit_end();
        }
        cv.visit_end();
    }
}

// -------------------------------------------------------------------------------------------------
// 5. Stateful adapters, tee, emit-at-end
// -------------------------------------------------------------------------------------------------

/// Counts instructions across all methods: the child borrows the counter *and* the downstream.
struct InsnCounter<V> {
    next: V,
    count: u32,
}

impl<V: ClassVisitor> ClassVisitor for InsnCounter<V> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut self.next)
    }
    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        // The child is created even if downstream returns None: the counter still counts.
        let next = self.next.visit_method(access, name, descriptor, exceptions);
        Some(Box::new(CountingMethod {
            next,
            count: &mut self.count,
        }))
    }
}

struct CountingMethod<'a> {
    next: Option<Box<dyn MethodVisitor + 'a>>,
    count: &'a mut u32,
}

impl MethodVisitor for CountingMethod<'_> {
    fn delegate(&mut self) -> Option<&mut dyn MethodVisitor> {
        self.next
            .as_deref_mut()
            .map(|mv| mv as &mut dyn MethodVisitor)
    }
    fn visit_insn(&mut self, opcode: u8) {
        *self.count += 1;
        if let Some(d) = self.delegate() {
            d.visit_insn(opcode);
        }
    }
    fn visit_method_insn(
        &mut self,
        opcode: u8,
        owner: &JavaStr,
        name: &JavaStr,
        descriptor: &JavaStr,
    ) {
        *self.count += 1;
        if let Some(d) = self.delegate() {
            d.visit_method_insn(opcode, owner, name, descriptor);
        }
    }
    fn visit_jump_insn(&mut self, opcode: u8, label: Label) {
        *self.count += 1;
        if let Some(d) = self.delegate() {
            d.visit_jump_insn(opcode, label);
        }
    }
}

/// Fans every event out to two downstreams; children borrow both, disjointly.
struct Tee<A, B>(A, B);

impl<A: ClassVisitor, B: ClassVisitor> ClassVisitor for Tee<A, B> {
    fn visit(&mut self, v: u32, a: u32, n: &JavaStr, s: Option<&JavaStr>, i: &[&JavaStr]) {
        self.0.visit(v, a, n, s, i);
        self.1.visit(v, a, n, s, i);
    }
    fn visit_method<'a>(
        &'a mut self,
        a: u32,
        n: &JavaStr,
        d: &JavaStr,
        e: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        match (
            self.0.visit_method(a, n, d, e),
            self.1.visit_method(a, n, d, e),
        ) {
            (Some(x), Some(y)) => Some(Box::new(Tee(x, y))),
            (x, y) => x.or(y),
        }
    }
    fn visit_end(&mut self) {
        self.0.visit_end();
        self.1.visit_end();
    }
}

impl<A: MethodVisitor, B: MethodVisitor> MethodVisitor for Tee<A, B> {
    fn visit_code(&mut self) {
        self.0.visit_code();
        self.1.visit_code();
    }
    fn visit_insn(&mut self, o: u8) {
        self.0.visit_insn(o);
        self.1.visit_insn(o);
    }
    fn visit_method_insn(&mut self, o: u8, ow: &JavaStr, n: &JavaStr, d: &JavaStr) {
        self.0.visit_method_insn(o, ow, n, d);
        self.1.visit_method_insn(o, ow, n, d);
    }
    fn visit_jump_insn(&mut self, o: u8, l: Label) {
        self.0.visit_jump_insn(o, l);
        self.1.visit_jump_insn(o, l);
    }
    fn visit_label(&mut self, l: Label) {
        self.0.visit_label(l);
        self.1.visit_label(l);
    }
    fn visit_end(&mut self) {
        self.0.visit_end();
        self.1.visit_end();
    }
}

/// Adds a `<clinit>` at `visit_end`, like StaticInitMerger/SerialVersionUIDAdder do.
struct AddClinit<'d> {
    next: &'d mut dyn ClassVisitor, // a `&mut dyn` downstream works as well as a generic one
}

impl ClassVisitor for AddClinit<'_> {
    fn delegate(&mut self) -> Option<&mut dyn ClassVisitor> {
        Some(&mut *self.next)
    }
    fn visit_end(&mut self) {
        if let Some(mut mv) = self.next.visit_method(
            8,
            JavaStr::from_str("<clinit>"),
            JavaStr::from_str("()V"),
            &[],
        ) {
            mv.visit_code();
            mv.visit_insn(RETURN);
            mv.visit_end();
        }
        self.next.visit_end();
    }
}

// -------------------------------------------------------------------------------------------------
// The checks
// -------------------------------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

fn main() {
    assert_send_sync::<Label>();
    assert_send_sync::<ClassNode>();
    assert_send_sync::<JavaString>();
    assert_send_sync::<&JavaStr>();

    let reader = ReaderStub::new();

    // 6a. reader → writer (same reader id): method 1 is copied, method 2 is written.
    let mut cw = ClassWriterStub::from_reader(&reader);
    reader.accept(&mut cw);
    assert_eq!(cw.methods.len(), 2);
    assert_eq!(cw.methods[0].copied.as_deref(), Some(&[0xca, 0xfe][..]));
    assert!(cw.methods[1].copied.is_none());
    let [hi, lo] = cw.methods[0].name_index.to_be_bytes(); // the weird name, interned once
    assert_eq!(
        cw.methods[1].code,
        [INVOKEVIRTUAL, hi, lo, GOTO, 0xff, 0xfd, RETURN]
    );
    assert_eq!(cw.methods[1].annotations, 1);
    assert_eq!(cw.annotations.len(), 1);

    // 6b. reader → remapper → writer: the remapper does not override try_copy_method, and the
    // default does not forward, so nothing is copied and the renames are applied.
    let weird = reader.weird_name.clone();
    let renamed = JavaString::from_utf16(&[0x6e, 0xdbff, 0x00]); // a different lone surrogate
    let mapping = HashMap::from([
        (JavaString::from("a/Foo"), JavaString::from("b/Bar")),
        (weird.clone(), renamed.clone()),
    ]);
    let mut cw2 = ClassWriterStub::from_reader(&reader);
    reader.accept(&mut ClassRemapper {
        next: &mut cw2,
        remapper: SimpleRemapper(mapping.clone()),
    });
    assert!(cw2.methods.iter().all(|m| m.copied.is_none()));
    let pool = |cw: &ClassWriterStub, i: u16| {
        JavaString::from_modified_utf8(&cw.symbols.pool[usize::from(i) - 1]).unwrap()
    };
    assert_eq!(pool(&cw2, cw2.methods[0].name_index), renamed);
    assert_eq!(pool(&cw2, 1), "b/Bar");
    // The class annotation went through AnnotationRemapper (and its nested child) into the writer.
    assert_eq!(cw2.annotations.len(), 1);

    // 7. JavaStr: the lone surrogate, NUL and the supplementary character survive the pool.
    let in_pool = cw.symbols.pool[usize::from(cw.methods[0].name_index) - 1].clone();
    assert_eq!(in_pool, b"m\xed\xa0\x80\xc0\x80\xed\xa0\xbd\xed\xb2\xa3");
    assert_eq!(JavaString::from_modified_utf8(&in_pool).unwrap(), weird);

    // 4. reader → ClassNode, then the same node into two writers (shared Label ids are fine,
    // each writer keeps its own side table).
    let mut node = ClassNode::default();
    reader.accept(&mut node);
    assert_eq!(node.methods.len(), 2);
    assert_eq!(node.methods[1].exceptions, ["java/io/IOException"]);
    assert_eq!(node.methods[1].invisible_annotations, ["La/Ann;"]);
    let (mut w1, mut w2) = (ClassWriterStub::default(), ClassWriterStub::default());
    node.accept(&mut w1);
    node.accept(&mut w2);
    assert_eq!(w1.methods[1].code, w2.methods[1].code);
    assert_eq!(w1.methods[1].code[3..], cw.methods[1].code[3..]); // same jump offsets

    // 5. counter + tee + emit-at-end, composed at runtime through `&mut dyn`.
    let mut tee = Tee(ClassNode::default(), ClassWriterStub::default());
    let mut add = AddClinit { next: &mut tee };
    let mut counter = InsnCounter {
        next: &mut add,
        count: 0,
    };
    reader.accept(&mut counter);
    let count = counter.count;
    assert_eq!(count, 6); // 3 per method body; <clinit> is added below the counter
    assert_eq!(tee.0.methods.len(), 3);
    assert_eq!(tee.1.methods.len(), 3);
    assert_eq!(tee.0.methods[2].name, "<clinit>");

    // The node round-trips through a boxed trait object, too.
    let mut boxed: Box<dyn ClassVisitor> = Box::new(ClassNode::default());
    node.accept(&mut boxed);

    println!("spike: all shapes hold");
}
