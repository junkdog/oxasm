//! [`SerialVersionUidAdder`]: ASM's `SerialVersionUIDAdder`.

use std::cmp::Ordering;
use std::fmt;

use oxasm::opcodes::*;
use oxasm::{ClassVisitor, Constant, FieldVisitor, JavaStr, JavaString, MethodVisitor, jstr};
use sha1::{Digest, Sha1};

use crate::support::illegal_state;

/// The JVM name of static initializer methods.
const CLINIT: &str = "<clinit>";

/// A [`ClassVisitor`] that adds a serial version unique identifier to a class if missing. A
/// typical usage of this class is:
///
/// ```ignore
/// let mut class_writer = ClassWriter::new(...);
/// let mut svuid_adder = SerialVersionUidAdder::new(&mut class_writer);
/// let mut class_visitor = MyClassAdapter::new(&mut svuid_adder);
/// ClassReader::new(original_class)?.accept(&mut class_visitor, ParsingOptions::empty())?;
/// ```
///
/// The SVUID algorithm can be found at
/// <https://docs.oracle.com/javase/10/docs/specs/serialization/class.html#stream-unique-identifiers>:
///
/// The serialVersionUID is computed using the signature of a stream of bytes that reflect the
/// class definition. The National Institute of Standards and Technology (NIST) Secure Hash
/// Algorithm (SHA-1) is used to compute a signature for the stream. The first two 32-bit
/// quantities are used to form a 64-bit hash. A java.lang.DataOutputStream is used to convert
/// primitive data types to a sequence of bytes. The values input to the stream are defined by the
/// Java Virtual Machine (VM) specification for classes.
///
/// The sequence of items in the stream is as follows:
///
/// 1. The class name written using UTF encoding.
/// 2. The class modifiers written as a 32-bit integer.
/// 3. The name of each interface sorted by name written using UTF encoding.
/// 4. For each field of the class sorted by field name (except private static and private
///    transient fields):
///    1. The name of the field in UTF encoding.
///    2. The modifiers of the field written as a 32-bit integer.
///    3. The descriptor of the field in UTF encoding
/// 5. If a class initializer exists, write out the following:
///    1. The name of the method, `<clinit>`, in UTF encoding.
///    2. The modifier of the method, STATIC, written as a 32-bit integer.
///    3. The descriptor of the method, `()V`, in UTF encoding.
/// 6. For each non-private constructor sorted by method name and signature:
///    1. The name of the method, `<init>`, in UTF encoding.
///    2. The modifiers of the method written as a 32-bit integer.
///    3. The descriptor of the method in UTF encoding.
/// 7. For each non-private method sorted by method name and signature:
///    1. The name of the method in UTF encoding.
///    2. The modifiers of the method written as a 32-bit integer.
///    3. The descriptor of the method in UTF encoding.
/// 8. The SHA-1 algorithm is executed on the stream of bytes produced by DataOutputStream and
///    produces five 32-bit values sha\[0..4\].
/// 9. The hash value is assembled from the first and second 32-bit values of the SHA-1 message
///    digest. If the result of the message digest, the five 32-bit words H0 H1 H2 H3 H4, is in an
///    array of five int values named sha, the hash value would be computed as follows:
///
///    ```java
///    long hash = ((sha[0] >>> 24) & 0xFF) | ((sha[0] >>> 16) & 0xFF) << 8
///        | ((sha[0] >>> 8) & 0xFF) << 16 | ((sha[0] >>> 0) & 0xFF) << 24
///        | ((sha[1] >>> 24) & 0xFF) << 32 | ((sha[1] >>> 16) & 0xFF) << 40
///        | ((sha[1] >>> 8) & 0xFF) << 48 | ((sha[1] >>> 0) & 0xFF) << 56;
///    ```
///
/// Strings are written with Java's `DataOutputStream.writeUTF` (modified UTF-8), and sorted as
/// Java's `String.compareTo` sorts them (by UTF-16 code units).
pub struct SerialVersionUidAdder<V> {
    /// The class visitor to which this visitor delegates calls.
    cv: V,
    /// A flag that indicates if we need to compute SVUID.
    compute_svuid: bool,
    /// Whether the class already has a SVUID.
    has_svuid: bool,
    /// The class access flags.
    access: u32,
    /// The internal name of the class (`None` until a class that is not an enum is visited).
    name: Option<JavaString>,
    /// The interfaces implemented by the class.
    interfaces: Vec<JavaString>,
    /// The fields of the class that are needed to compute the SVUID.
    svuid_fields: Vec<Item>,
    /// Whether the class has a static initializer.
    has_static_initializer: bool,
    /// The constructors of the class that are needed to compute the SVUID.
    svuid_constructors: Vec<Item>,
    /// The methods of the class that are needed to compute the SVUID.
    svuid_methods: Vec<Item>,
}

