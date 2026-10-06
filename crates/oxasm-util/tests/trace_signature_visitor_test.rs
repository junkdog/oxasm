//! Port of ASM's `TraceSignatureVisitorTest`.

use oxasm::opcodes;
use oxasm::signature::{SignatureReader, SignatureVisitor};
use oxasm_util::TraceSignatureVisitor;

const CLASS_SIGNATURES: &[(bool, &str, &str)] = &[
    (
        false,
        "<E extends java.lang.Enum<E>> implements java.lang.Comparable<E>, java.io.Serializable",
        "<E:Ljava/lang/Enum<TE;>;>Ljava/lang/Object;Ljava/lang/Comparable<TE;>;Ljava/io/Serializable;",
    ),
    (
        true,
        "<D extends java.lang.reflect.GenericDeclaration> extends java.lang.reflect.Type",
        "<D::Ljava/lang/reflect/GenericDeclaration;>Ljava/lang/Object;Ljava/lang/reflect/Type;",
    ),
    (
        false,
        "<K, V> extends java.util.AbstractMap<K, V> implements java.util.concurrent.ConcurrentMap<K, V>, java.io.Serializable",
        "<K:Ljava/lang/Object;V:Ljava/lang/Object;>Ljava/util/AbstractMap<TK;TV;>;Ljava/util/concurrent/ConcurrentMap<TK;TV;>;Ljava/io/Serializable;",
    ),
    (
        false,
        "<K extends java.lang.Enum<K>, V> extends java.util.AbstractMap<K, V> implements java.io.Serializable, java.lang.Cloneable",
        "<K:Ljava/lang/Enum<TK;>;V:Ljava/lang/Object;>Ljava/util/AbstractMap<TK;TV;>;Ljava/io/Serializable;Ljava/lang/Cloneable;",
    ),
    (
        false,
        "<T, R extends T>",
        "<T:Ljava/lang/Object;R:TT;>Ljava/lang/Object;",
    ),
];

const FIELD_SIGNATURES: &[(&str, &str)] = &[
    ("T[]", "[TT;"),
    ("AA<byte[][]>", "LAA<[[B>;"),
    ("java.lang.Class<?>", "Ljava/lang/Class<*>;"),
    (
        "java.lang.reflect.Constructor<T>",
        "Ljava/lang/reflect/Constructor<TT;>;",
    ),
    ("java.util.Hashtable<?, ?>", "Ljava/util/Hashtable<**>;"),
    (
        "java.util.concurrent.atomic.AtomicReferenceFieldUpdater<java.io.BufferedInputStream, byte[]>",
        "Ljava/util/concurrent/atomic/AtomicReferenceFieldUpdater<Ljava/io/BufferedInputStream;[B>;",
    ),
    (
        "AA<java.util.Map<java.lang.String, java.lang.String>[][]>",
        "LAA<[[Ljava/util/Map<Ljava/lang/String;Ljava/lang/String;>;>;",
    ),
    (
        "java.util.Hashtable<java.lang.Object, java.lang.String>",
        "Ljava/util/Hashtable<Ljava/lang/Object;Ljava/lang/String;>;",
    ),
    (
        "java.util.List<java.lang.Boolean[]>[][][]",
        "[[[Ljava/util/List<[Ljava/lang/Boolean;>;",
    ),
];

