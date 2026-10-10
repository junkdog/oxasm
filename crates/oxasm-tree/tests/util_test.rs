//! Port of ASM's `UtilTest`. ASM's `Util` converts arrays to lists for the nodes' fields; it is
//! not ported, since the nodes hold `Vec`s and Java's `null` arrays are empty slices:
//!
//! - `testAsArrayList_nullArray` and `testAsArrayList_withLength` are dropped (there are no null
//!   arrays, and a chop frame carries its count in its kind instead of a list of `null`s);
//! - `testAsArrayList_withLengthAndArray`, the conversion of the first elements of an array, is
//!   what `FrameNode::new` does for a same locals 1 stack item frame.

use oxasm::FrameKind;
use oxasm_tree::{FrameNode, SymFrameValue};

#[test]
fn test_as_array_list_with_length_and_array() {
    let stack = [
        SymFrameValue::Integer,
        SymFrameValue::Float,
        SymFrameValue::Long,
        SymFrameValue::Double,
        SymFrameValue::Null,
    ];

    let frame_node = FrameNode::new(FrameKind::Same1, &[], &stack);

    assert_eq!(frame_node.stack(), [SymFrameValue::Integer]);
}
