//! M1's acceptance probe (vault-docs/oxasm-port-plan.md, M1): the two pieces of sift's read side
//! that lean hardest on ASM's text and analysis APIs, ported onto oxasm.
//!
//! - **`TextifierLines`** (sift `cli-scan`): the line grammar sift's tree renderer reads Textifier
//!   output with. Pure string functions, ported as [`textifier_lines`].
//! - **`OriginSourceInterpreter`** (sift `core`): a `SourceInterpreter` whose copies and
//!   `CHECKCAST`s pass a tracked producer through, and whose parameters get a producer of their
//!   own (`ParameterOrigin`, a fake instruction node). Ported as [`OriginSourceInterpreter`], whose
//!   values are `SourceValue<Origin>`: a parameter's producer is no instruction.
//!
//! For each input class it prints a record in the oracle's batch format, whose payload is exactly
//! what the oracle's `sift-probe` command prints with sift's own classes (see `oracle/Oracle.java`
//! for the format), so the two outputs compare byte for byte:
//!
//! ```sh
//! S=~/opt/dev/jvm/sift   # after `mvn test-compile`
//! CP=$S/core/target/classes:$S/cli-scan/target/classes:<kotlin-stdlib.jar>
//! cargo xtask oracle sift-probe --sift $CP $S/core/target/test-classes > asm.txt
//! cargo run --release -p oxasm-util --example sift_probe -- $S/core/target/test-classes > oxasm.txt
//! cmp asm.txt oxasm.txt
//! ```
//!
//! Inputs are class files and directories (walked recursively, in the oracle's order); unlike the
//! oracle, jars are not read.

use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxasm::opcodes::CHECKCAST;
use oxasm::{ClassReader, Interner, JavaStr, JavaString, ParsingOptions, Type};
use oxasm_analysis::{
    Analyzer, AnalyzerError, InsnRef, Interpreter, SetArena, SourceInterpreter, SourceValue,
};
use oxasm_tree::{ClassNode, InsnId, InsnList, MethodNode};
use oxasm_util::printer::text_to_java_string;
use oxasm_util::{Printer, Textifier, TraceClassVisitor, TraceMethodVisitor};

use textifier_lines::{InstructionCategory, RenderToggles};

// -------------------------------------------------------------------------------------------------
// OriginSourceInterpreter
// -------------------------------------------------------------------------------------------------

/// What produced a value: an instruction, or the caller (sift's `ParameterOrigin`, the entry value
/// of the parameter in local `slot`).
///
/// sift mints a fresh `ParameterOrigin` node per parameter slot per analysis, compared by
/// identity; one analysis has one per slot, so comparing slots is the same relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Origin {
    Insn(InsnId),
    Parameter(usize),
}

impl From<InsnId> for Origin {
    fn from(insn: InsnId) -> Origin {
        Origin::Insn(insn)
    }
}

/// The values of [`OriginSourceInterpreter`]: `SourceValue`s whose producers may be parameters.
type OriginValue<'s> = SourceValue<'s, Origin>;

/// sift's `OriginSourceInterpreter`: "an instruction that doesn't change the value isn't its
/// producer", and "a value the caller produced needs a producer too". It overrides three
/// operations of `SourceInterpreter`, and delegates the others. Its merges allocate their sets
/// of three or more producers in `sets`.
struct OriginSourceInterpreter<'s> {
    sets: &'s SetArena,
}

impl<'s> Interpreter for OriginSourceInterpreter<'s> {
    type V = OriginValue<'s>;

