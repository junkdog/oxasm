//! JVM class file reading, writing and transformation: a Rust port of [OW2 ASM] 9.10.1.
//!
//! This crate corresponds to ASM's `asm` module (`org.objectweb.asm` and
//! `org.objectweb.asm.signature`): the class reader and writer, the visitor traits, `Type`,
//! `Handle` and the signature parser.
//!
//! [OW2 ASM]: https://asm.ow2.io/