impl<V: ClassVisitor> SerialVersionUidAdder<V> {
    /// Constructs a new [`SerialVersionUidAdder`].
    ///
    /// - `class_visitor`: a [`ClassVisitor`] to which this visitor will delegate calls.
    pub fn new(class_visitor: V) -> Self {
        SerialVersionUidAdder {
            cv: class_visitor,
            compute_svuid: false,
            has_svuid: false,
            access: 0,
            name: None,
            interfaces: Vec::new(),
            svuid_fields: Vec::new(),
            has_static_initializer: false,
            svuid_constructors: Vec::new(),
            svuid_methods: Vec::new(),
        }
    }

    /// The class visitor to which this visitor delegates calls.
    pub fn get_delegate(&mut self) -> &mut V {
        &mut self.cv
    }

    /// Returns this adder's class visitor.
    pub fn into_inner(self) -> V {
        self.cv
    }

    /// Returns true if the class already has a SVUID field. The result of this method is only
    /// valid when `visit_end` has been called.
    pub fn has_svuid(&self) -> bool {
        self.has_svuid
    }

    /// Adds a final static serialVersionUID field to the class, with the given value.
    fn add_svuid(&mut self, svuid: i64) {
        if let Some(mut field_visitor) = self.cv.visit_field(
            ACC_FINAL + ACC_STATIC,
            jstr!("serialVersionUID"),
            jstr!("J"),
            None,
            Some(&Constant::Long(svuid)),
        ) {
            field_visitor.visit_end();
        }
    }

    /// Computes and returns the value of SVUID, from what this visitor has visited so far.
    ///
    /// Returns an error where ASM's `DataOutputStream` throws an `IOException`: when a string
    /// is too long for `writeUTF` (more than 65535 bytes in modified UTF-8).
    ///
    /// # Panics
    ///
    /// If no class, or an enum, has been visited (ASM throws a `NullPointerException`).
    pub fn compute_svuid(&self) -> Result<i64, EncodedStringTooLong> {
        let Some(name) = &self.name else {
            panic!("java.lang.NullPointerException");
        };
        let mut data_output_stream = DataOutputStream::default();

        // 1. The class name written using UTF encoding.
        data_output_stream.write_utf(&name.replace('/', '.'))?;

        // 2. The class modifiers written as a 32-bit integer.
        let mut mods = self.access;
        if (mods & ACC_INTERFACE) != 0 {
            mods = if self.svuid_methods.is_empty() {
                mods & !ACC_ABSTRACT
            } else {
                mods | ACC_ABSTRACT
            };
        }
        data_output_stream
            .write_int(mods & (ACC_PUBLIC | ACC_FINAL | ACC_INTERFACE | ACC_ABSTRACT));

        // 3. The name of each interface sorted by name written using UTF encoding.
        let mut interfaces: Vec<&JavaStr> = self.interfaces.iter().map(|i| &**i).collect();
        interfaces.sort_by(|a, b| a.cmp_utf16(b));
        for interface_name in interfaces {
            data_output_stream.write_utf(&interface_name.replace('/', '.'))?;
        }

        // 4. For each field of the class sorted by field name (except private static and private
        // transient fields):
        //   1. The name of the field in UTF encoding.
        //   2. The modifiers of the field written as a 32-bit integer.
        //   3. The descriptor of the field in UTF encoding.
        // Note that field signatures are not dot separated. Method and constructor signatures are
        // dot separated. Go figure...
        write_items(&self.svuid_fields, &mut data_output_stream, false)?;

        // 5. If a class initializer exists, write out the following:
        //   1. The name of the method, <clinit>, in UTF encoding.
        //   2. The modifier of the method, ACC_STATIC, written as a 32-bit integer.
        //   3. The descriptor of the method, ()V, in UTF encoding.
        if self.has_static_initializer {
            data_output_stream.write_utf(jstr!(CLINIT))?;
            data_output_stream.write_int(ACC_STATIC);
            data_output_stream.write_utf(jstr!("()V"))?;
        }

        // 6. For each non-private constructor sorted by method name and signature:
        //   1. The name of the method, <init>, in UTF encoding.
        //   2. The modifiers of the method written as a 32-bit integer.
        //   3. The descriptor of the method in UTF encoding.
        write_items(&self.svuid_constructors, &mut data_output_stream, true)?;

        // 7. For each non-private method sorted by method name and signature:
        //   1. The name of the method in UTF encoding.
        //   2. The modifiers of the method written as a 32-bit integer.
        //   3. The descriptor of the method in UTF encoding.
        write_items(&self.svuid_methods, &mut data_output_stream, true)?;

        // 8. The SHA-1 algorithm is executed on the stream of bytes produced by DataOutputStream
        // and produces five 32-bit values sha[0..4].
        let hash_bytes = Sha1::digest(&data_output_stream.bytes);

        // 9. The hash value is assembled from the first and second 32-bit values of the SHA-1
        // message digest. If the result of the message digest, the five 32-bit words H0 H1 H2 H3
        // H4, is in an array of five int values named sha, the hash value would be computed as
        // follows (ASM shifts the first 8 bytes in, last first: a little endian read):
        let mut first_bytes = [0u8; 8];
        first_bytes.copy_from_slice(&hash_bytes[..8]);
        Ok(i64::from_le_bytes(first_bytes))
    }
}