    fn new_value(&mut self, ty: Option<&Type>) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        SourceInterpreter::new_value(ty)
    }

    /// A parameter's entry value is produced by the caller, with the size of its type.
    fn new_parameter_value(
        &mut self,
        _is_instance_method: bool,
        local: usize,
        ty: &Type,
    ) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        Ok(Some(SourceValue::of(
            ty.get_size(),
            Origin::Parameter(local),
        )))
    }

    fn new_operation(
        &mut self,
        insn: InsnRef<'_>,
    ) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        SourceInterpreter::new_operation(insn).map(Some)
    }

    /// A copy keeps the producers of its value, unless it has none.
    fn copy_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &OriginValue<'s>,
    ) -> Result<OriginValue<'s>, AnalyzerError> {
        Ok(if value.insns.is_empty() {
            SourceInterpreter::copy_operation(insn, value)
        } else {
            *value
        })
    }

    /// A `CHECKCAST` keeps the producers of its value, unless it has none.
    fn unary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value: &OriginValue<'s>,
    ) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        if insn.opcode() == Some(CHECKCAST) && !value.insns.is_empty() {
            return Ok(Some(*value));
        }
        SourceInterpreter::unary_operation(insn, value).map(Some)
    }

    fn binary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &OriginValue<'s>,
        value2: &OriginValue<'s>,
    ) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        Ok(Some(SourceInterpreter::binary_operation(
            insn, value1, value2,
        )))
    }

    fn ternary_operation(
        &mut self,
        insn: InsnRef<'_>,
        value1: &OriginValue<'s>,
        value2: &OriginValue<'s>,
        value3: &OriginValue<'s>,
    ) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        Ok(Some(SourceInterpreter::ternary_operation(
            insn, value1, value2, value3,
        )))
    }

    fn nary_operation(
        &mut self,
        insn: InsnRef<'_>,
        values: &[OriginValue<'s>],
        return_type: Option<&Type<'_>>,
    ) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        SourceInterpreter::nary_operation(insn, values, return_type).map(Some)
    }

    fn return_operation(
        &mut self,
        _insn: InsnRef<'_>,
        _value: &OriginValue<'s>,
        _expected: Option<&OriginValue<'s>>,
    ) -> Result<(), AnalyzerError> {
        Ok(())
    }

    fn merge(
        &mut self,
        value1: &OriginValue<'s>,
        value2: &OriginValue<'s>,
    ) -> Result<Option<OriginValue<'s>>, AnalyzerError> {
        Ok(SourceInterpreter::merge(self.sets, value1, value2))
    }

    /// The merges of `SourceInterpreter`, so its loop over a frame's values too.
    fn merge_values(
        &mut self,
        values: &mut [OriginValue<'s>],
        other: &[OriginValue<'s>],
    ) -> Result<bool, AnalyzerError> {
        Ok(SourceInterpreter::merge_values(self.sets, values, other))
    }
}

// -------------------------------------------------------------------------------------------------
// TextifierLines
// -------------------------------------------------------------------------------------------------

/// sift's `TextifierLines` (and the `categoryByMnemonic` and `RenderToggles` it reads): total
/// functions over one line of Textifier output.
///
/// Kotlin indexes strings by UTF-16 unit and these functions by byte. Every delimiter they search
/// for is ASCII, which never occurs inside a multi-byte sequence, so the cuts are the same.
/// Whitespace is Kotlin's `Char.isWhitespace()`, not Rust's. One deliberate difference: Kotlin's
/// `isDigit()` and `toIntOrNull()` accept any Unicode decimal digit, these functions ASCII digits
/// only. Textifier prints numbers in ASCII, so this only shows on a constant dynamic whose name
/// starts with another script's digit (an `LDC` sift takes for a numeric constant).
mod textifier_lines {
    use oxasm::jstr::JavaCodePoint;
    use oxasm::opcodes::*;
    use oxasm::{JavaStr, JavaString};
    use oxasm_util::printer::OPCODES;

    /// sift's `InstructionCategory`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InstructionCategory {
        Const,
        Load,
        Store,
        Stack,
        Math,
        Cast,
        Field,
        Invoke,
        Object,
        Control,
    }

    impl InstructionCategory {
        /// The Kotlin enum constant's name.
        pub fn name(self) -> &'static str {
            match self {
                InstructionCategory::Const => "CONST",
                InstructionCategory::Load => "LOAD",
                InstructionCategory::Store => "STORE",
                InstructionCategory::Stack => "STACK",
                InstructionCategory::Math => "MATH",
                InstructionCategory::Cast => "CAST",
                InstructionCategory::Field => "FIELD",
                InstructionCategory::Invoke => "INVOKE",
                InstructionCategory::Object => "OBJECT",
                InstructionCategory::Control => "CONTROL",
            }
        }
    }

    /// The toggles of sift's `RenderToggles` that `should_emit` reads.
    #[derive(Clone, Copy, Debug)]
    #[allow(clippy::struct_excessive_bools)]
    pub struct RenderToggles {
        pub include_line_numbers: bool,
        pub include_local_variables: bool,
        pub include_try_catch: bool,
        pub include_frames: bool,
    }

    /// The `(mnemonic, owner, name, desc)` quadruple of [`invoke_match`] and [`field_match`].
    #[derive(Debug)]
    pub struct MatchTokens<'a> {
        pub mnemonic: &'a JavaStr,
        pub owner: &'a JavaStr,
        pub name: &'a JavaStr,
        pub desc: &'a JavaStr,
    }

    /// The `(mnemonic, slot)` pair of [`var_match`].
    #[derive(Debug)]
    pub struct VarTokens<'a> {
        pub mnemonic: &'a JavaStr,
        pub slot: i32,
    }

    const FIELD_MNEMONICS: [&str; 4] = ["GETFIELD", "PUTFIELD", "GETSTATIC", "PUTSTATIC"];

    const VAR_MNEMONICS: [&str; 12] = [
        "ILOAD", "LLOAD", "FLOAD", "DLOAD", "ALOAD", "ISTORE", "LSTORE", "FSTORE", "DSTORE",
        "ASTORE", "RET", "IINC",
    ];

    /// An INVOKE row (`INVOKEVIRTUAL com/acme/Foo.bar (Ljava/lang/String;)V`), INVOKEDYNAMIC
    /// excluded.
    pub fn invoke_match(body: &JavaStr) -> Option<MatchTokens<'_>> {
        let space1 = body.find(' ')?;
        let mnemonic = &body[..space1];
        if !mnemonic.starts_with("INVOKE") || mnemonic == "INVOKEDYNAMIC" {
            return None;
        }
        let open_paren = space1 + 1 + body[space1 + 1..].find('(')?;
        let name_dot = body[..open_paren].rfind('.')?;
        if name_dot <= space1 {
            return None;
        }
        let owner = trim(&body[space1 + 1..name_dot]);
        let name_end = name_end(body, name_dot, open_paren);
        let name = trim(&body[name_dot + 1..name_end]);
        let desc = trim_end(&body[open_paren..]);
        let desc = desc.strip_suffix(" (itf)").unwrap_or(desc);
        Some(MatchTokens {
            mnemonic,
            owner,
            name,
            desc,
        })
    }

    /// Replaces the name token of an INVOKE row that [`invoke_match`] matched.
    pub fn splice_invoke_name(body: &JavaStr, display_name: &JavaStr) -> JavaString {
        let space1 = body.find(' ').expect("an INVOKE row");
        let open_paren = space1 + 1 + body[space1 + 1..].find('(').expect("an INVOKE row");
        let name_dot = body[..open_paren].rfind('.').expect("an INVOKE row");
        let name_end = name_end(body, name_dot, open_paren);
        let mut spliced = JavaString::from(&body[..name_dot + 1]);
        spliced.push_java_str(display_name);
        spliced.push_java_str(&body[name_end..]);
        spliced
    }

    /// The end of an INVOKE row's name: the last space before the descriptor, if after the dot.
    fn name_end(body: &JavaStr, name_dot: usize, open_paren: usize) -> usize {
        body[..open_paren]
            .rfind(' ')
            .filter(|&space| space > name_dot)
            .unwrap_or(open_paren)
    }

    /// A FIELD row (`GETFIELD com/acme/FooService.repo : Lcom/acme/Repo;`).
    pub fn field_match(body: &JavaStr) -> Option<MatchTokens<'_>> {
        let space1 = body.find(' ')?;
        let mnemonic = &body[..space1];
        if !FIELD_MNEMONICS.iter().any(|&m| mnemonic == m) {
            return None;
        }
        let colon = space1 + 1 + body[space1 + 1..].find(" : ")?;
        let owner_and_name = trim(&body[space1 + 1..colon]);
        let last_dot = owner_and_name.rfind('.')?;
        Some(MatchTokens {
            mnemonic,
            owner: &owner_and_name[..last_dot],
            name: &owner_and_name[last_dot + 1..],
            desc: trim(&body[colon + 3..]),
        })
    }

    /// Replaces the name token of a FIELD row that [`field_match`] matched.
    pub fn splice_field_name(body: &JavaStr, display_name: &JavaStr) -> JavaString {
        let space1 = body.find(' ').expect("a FIELD row");
        let colon = space1 + 1 + body[space1 + 1..].find(" : ").expect("a FIELD row");
        let name_dot = body[..colon].rfind('.').expect("a FIELD row");
        let mut spliced = JavaString::from(&body[..name_dot + 1]);
        spliced.push_java_str(display_name);
        spliced.push_java_str(&body[colon..]);
        spliced
    }

    /// A local variable instruction row (`ALOAD 0`, `IINC 3 1`).
    pub fn var_match(body: &JavaStr) -> Option<VarTokens<'_>> {
        let space1 = body.find(' ')?;
        let mnemonic = &body[..space1];
        if !VAR_MNEMONICS.iter().any(|&m| mnemonic == m) {
            return None;
        }
        let rest = &body[space1 + 1..];
        let slot_end = rest.find(' ').unwrap_or(rest.len());
        let slot = to_int_or_null(&rest[..slot_end])?;
        Some(VarTokens { mnemonic, slot })
    }

    /// The trivia category of `line`: LOAD, STORE, STACK, MATH, or CONST for a numeric constant.
    pub fn trivia_category_of(line: &JavaStr) -> Option<InstructionCategory> {
        let body = trim_start(line);
        let mnemonic = substring_before_space(body);
        let category = category_by_mnemonic(mnemonic)?;
        match category {
            InstructionCategory::Load
            | InstructionCategory::Store
            | InstructionCategory::Stack
            | InstructionCategory::Math => Some(category),
            InstructionCategory::Const if is_numeric_const(mnemonic, body) => Some(category),
            _ => None,
        }
    }

    /// Whether a CONST row pushes a number (any but `ACONST_NULL` and a non-numeric `LDC`).
    pub fn is_numeric_const(mnemonic: &JavaStr, body: &JavaStr) -> bool {
        if mnemonic == "ACONST_NULL" {
            false
        } else if mnemonic == "LDC" {
            let operand = trim_start(&body["LDC".len()..]);
            operand
                .chars()
                .next()
                .is_some_and(|first| matches!(first.as_char(), Some('0'..='9' | '-' | '+' | '.')))
        } else {
            true
        }
    }

    /// Whether `line` survives the `toggles` prefix filter.
    pub fn should_emit(line: &JavaStr, toggles: RenderToggles) -> bool {
        let t = trim_start(line);
        !((!toggles.include_line_numbers && t.starts_with("LINENUMBER "))
            || (!toggles.include_local_variables
                && (t.starts_with("LOCALVARIABLE ")
                    || t.starts_with("MAXSTACK ")
                    || t.starts_with("MAXLOCALS ")))
            || (!toggles.include_try_catch && t.starts_with("TRYCATCHBLOCK "))
            || (!toggles.include_frames && t.starts_with("FRAME ")))
    }

    /// sift's `categoryByMnemonic`: `Printer.OPCODES` through `opcodeCategory`.
    fn category_by_mnemonic(mnemonic: &JavaStr) -> Option<InstructionCategory> {
        if mnemonic.is_empty() {
            return None;
        }
        let opcode = OPCODES.iter().position(|&name| mnemonic == name)?;
        Some(opcode_category(u8::try_from(opcode).expect("200 opcodes")))
    }

    /// sift's `opcodeCategory`.
    fn opcode_category(opcode: u8) -> InstructionCategory {
        use InstructionCategory::*;
        match opcode {
            ACONST_NULL | ICONST_M1..=DCONST_1 | BIPUSH | SIPUSH | LDC => Const,
            ILOAD..=ALOAD | IALOAD..=SALOAD | ARRAYLENGTH => Load,
            ISTORE..=ASTORE | IASTORE..=SASTORE => Store,
            IADD..=DREM | INEG..=LXOR | IINC | LCMP..=DCMPG => Math,
            I2L..=I2S | CHECKCAST | INSTANCEOF => Cast,
            IFEQ..=RET
            | TABLESWITCH
            | LOOKUPSWITCH
            | IRETURN..=RETURN
            | ATHROW
            | IFNULL
            | IFNONNULL
            | 200
            | 201 => Control,
            GETSTATIC..=PUTFIELD => Field,
            INVOKEVIRTUAL..=INVOKEDYNAMIC => Invoke,
            NEW | NEWARRAY | ANEWARRAY | MONITORENTER | MONITOREXIT | MULTIANEWARRAY => Object,
            // NOP, POP ... SWAP, and the opcodes the table leaves at its default.
            _ => Stack,
        }
    }

    /// Kotlin's `substringBefore(' ')`.
    fn substring_before_space(s: &JavaStr) -> &JavaStr {
        s.find(' ').map_or(s, |space| &s[..space])
    }

    /// Kotlin's `String.toIntOrNull()`, with ASCII digits.
    fn to_int_or_null(s: &JavaStr) -> Option<i32> {
        let digits = s
            .strip_prefix('-')
            .or_else(|| s.strip_prefix('+'))
            .unwrap_or(s);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // Only ASCII is left, and i32's parser takes the same signs.
        s.as_str().ok()?.parse().ok()
    }

    /// Kotlin's `Char.isWhitespace()` on the JVM: `Character.isWhitespace(c) ||
    /// Character.isSpaceChar(c)`, that is the separators (Zs, Zl, Zp), U+0009 to U+000D and
    /// U+001C to U+001F.
    fn is_whitespace(c: JavaCodePoint) -> bool {
        matches!(
            c.as_u32(),
            0x09..=0x0d
                | 0x1c..=0x20
                | 0xa0
                | 0x1680
                | 0x2000..=0x200a
                | 0x2028
                | 0x2029
                | 0x202f
                | 0x205f
                | 0x3000
        )
    }

    /// Kotlin's `trimStart()`.
    pub fn trim_start(s: &JavaStr) -> &JavaStr {
        let start = s
            .char_indices()
            .find(|&(_, c)| !is_whitespace(c))
            .map_or(s.len(), |(i, _)| i);
        &s[start..]
    }

    fn trim_end(s: &JavaStr) -> &JavaStr {
        let end = s
            .char_indices()
            .rev()
            .find(|&(_, c)| !is_whitespace(c))
            .map_or(0, |(i, c)| i + c.len_semi_utf8());
        &s[..end]
    }

    fn trim(s: &JavaStr) -> &JavaStr {
        trim_end(trim_start(s))
    }
}

