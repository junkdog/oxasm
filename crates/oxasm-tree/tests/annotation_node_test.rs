//! Port of ASM's `AnnotationNodeTest`. `testConstructor_illegalState` (subclasses must use the
//! constructor with an `api`) is dropped: there is no subclassing, nor `api` (ADR-09).

use oxasm::constant::AnnotationValue;
use oxasm::{AnnotationVisitor, JavaStr, jstr};
use oxasm_tree::{AnnotationNode, AnnotationNodeValue};

fn value(value: AnnotationValue<'static>) -> AnnotationNodeValue {
    AnnotationNodeValue::Value(value)
}

fn array(values: Vec<AnnotationValue<'static>>) -> AnnotationNodeValue {
    AnnotationNodeValue::Array(values.into_iter().map(value).collect())
}

#[test]
fn test_constructor() {
    let annotation_node = AnnotationNode::new("LI;");

    assert_eq!(annotation_node.desc, "LI;");
}

#[test]
fn test_visit() {
    use AnnotationValue as V;
    let mut annotation_node = AnnotationNode::new("LI;");

    let name = |s: &'static str| Some(jstr!(s));
    annotation_node.visit(name("bytes"), &V::ByteArray(vec![0, 1].into()));
    annotation_node.visit(name("booleans"), &V::BooleanArray(vec![false, true].into()));
    annotation_node.visit(name("shorts"), &V::ShortArray(vec![0, 1].into()));
    annotation_node.visit(
        name("chars"),
        &V::CharArray(vec![u16::from(b'0'), u16::from(b'1')].into()),
    );
    annotation_node.visit(name("ints"), &V::IntArray(vec![0, 1].into()));
    annotation_node.visit(name("longs"), &V::LongArray(vec![0, 1].into()));
    annotation_node.visit(name("floats"), &V::FloatArray(vec![0.0, 1.0].into()));
    annotation_node.visit(name("doubles"), &V::DoubleArray(vec![0.0, 1.0].into()));
    annotation_node.visit(name("string"), &"value".into());
    annotation_node.visit_annotation(name("annotation"), jstr!("Lpkg/Annotation;"));

    let values = &annotation_node.values;
    let names: Vec<&JavaStr> = values.iter().map(|(n, _)| n.as_deref().unwrap()).collect();
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
    assert_eq!(values[0].1, array(vec![V::Byte(0), V::Byte(1)]));
    assert_eq!(
        values[1].1,
        array(vec![V::Boolean(false), V::Boolean(true)])
    );
    assert_eq!(values[2].1, array(vec![V::Short(0), V::Short(1)]));
    assert_eq!(
        values[3].1,
        array(vec![V::Char(u16::from(b'0')), V::Char(u16::from(b'1'))])
    );
    assert_eq!(values[4].1, array(vec![V::Int(0), V::Int(1)]));
    assert_eq!(values[5].1, array(vec![V::Long(0), V::Long(1)]));
    assert_eq!(values[6].1, array(vec![V::Float(0.0), V::Float(1.0)]));
    assert_eq!(values[7].1, array(vec![V::Double(0.0), V::Double(1.0)]));
    assert_eq!(values[8].1, value("value".into()));
    let AnnotationNodeValue::Annotation(annotation) = &values[9].1 else {
        panic!("not an annotation: {:?}", values[9].1);
    };
    assert_eq!(annotation.desc, "Lpkg/Annotation;");
}

#[test]
fn test_annotation_node_accept_skip_nested_annotations() {
    struct SkipNestedAnnotationsVisitor<'a>(&'a mut AnnotationNode);
    impl AnnotationVisitor for SkipNestedAnnotationsVisitor<'_> {
        fn delegate(&mut self) -> Option<&mut dyn AnnotationVisitor> {
            Some(self.0)
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
    let mut annotation_node = AnnotationNode::new("LI;");
    annotation_node.visit(
        Some(jstr!("bytes")),
        &AnnotationValue::ByteArray(vec![0, 1].into()),
    );
    annotation_node.visit_annotation(Some(jstr!("annotation")), jstr!("Lpkg/Annotation;"));
    let mut dst_annotation_node = AnnotationNode::new("LJ;");

    annotation_node.accept(&mut SkipNestedAnnotationsVisitor(&mut dst_annotation_node));

    assert!(dst_annotation_node.values.is_empty());
}

/// oxasm-specific: enum values, nested arrays and annotations survive a visit and an accept.
#[test]
fn accept_replays_every_kind_of_value() {
    let mut annotation_node = AnnotationNode::new("LI;");
    annotation_node.visit_enum(Some(jstr!("e")), jstr!("LE;"), jstr!("A"));
    {
        let mut array = annotation_node.visit_array(Some(jstr!("a"))).unwrap();
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
    let mut copy = AnnotationNode::new("LI;");

    annotation_node.accept(&mut copy);

    assert_eq!(copy, annotation_node);
    let mut nested = AnnotationNode::new("LN;");
    nested.values.push((
        Some("x".into()),
        AnnotationNodeValue::Value(AnnotationValue::Long(2)),
    ));
    assert_eq!(
        copy.values[1].1,
        AnnotationNodeValue::Array(vec![
            AnnotationNodeValue::Value(AnnotationValue::Int(1)),
            AnnotationNodeValue::Array(vec![AnnotationNodeValue::Enum {
                descriptor: "LE;".into(),
                value: "B".into()
            }]),
            AnnotationNodeValue::Annotation(nested),
        ])
    );
}
