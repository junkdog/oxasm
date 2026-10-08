//! Port of ASM's `AnnotationNodeTest`. `testConstructor_illegalState` (subclasses must use the
//! constructor with an `api`) is dropped: there is no subclassing, nor `api` (ADR-09).
//!
//! An annotation node holds symbols of an interner; ASM's `AnnotationNode` visitor is
//! [`AnnotationNode::visitor`].

use oxasm::constant::AnnotationValue;
use oxasm::{AnnotationVisitor, Interner, JavaStr, jstr};
use oxasm_tree::{AnnotationNode, AnnotationNodeValue, AnnotationNodeVisitor};

#[test]
fn test_constructor() {
    let interner = Interner::new();
    let annotation_node = AnnotationNode::new(interner.intern("LI;"));

    assert_eq!(interner.resolve(annotation_node.desc), "LI;");
}

#[test]
fn test_visit() {
    use AnnotationNodeValue as N;
    use AnnotationValue as V;
    let interner = Interner::new();
    let mut annotation_node = AnnotationNode::new(interner.intern("LI;"));

    let name = |s: &'static str| Some(jstr!(s));
    {
        let mut visitor = annotation_node.visitor(&interner);
        visitor.visit(name("bytes"), &V::ByteArray(vec![0, 1].into()));
        visitor.visit(name("booleans"), &V::BooleanArray(vec![false, true].into()));
        visitor.visit(name("shorts"), &V::ShortArray(vec![0, 1].into()));
        visitor.visit(
            name("chars"),
            &V::CharArray(vec![u16::from(b'0'), u16::from(b'1')].into()),
        );
        visitor.visit(name("ints"), &V::IntArray(vec![0, 1].into()));
        visitor.visit(name("longs"), &V::LongArray(vec![0, 1].into()));
        visitor.visit(name("floats"), &V::FloatArray(vec![0.0, 1.0].into()));
        visitor.visit(name("doubles"), &V::DoubleArray(vec![0.0, 1.0].into()));
        visitor.visit(name("string"), &"value".into());
        visitor.visit_annotation(name("annotation"), jstr!("Lpkg/Annotation;"));
    }

    let values = &annotation_node.values;
    let names: Vec<&JavaStr> = values
        .iter()
        .map(|(n, _)| interner.resolve(n.unwrap()))
        .collect();
    assert_eq!(
        names,
        [
            "bytes",
            "booleans",
            "shorts",
            "chars",
            "ints",
            "longs",
            "floats",
            "doubles",
            "string",
            "annotation"
        ]
    );
    assert_eq!(values[0].1, N::Array(vec![N::Byte(0), N::Byte(1)]));
    assert_eq!(
        values[1].1,
        N::Array(vec![N::Boolean(false), N::Boolean(true)])
    );
    assert_eq!(values[2].1, N::Array(vec![N::Short(0), N::Short(1)]));
    assert_eq!(
        values[3].1,
        N::Array(vec![N::Char(u16::from(b'0')), N::Char(u16::from(b'1'))])
    );
    assert_eq!(values[4].1, N::Array(vec![N::Int(0), N::Int(1)]));
    assert_eq!(values[5].1, N::Array(vec![N::Long(0), N::Long(1)]));
    assert_eq!(values[6].1, N::Array(vec![N::Float(0.0), N::Float(1.0)]));
    assert_eq!(values[7].1, N::Array(vec![N::Double(0.0), N::Double(1.0)]));
    assert_eq!(values[8].1, N::String(interner.intern("value")));
    let N::Annotation(annotation) = &values[9].1 else {
        panic!("not an annotation: {:?}", values[9].1);
    };
    assert_eq!(interner.resolve(annotation.desc), "Lpkg/Annotation;");
}

#[test]
fn test_annotation_node_accept_skip_nested_annotations() {
    struct SkipNestedAnnotationsVisitor<'a>(AnnotationNodeVisitor<'a>);
    impl AnnotationVisitor for SkipNestedAnnotationsVisitor<'_> {
        fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
            Some(&mut self.0)
        }
        fn visit_annotation<'a>(
            &'a mut self,
            _name: Option<&JavaStr>,
            _descriptor: &JavaStr,
        ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            None
        }
        fn visit_array<'a>(
            &'a mut self,
            _name: Option<&JavaStr>,
        ) -> Option<Box<dyn AnnotationVisitor + 'a>> {
            None
        }
    }
    let interner = Interner::new();
    let mut annotation_node = AnnotationNode::new(interner.intern("LI;"));
    {
        let mut visitor = annotation_node.visitor(&interner);
        visitor.visit(
            Some(jstr!("bytes")),
            &AnnotationValue::ByteArray(vec![0, 1].into()),
        );
        visitor.visit_annotation(Some(jstr!("annotation")), jstr!("Lpkg/Annotation;"));
    }
    let mut dst_annotation_node = AnnotationNode::new(interner.intern("LJ;"));

    annotation_node.accept(
        &interner,
        &mut SkipNestedAnnotationsVisitor(dst_annotation_node.visitor(&interner)),
    );

    assert!(dst_annotation_node.values.is_empty());
}

/// oxasm-specific: enum values, nested arrays and annotations survive a visit and an accept.
#[test]
fn accept_replays_every_kind_of_value() {
    let interner = Interner::new();
    let mut annotation_node = AnnotationNode::new(interner.intern("LI;"));
    {
        let mut visitor = annotation_node.visitor(&interner);
        visitor.visit_enum(Some(jstr!("e")), jstr!("LE;"), jstr!("A"));
        let mut array = visitor.visit_array(Some(jstr!("a"))).unwrap();
        array.visit(None, &AnnotationValue::Int(1));
        {
            let mut nested = array.visit_array(None).unwrap();
            nested.visit_enum(None, jstr!("LE;"), jstr!("B"));
            nested.visit_end();
        }
        {
            let mut annotation = array.visit_annotation(None, jstr!("LN;")).unwrap();
            annotation.visit(Some(jstr!("x")), &AnnotationValue::Long(2));
            annotation.visit_end();
        }
        array.visit_end();
    }
    let mut copy = AnnotationNode::new(interner.intern("LI;"));

    annotation_node.accept(&interner, &mut copy.visitor(&interner));

    assert_eq!(copy, annotation_node);
    let mut nested = AnnotationNode::new(interner.intern("LN;"));
    nested
        .values
        .push((Some(interner.intern("x")), AnnotationNodeValue::Long(2)));
    assert_eq!(
        copy.values[1].1,
        AnnotationNodeValue::Array(vec![
            AnnotationNodeValue::Int(1),
            AnnotationNodeValue::Array(vec![AnnotationNodeValue::Enum {
                descriptor: interner.intern("LE;"),
                value: interner.intern("B")
            }]),
            AnnotationNodeValue::Annotation(nested),
        ])
    );
}