// -------------------------------------------------------------------------------------------------
// The probe
// -------------------------------------------------------------------------------------------------

/// The probe's payload for a class: see `siftProbe` in `oracle/Oracle.java`.
fn probe(interner: &Arc<Interner>, class_file: &[u8]) -> Result<Vec<u8>, String> {
    let reader = ClassReader::new(class_file).map_err(|e| e.to_string())?;
    let mut textified = Vec::new();
    reader
        .accept(
            &mut TraceClassVisitor::new(&mut textified),
            ParsingOptions::empty(),
        )
        .map_err(|e| e.to_string())?;
    let textified = JavaStr::from_semi_utf8(&textified).map_err(|e| e.to_string())?;

    let mut out = b"#lines\n".to_vec();
    for line in split_lines(textified) {
        probe_line(line, &mut out);
    }

    out.extend_from_slice(b"#frames\n");
    // One interner for every class, as sift shares one across a classpath.
    let mut class_node = ClassNode::with_interner(interner.clone());
    reader
        .accept(&mut class_node, ParsingOptions::empty())
        .map_err(|e| e.to_string())?;
    for method in &class_node.methods {
        out.extend_from_slice(method.resolve(method.name).as_bytes());
        out.extend_from_slice(method.resolve(method.desc).as_bytes());
        out.push(b'\n');
        analyze_method(&class_node, method, &mut out)?;
    }
    Ok(out)
}

