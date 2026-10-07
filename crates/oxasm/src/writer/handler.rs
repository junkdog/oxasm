//! ASM's `Handler`: the exception handlers of a method being written.

use crate::jstr::JavaCompactStr;
use crate::label::Label;
use crate::writer::byte_vector::ByteVector;
use crate::writer::label_table::LabelTable;

/// Information about an exception handler (ASM's `Handler`). Corresponds to an element of the
/// `exception_table` array of a Code attribute, as defined in the Java Virtual Machine
/// Specification (JVMS). A method writer keeps them in a `Vec`, in visit order (ASM's
/// `nextHandler` list).
#[derive(Clone, Debug)]
pub(crate) struct Handler {
    /// The `start_pc` field of this JVMS exception_table entry.
    pub(crate) start_pc: Label,
    /// The `end_pc` field of this JVMS exception_table entry.
    pub(crate) end_pc: Label,
    /// The `handler_pc` field of this JVMS exception_table entry.
    pub(crate) handler_pc: Label,
    /// The `catch_type` field of this JVMS exception_table entry. This is the constant pool
    /// index of the internal name of the type of exceptions handled by this handler, or 0 to
    /// catch any exceptions.
    pub(crate) catch_type: i32,
    /// The internal name of the type of exceptions handled by this handler, or `None` to catch
    /// any exceptions.
    #[allow(dead_code)] // used by the frame computation (P6)
    pub(crate) catch_type_descriptor: Option<JavaCompactStr>,
}

/// Returns the size in bytes of the JVMS `exception_table` corresponding to the given handlers,
/// including its `exception_table_length` field (ASM's `getExceptionTableSize`).
pub(crate) fn get_exception_table_size(handlers: &[Handler]) -> usize {
    2 + 8 * handlers.len()
}

/// Puts the JVMS `exception_table` corresponding to the given handlers in the given byte vector
/// (ASM's `putExceptionTable`).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Java's ints
pub(crate) fn put_exception_table(
    handlers: &[Handler],
    labels: &LabelTable,
    output: &mut ByteVector,
) {
    output.put_short(handlers.len() as i32);
    for handler in handlers {
        output
            .put_short(labels.offset(handler.start_pc))
            .put_short(labels.offset(handler.end_pc))
            .put_short(labels.offset(handler.handler_pc))
            .put_short(handler.catch_type);
    }
}

/// Port of ASM's `HandlerTest`. `removeRange` and the copy constructor it uses are P6 (the frame
/// computation): `testCopyConstructor` and the four `testRemoveRange_*` are pending. The tests of
/// the exception table use the two handlers `removeRange` makes in ASM's tests
/// (`[10, 13[` and `[17, 20[`), built directly.
#[cfg(test)]
mod tests {
    use super::{Handler, get_exception_table_size, put_exception_table};
    use crate::jstr::JavaCompactStr;
    use crate::label::Label;
    use crate::writer::byte_vector::ByteVector;
    use crate::writer::label_table::LabelTable;

    /// A label resolved at the given bytecode offset in `labels`.
    fn new_label(labels: &mut LabelTable, bytecode_offset: i32) -> Label {
        let label = Label::new();
        labels.resolve(label, &mut ByteVector::new(), None, bytecode_offset);
        label
    }

    fn new_handler(labels: &mut LabelTable, start_pc: i32, end_pc: i32) -> Handler {
        Handler {
            start_pc: new_label(labels, start_pc),
            end_pc: new_label(labels, end_pc),
            handler_pc: new_label(labels, 0),
            catch_type: 0,
            catch_type_descriptor: Some(JavaCompactStr::from("")),
        }
    }

    #[test]
    fn test_constructor() {
        let start_pc = Label::new();
        let end_pc = Label::new();
        let handler_pc = Label::new();
        let catch_type = 123;
        let catch_descriptor = JavaCompactStr::from("123");
        let handler = Handler {
            start_pc,
            end_pc,
            handler_pc,
            catch_type,
            catch_type_descriptor: Some(catch_descriptor.clone()),
        };
        assert_eq!(handler.start_pc, start_pc);
        assert_eq!(handler.end_pc, end_pc);
        assert_eq!(handler.handler_pc, handler_pc);
        assert_eq!(handler.catch_type, catch_type);
        assert_eq!(handler.catch_type_descriptor, Some(catch_descriptor));
    }

    /// ASM's `getExceptionTableLength` is the length of the handler list.
    #[test]
    fn test_get_exception_table_length() {
        let mut labels = LabelTable::default();
        let handler = new_handler(&mut labels, 10, 20);
        assert_eq!(Vec::<Handler>::new().len(), 0);
        assert_eq!([handler].len(), 1);
    }

    #[test]
    fn test_get_exception_table_size() {
        let mut labels = LabelTable::default();
        let handler_list = [
            new_handler(&mut labels, 10, 13),
            new_handler(&mut labels, 17, 20),
        ];
        assert_eq!(get_exception_table_size(&[]), 2);
        assert_eq!(get_exception_table_size(&handler_list), 18);
    }

    #[test]
    fn test_put_exception_table() {
        let mut labels = LabelTable::default();
        let handler_list = [
            new_handler(&mut labels, 10, 13),
            new_handler(&mut labels, 17, 20),
        ];
        let mut byte_vector = ByteVector::new();
        put_exception_table(&handler_list, &labels, &mut byte_vector);
        assert_eq!(byte_vector.size(), 18);
        assert_eq!(
            byte_vector.data(),
            [0, 2, 0, 10, 0, 13, 0, 0, 0, 0, 0, 17, 0, 20, 0, 0, 0, 0]
        );
    }
}