impl<V> fmt::Debug for SerialVersionUidAdder<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SerialVersionUidAdder")
            .field("compute_svuid", &self.compute_svuid)
            .field("has_svuid", &self.has_svuid)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<V: ClassVisitor> ClassVisitor for SerialVersionUidAdder<V> {
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
        // Get the class name, access flags, and interfaces information (step 1, 2 and 3) for
        // SVUID computation.
        self.compute_svuid = (access & ACC_ENUM) == 0;

        if self.compute_svuid {
            self.name = Some(name.to_owned());
            self.access = access;
            self.interfaces = interfaces.iter().map(|&i| i.to_owned()).collect();
            self.svuid_fields = Vec::new();
            self.svuid_constructors = Vec::new();
            self.svuid_methods = Vec::new();
        }

        self.cv
            .visit(version, access, name, signature, super_name, interfaces);
    }

    fn visit_method<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        exceptions: &[&JavaStr],
    ) -> Option<Box<dyn MethodVisitor + 'a>> {
        // Get constructor and method information (step 5 and 7). Also determine if there is a
        // class initializer (step 6).
        if self.compute_svuid {
            if name == CLINIT {
                self.has_static_initializer = true;
            }
            // Collect the non private constructors and methods. Only the ACC_PUBLIC, ACC_PRIVATE,
            // ACC_PROTECTED, ACC_STATIC, ACC_FINAL, ACC_SYNCHRONIZED, ACC_NATIVE, ACC_ABSTRACT and
            // ACC_STRICT flags are used.
            let mods = access
                & (ACC_PUBLIC
                    | ACC_PRIVATE
                    | ACC_PROTECTED
                    | ACC_STATIC
                    | ACC_FINAL
                    | ACC_SYNCHRONIZED
                    | ACC_NATIVE
                    | ACC_ABSTRACT
                    | ACC_STRICT);

            if (access & ACC_PRIVATE) == 0 {
                if name == "<init>" {
                    self.svuid_constructors
                        .push(Item::new(name, mods, descriptor));
                } else if name != CLINIT {
                    self.svuid_methods.push(Item::new(name, mods, descriptor));
                }
            }
        }

        self.cv
            .visit_method(access, name, descriptor, signature, exceptions)
    }

    fn visit_field<'a>(
        &'a mut self,
        access: u32,
        name: &JavaStr,
        descriptor: &JavaStr,
        signature: Option<&JavaStr>,
        value: Option<&Constant<'_>>,
    ) -> Option<Box<dyn FieldVisitor + 'a>> {
        // Get the class field information for step 4 of the algorithm. Also determine if the
        // class already has a SVUID.
        if self.compute_svuid {
            if name == "serialVersionUID" {
                // Since the class already has SVUID, we won't be computing it.
                self.compute_svuid = false;
                self.has_svuid = true;
            }
            // Collect the non private fields. Only the ACC_PUBLIC, ACC_PRIVATE, ACC_PROTECTED,
            // ACC_STATIC, ACC_FINAL, ACC_VOLATILE, and ACC_TRANSIENT flags are used when
            // computing serialVersionUID values.
            if (access & ACC_PRIVATE) == 0 || (access & (ACC_STATIC | ACC_TRANSIENT)) == 0 {
                let mods = access
                    & (ACC_PUBLIC
                        | ACC_PRIVATE
                        | ACC_PROTECTED
                        | ACC_STATIC
                        | ACC_FINAL
                        | ACC_VOLATILE
                        | ACC_TRANSIENT);
                self.svuid_fields.push(Item::new(name, mods, descriptor));
            }
        }

        self.cv
            .visit_field(access, name, descriptor, signature, value)
    }

    fn visit_inner_class(
        &mut self,
        inner_class_name: &JavaStr,
        outer_name: Option<&JavaStr>,
        inner_name: Option<&JavaStr>,
        inner_class_access: u32,
    ) {
        // Handles a bizarre special case. Nested classes (static classes declared inside another
        // class) that are protected have their access bit set to public in their class files to
        // deal with some odd reflection situation. Our SVUID computation must do as the JVM does
        // and ignore access bits in the class file in favor of the access bits of the InnerClass
        // attribute.
        if self.name.as_deref() == Some(inner_class_name) {
            self.access = inner_class_access;
        }
        self.cv
            .visit_inner_class(inner_class_name, outer_name, inner_name, inner_class_access);
    }

    fn visit_end(&mut self) {
        // Add the SVUID field to the class if it doesn't have one.
        if self.compute_svuid && !self.has_svuid {
            match self.compute_svuid() {
                Ok(svuid) => self.add_svuid(svuid),
                Err(_) => illegal_state(format_args!(
                    "Error while computing SVUID for {}",
                    self.name.as_deref().unwrap_or(jstr!("null"))
                )),
            }
        }

        self.cv.visit_end();
    }
}