/// One line's `trivia \t emit \t numeric \t var \t invoke \t field | line`.
fn probe_line(line: &JavaStr, out: &mut Vec<u8>) {
    // BytecodeTreeRenderer's `line.substring(line.takeWhile { it.isWhitespace() }.length)`.
    let body = textifier_lines::trim_start(line);
    let mnemonic = body.find(' ').map_or(body, |space| &body[..space]);

    let trivia = textifier_lines::trivia_category_of(line).map_or("-", InstructionCategory::name);
    let mut emit = 0u16;
    for toggles in 0..16 {
        let render_toggles = RenderToggles {
            include_line_numbers: toggles & 1 != 0,
            include_local_variables: toggles & 2 != 0,
            include_try_catch: toggles & 4 != 0,
            include_frames: toggles & 8 != 0,
        };
        if textifier_lines::should_emit(line, render_toggles) {
            emit |= 1 << toggles;
        }
    }
    let numeric = u8::from(textifier_lines::is_numeric_const(mnemonic, body));
    write!(out, "{trivia}\t{emit:04x}\t{numeric}\t").expect("writing to a vector");
    match textifier_lines::var_match(body) {
        Some(var) => {
            out.extend_from_slice(var.mnemonic.as_bytes());
            write!(out, " {}", var.slot).expect("writing to a vector");
        }
        None => out.push(b'-'),
    }
    out.push(b'\t');
    let display_name = JavaStr::from_str("<x>");
    match textifier_lines::invoke_match(body) {
        Some(tokens) => tokens_and_splice(
            &tokens,
            &textifier_lines::splice_invoke_name(body, display_name),
            out,
        ),
        None => out.push(b'-'),
    }
    out.push(b'\t');
    match textifier_lines::field_match(body) {
        Some(tokens) => tokens_and_splice(
            &tokens,
            &textifier_lines::splice_field_name(body, display_name),
            out,
        ),
        None => out.push(b'-'),
    }
    out.extend_from_slice(b" | ");
    out.extend_from_slice(line.as_bytes());
    out.push(b'\n');
}

