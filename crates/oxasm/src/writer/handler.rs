//! ASM's `Handler`: the exception handlers of a method being written.

use crate::jstr::JavaCompactStr;
use crate::label::Label;
use crate::writer::byte_vector::ByteVector;
use crate::writer::label_table::LabelTable;

/// Information about an exception handler (ASM's `Handler`). Corresponds to an element of the
/// `exception_table` array of a Code attribute, as defined in the Java Virtual Machine
/// Specification (JVMS). A method writer keeps them in a `Vec`, in visit order (ASM's
/// `nextHandler` list).
#[derive(Clone, Debug, PartialEq)]
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
    pub(crate) catch_type_descriptor: Option<JavaCompactStr>,
}

impl Handler {
    /// Constructs a new handler from the given one, with a different scope (ASM's copy
    /// constructor).
    pub(crate) fn with_range(&self, start_pc: Label, end_pc: Label) -> Handler {
        Handler {
            start_pc,
            end_pc,
            ..self.clone()
        }
    }
}

/// Removes the range between `start` and `end` from the given exception handlers (ASM's
/// `removeRange`, which is recursive; each handler is replaced with zero, one or two handlers, in
/// order).
///
/// - `start`: the start of the range to be removed.
/// - `end`: the end of the range to be removed. Maybe `None`, for a range that ends with the
///   code.
///
/// Returns the exception handlers with the given range removed.
pub(crate) fn remove_range(
    handlers: Vec<Handler>,
    labels: &LabelTable,
    start: Label,
    end: Option<Label>,
) -> Vec<Handler> {
    let range_start = labels.offset(start);
    let range_end = end.map_or(i32::MAX, |end| labels.offset(end));
    let mut result = Vec::with_capacity(handlers.len() + 1);
    for handler in handlers {
        let handler_start = labels.offset(handler.start_pc);
        let handler_end = labels.offset(handler.end_pc);
        // Keep the handler if [handlerStart,handlerEnd[ and [rangeStart,rangeEnd[ don't
        // intersect.
        if range_start >= handler_end || range_end <= handler_start {
            result.push(handler);
        } else if range_start <= handler_start {
            // If [handlerStart,handlerEnd[ is included in [rangeStart,rangeEnd[, remove the
            // handler (range_end >= handler_end: `end` is not None).
            if let (Some(end), true) = (end, range_end < handler_end) {
                // [handlerStart,handlerEnd[ - [rangeStart,rangeEnd[ = [rangeEnd,handlerEnd[
                result.push(handler.with_range(end, handler.end_pc));
            }
        } else if range_end >= handler_end {
            // [handlerStart,handlerEnd[ - [rangeStart,rangeEnd[ = [handlerStart,rangeStart[
            result.push(handler.with_range(handler.start_pc, start));
        } else if let Some(end) = end {
            // [handlerStart,handlerEnd[ - [rangeStart,rangeEnd[ =
            //     [handlerStart,rangeStart[ + [rangeEnd,handerEnd[
            result.push(handler.with_range(handler.start_pc, start));
            result.push(handler.with_range(end, handler.end_pc));
        }
    }
    result
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

/// Port of ASM's `HandlerTest`. ASM's handler lists are `Vec`s: `null` is an empty list, and
/// `getExceptionTableLength` is the length of the `Vec`.
#[cfg(test)]
mod tests {
    use super::{Handler, get_exception_table_size, put_exception_table, remove_range};
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

    #[test]
    fn test_copy_constructor() {
        let start_pc1 = Label::new();
        let end_pc1 = Label::new();
        let handler_pc = Label::new();
        let catch_type = 123;
        let catch_descriptor = JavaCompactStr::from("123");
        let handler1 = Handler {
            start_pc: start_pc1,
            end_pc: end_pc1,
            handler_pc,
            catch_type,
            catch_type_descriptor: Some(catch_descriptor.clone()),
        };
        let start_pc2 = Label::new();
        let end_pc2 = Label::new();

        let handler2 = handler1.with_range(start_pc2, end_pc2);

        assert_eq!(handler2.start_pc, start_pc2);
        assert_eq!(handler2.end_pc, end_pc2);
        assert_eq!(handler2.handler_pc, handler_pc);
        assert_eq!(handler2.catch_type, catch_type);
        assert_eq!(handler2.catch_type_descriptor, Some(catch_descriptor));
    }

    #[test]
    fn test_remove_range_remove_all_or_nothing() {
        let mut labels = LabelTable::default();
        let handler = new_handler(&mut labels, 10, 20);
        let range = |labels: &mut LabelTable, start: i32, end: Option<i32>| {
            (
                new_label(labels, start),
                end.map(|end| new_label(labels, end)),
            )
        };

        let (start, end) = range(&mut labels, 0, Some(10));
        assert_eq!(remove_range(Vec::new(), &labels, start, end), []);
        let (start, end) = range(&mut labels, 0, Some(10));
        assert_eq!(
            remove_range(vec![handler.clone()], &labels, start, end),
            std::slice::from_ref(&handler)
        );
        let (start, end) = range(&mut labels, 20, Some(30));
        assert_eq!(
            remove_range(vec![handler.clone()], &labels, start, end),
            std::slice::from_ref(&handler)
        );
        let (start, end) = range(&mut labels, 20, None);
        assert_eq!(
            remove_range(vec![handler.clone()], &labels, start, end),
            std::slice::from_ref(&handler)
        );
        let (start, end) = range(&mut labels, 0, Some(30));
        assert_eq!(remove_range(vec![handler], &labels, start, end), []);
    }

    #[test]
    fn test_remove_range_remove_start() {
        let mut labels = LabelTable::default();
        let handler = new_handler(&mut labels, 10, 20);
        let (start, end) = (new_label(&mut labels, 0), new_label(&mut labels, 15));

        let handlers = remove_range(vec![handler], &labels, start, Some(end));

        assert_eq!(handlers.len(), 1);
        assert_eq!(labels.offset(handlers[0].start_pc), 15);
        assert_eq!(labels.offset(handlers[0].end_pc), 20);
    }

    #[test]
    fn test_remove_range_remove_end() {
        let mut labels = LabelTable::default();
        let handler = new_handler(&mut labels, 10, 20);
        let (start, end) = (new_label(&mut labels, 15), new_label(&mut labels, 30));

        let handlers = remove_range(vec![handler], &labels, start, Some(end));

        assert_eq!(handlers.len(), 1);
        assert_eq!(labels.offset(handlers[0].start_pc), 10);
        assert_eq!(labels.offset(handlers[0].end_pc), 15);
    }

    #[test]
    fn test_remove_range_remove_middle() {
        let mut labels = LabelTable::default();
        let handler = new_handler(&mut labels, 10, 20);
        let (start, end) = (new_label(&mut labels, 13), new_label(&mut labels, 17));

        let handlers = remove_range(vec![handler], &labels, start, Some(end));

        assert_eq!(handlers.len(), 2);
        assert_eq!(labels.offset(handlers[0].start_pc), 10);
        assert_eq!(labels.offset(handlers[0].end_pc), 13);
        assert_eq!(labels.offset(handlers[1].start_pc), 17);
        assert_eq!(labels.offset(handlers[1].end_pc), 20);
    }

    /// ASM's `getExceptionTableLength` is the length of the handler list.
    #[test]
    fn test_get_exception_table_length() {
        let mut labels = LabelTable::default();
        let handler = new_handler(&mut labels, 10, 20);
        assert_eq!(Vec::<Handler>::new().len(), 0);
        assert_eq!([handler].len(), 1);
    }

    /// The handlers `[10, 13[` and `[17, 20[`, made by removing `[13, 17[` from `[10, 20[`.
    fn new_handler_list(labels: &mut LabelTable) -> Vec<Handler> {
        let handler = new_handler(labels, 10, 20);
        let (start, end) = (new_label(labels, 13), new_label(labels, 17));
        remove_range(vec![handler], labels, start, Some(end))
    }

    #[test]
    fn test_get_exception_table_size() {
        let mut labels = LabelTable::default();
        let handler_list = new_handler_list(&mut labels);
        assert_eq!(get_exception_table_size(&[]), 2);
        assert_eq!(get_exception_table_size(&handler_list), 18);
    }

    #[test]
    fn test_put_exception_table() {
        let mut labels = LabelTable::default();
        let handler_list = new_handler_list(&mut labels);
        let mut byte_vector = ByteVector::new();
        put_exception_table(&handler_list, &labels, &mut byte_vector);
        assert_eq!(byte_vector.size(), 18);
        assert_eq!(
            byte_vector.data(),
            [0, 2, 0, 10, 0, 13, 0, 0, 0, 0, 0, 17, 0, 20, 0, 0, 0, 0]
        );
    }
}