const METHOD_SIGNATURES: &[(&str, &str)] = &[
    ("void()E, F", "()V^TE;^TF;"),
    ("void(A<E>.B)", "(LA<TE;>.B;)V"),
    ("void(A<E>.B<F>)", "(LA<TE;>.B<TF;>;)V"),
    (
        "void(boolean, byte, char, short, int, float, long, double)",
        "(ZBCSIFJD)V",
    ),
    (
        "java.lang.Class<? extends E><E extends java.lang.Class>()",
        "<E:Ljava/lang/Class;>()Ljava/lang/Class<+TE;>;",
    ),
    (
        "java.lang.Class<? super E><E extends java.lang.Class>()",
        "<E:Ljava/lang/Class;>()Ljava/lang/Class<-TE;>;",
    ),
    (
        "void(java.lang.String, java.lang.Class<?>, java.lang.reflect.Method[], java.lang.reflect.Method, java.lang.reflect.Method)",
        "(Ljava/lang/String;Ljava/lang/Class<*>;[Ljava/lang/reflect/Method;Ljava/lang/reflect/Method;Ljava/lang/reflect/Method;)V",
    ),
    (
        "java.util.Map<java.lang.Object, java.lang.String>(java.lang.Object, java.util.Map<java.lang.Object, java.lang.String>)",
        "(Ljava/lang/Object;Ljava/util/Map<Ljava/lang/Object;Ljava/lang/String;>;)Ljava/util/Map<Ljava/lang/Object;Ljava/lang/String;>;",
    ),
    (
        "java.util.Map<java.lang.Object, java.lang.String><T>(java.lang.Object, java.util.Map<java.lang.Object, java.lang.String>, T)",
        "<T:Ljava/lang/Object;>(Ljava/lang/Object;Ljava/util/Map<Ljava/lang/Object;Ljava/lang/String;>;TT;)Ljava/util/Map<Ljava/lang/Object;Ljava/lang/String;>;",
    ),
    (
        "java.util.Map<java.lang.Object, java.lang.String><E, T extends java.lang.Comparable<E>>(java.lang.Object, java.util.Map<java.lang.Object, java.lang.String>, T)",
        "<E:Ljava/lang/Object;T::Ljava/lang/Comparable<TE;>;>(Ljava/lang/Object;Ljava/util/Map<Ljava/lang/Object;Ljava/lang/String;>;TT;)Ljava/util/Map<Ljava/lang/Object;Ljava/lang/String;>;",
    ),
    (
        "(java.util.Map<java.util.Map<java.lang.String, ? extends java.util.List<? super java.lang.Runnable>>, java.util.List<java.lang.Boolean[]>[]>)",
        "(Ljava/util/Map<Ljava/util/Map<Ljava/lang/String;+Ljava/util/List<-Ljava/lang/Runnable;>;>;[Ljava/util/List<[Ljava/lang/Boolean;>;>;)Ljava/lang/Object;",
    ),
];

#[test]
#[should_panic(expected = "invalid base type descriptor")]
fn test_visit_base_type_invalid_signature() {
    let mut trace_signature_visitor = TraceSignatureVisitor::new(0);
    trace_signature_visitor.visit_base_type('-');
}

#[test]
fn test_visit_methods_class_signature() {
    for &(is_interface, declaration, signature) in CLASS_SIGNATURES {
        let signature_reader = SignatureReader::new(signature);
        let mut trace_signature_visitor = TraceSignatureVisitor::new(if is_interface {
            opcodes::ACC_INTERFACE
        } else {
            0
        });
        signature_reader
            .accept(&mut trace_signature_visitor)
            .unwrap();
        assert_eq!(
            trace_signature_visitor.get_declaration(),
            declaration,
            "{signature}"
        );
    }
}

#[test]
fn test_visit_methods_field_signature() {
    for &(declaration, signature) in FIELD_SIGNATURES {
        let signature_reader = SignatureReader::new(signature);
        let mut trace_signature_visitor = TraceSignatureVisitor::new(0);
        signature_reader
            .accept_type(&mut trace_signature_visitor)
            .unwrap();
        assert_eq!(
            trace_signature_visitor.get_declaration(),
            declaration,
            "{signature}"
        );
    }
}

#[test]
fn test_visit_methods_method_signature() {
    for &(declaration, signature) in METHOD_SIGNATURES {
        let signature_reader = SignatureReader::new(signature);
        let mut trace_signature_visitor = TraceSignatureVisitor::new(0);
        signature_reader
            .accept(&mut trace_signature_visitor)
            .unwrap();
        let mut full_method_declaration = trace_signature_visitor
            .get_return_type()
            .map(ToString::to_string)
            .unwrap_or_else(|| "null".to_owned());
        full_method_declaration.push_str(&trace_signature_visitor.get_declaration().to_string());
        if let Some(exceptions) = trace_signature_visitor.get_exceptions() {
            full_method_declaration.push_str(&exceptions.to_string());
        }
        assert_eq!(full_method_declaration, declaration, "{signature}");
    }
}

/// oxasm-specific: malformed signatures, which a class file may contain, never make the visitor
/// panic (ASM's throws on some of them).
#[test]
fn test_malformed_signatures_do_not_panic() {
    for signature in [
        "<T:",
        "Ljava/util/List<",
        "(Ljava/util/List<*>",
        "<>",
        "LA<TE;>.B<",
        "[[[",
        "TT",
    ] {
        let mut trace_signature_visitor = TraceSignatureVisitor::new(0);
        let _ = SignatureReader::new(signature).accept(&mut trace_signature_visitor);
        let mut trace_signature_visitor = TraceSignatureVisitor::new(0);
        let _ = SignatureReader::new(signature).accept_type(&mut trace_signature_visitor);
    }
}