fn tokens_and_splice(
    tokens: &textifier_lines::MatchTokens<'_>,
    spliced: &JavaString,
    out: &mut Vec<u8>,
) {
    for token in [tokens.mnemonic, tokens.owner, tokens.name, tokens.desc] {
        out.extend_from_slice(token.as_bytes());
        out.push(b'|');
    }
    out.extend_from_slice(spliced.as_bytes());
}

/// Kotlin's `lineSequence()`: lines split at `\r\n`, `\n` or `\r`, with the last (empty) line
/// after a final terminator.
fn split_lines(text: &JavaStr) -> Vec<&JavaStr> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                start = i + 1;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    lines.push(&text[start..]);
    lines
}

/// The oracle's `analyze` output for one method (as xtask's `analysis::analyze_method`), with the
/// values of [`OriginSourceInterpreter`].
fn analyze_method(
    class_node: &ClassNode,
    method: &MethodNode,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let sets = SetArena::new();
    let mut analyzer = Analyzer::new(OriginSourceInterpreter { sets: &sets });
    let frames = match analyzer.analyze(class_node.resolve(class_node.name), method) {
        Ok(frames) => frames,
        Err(e) => {
            let index = e
                .insn
                .and_then(|id| method.instructions.index_of(id))
                .map_or(-1, |i| i as i64);
            write!(out, "! {index} ").expect("writing to a vector");
            out.extend_from_slice(e.message.as_bytes());
            out.push(b'\n');
            return Ok(());
        }
    };
    let mut textifier = Textifier::new();
    let mut trace_method_visitor = TraceMethodVisitor::new(&mut textifier);
    for (i, id) in method.instructions.ids().enumerate() {
        let start = trace_method_visitor.p.get_text().len();
        method
            .instructions
            .accept_insn(id, method.interner(), &mut trace_method_visitor);
        let insn_text = text_to_java_string(&trace_method_visitor.p.get_text()[start..]);
        write!(out, "{i} ").expect("writing to a vector");
        match frames.get(i) {
            None => out.extend_from_slice(b"null"),
            Some(frame) => {
                for value in frame.locals() {
                    origin(&method.instructions, value, out);
                }
                out.push(b' ');
                for value in frame.stack() {
                    origin(&method.instructions, value, out);
                }
            }
        }
        out.extend_from_slice(b" | ");
        let lines: Vec<&[u8]> = insn_text
            .as_bytes()
            .split(|&b| b == b'\n' || b == b'\r')
            .map(java_trim)
            .filter(|line| !line.is_empty())
            .collect();
        out.extend_from_slice(&lines.join(&b' '));
        out.push(b'\n');
    }
    Ok(())
}