/// The error of Java's `DataOutputStream.writeUTF` (a `UTFDataFormatException`) on a string
/// whose modified UTF-8 encoding is longer than 65535 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedStringTooLong {
    /// The length of the encoded string, in bytes.
    pub length: usize,
}

impl fmt::Display for EncodedStringTooLong {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "encoded string too long: {} bytes", self.length)
    }
}

impl std::error::Error for EncodedStringTooLong {}

/// The subset of Java's `DataOutputStream` (into a `ByteArrayOutputStream`) that the SVUID
/// computation uses.
#[derive(Default)]
struct DataOutputStream {
    bytes: Vec<u8>,
}

impl DataOutputStream {
    /// Java's `writeUTF`: the length of the modified UTF-8 encoding as an unsigned short, then
    /// the encoding.
    fn write_utf(&mut self, value: &JavaStr) -> Result<(), EncodedStringTooLong> {
        let encoded = value.to_modified_utf8();
        let length = u16::try_from(encoded.len()).map_err(|_| EncodedStringTooLong {
            length: encoded.len(),
        })?;
        self.bytes.extend_from_slice(&length.to_be_bytes());
        self.bytes.extend_from_slice(&encoded);
        Ok(())
    }

    /// Java's `writeInt`: big endian.
    fn write_int(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
}

/// Sorts the items in the collection and writes it to the given output stream.
///
/// - `dotted`: whether package names must use dots, instead of slashes.
fn write_items(
    item_collection: &[Item],
    data_output_stream: &mut DataOutputStream,
    dotted: bool,
) -> Result<(), EncodedStringTooLong> {
    let mut items: Vec<&Item> = item_collection.iter().collect();
    // Java's Arrays.sort of objects is stable, like sort_by.
    items.sort_by(|a, b| a.compare_to(b));
    for item in items {
        data_output_stream.write_utf(&item.name)?;
        data_output_stream.write_int(item.access);
        if dotted {
            data_output_stream.write_utf(&item.descriptor.replace('/', '.'))?;
        } else {
            data_output_stream.write_utf(&item.descriptor)?;
        }
    }
    Ok(())
}

/// A field, constructor or method of the class, as the SVUID computation sees it.
#[derive(Clone, Debug)]
struct Item {
    name: JavaString,
    access: u32,
    descriptor: JavaString,
}

impl Item {
    fn new(name: &JavaStr, access: u32, descriptor: &JavaStr) -> Item {
        Item {
            name: name.to_owned(),
            access,
            descriptor: descriptor.to_owned(),
        }
    }

    /// Java's `compareTo`: by name, then by descriptor, as Java strings.
    fn compare_to(&self, item: &Item) -> Ordering {
        self.name
            .cmp_utf16(&item.name)
            .then_with(|| self.descriptor.cmp_utf16(&item.descriptor))
    }
}