/// `size{sorted insn indices,p<slot>...}`.
fn origin(insns: &InsnList, value: &OriginValue<'_>, out: &mut Vec<u8>) {
    let mut indices = Vec::new();
    let mut slots = Vec::new();
    for producer in value.insns.iter() {
        match producer {
            Origin::Insn(id) => indices.push(insns.index_of(id).map_or(-1, |i| i as i64)),
            Origin::Parameter(slot) => slots.push(slot),
        }
    }
    indices.sort_unstable();
    slots.sort_unstable();
    let producers: Vec<String> = indices
        .iter()
        .map(i64::to_string)
        .chain(slots.iter().map(|slot| format!("p{slot}")))
        .collect();
    write!(out, "{}{{{}}}", value.size, producers.join(",")).expect("writing to a vector");
}

/// Java's `String.trim()` on semi-UTF-8.
fn java_trim(line: &[u8]) -> &[u8] {
    let start = line.iter().position(|&b| b > b' ').unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|&b| b > b' ')
        .map_or(start, |i| i + 1);
    &line[start..end]
}

// -------------------------------------------------------------------------------------------------
// Inputs and records
// -------------------------------------------------------------------------------------------------

/// The class files of the inputs, with the oracle's record ids, in the oracle's order.
fn inputs(args: &[String]) -> Vec<(String, PathBuf)> {
    let mut inputs = Vec::new();
    for arg in args {
        let path = Path::new(arg);
        if path.is_dir() {
            let mut files = Vec::new();
            walk(path, &mut files);
            // Java's Path.compareTo: the paths' bytes.
            files.sort_by(|a, b| a.as_os_str().as_bytes().cmp(b.as_os_str().as_bytes()));
            for file in files {
                let id = file
                    .strip_prefix(path)
                    .expect("walked from path")
                    .to_string_lossy()
                    .into_owned();
                inputs.push((id, file));
            }
        } else if path.is_file() {
            inputs.push((arg.clone(), path.to_owned()));
        } else {
            panic!("no such input: {arg}");
        }
    }
    inputs
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a readable directory") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            walk(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "class" || e == "clazz")
        {
            out.push(path);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(
        !args.is_empty(),
        "usage: sift_probe <class files or dirs...>"
    );
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let interner = Arc::new(Interner::new());
    for (id, path) in inputs(&args) {
        let class_file = std::fs::read(&path).expect("a readable class file");
        let (status, payload) = match probe(&interner, &class_file) {
            Ok(payload) => ("ok", payload),
            Err(message) => ("err", message.into_bytes()),
        };
        writeln!(out, "#oracle {status} {} {id}", payload.len()).expect("writing to stdout");
        out.write_all(&payload).expect("writing to stdout");
        out.write_all(b"\n").expect("writing to stdout");
    }
}
