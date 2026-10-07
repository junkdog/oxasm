/*
 * The oxasm oracle: runs the released ASM 9.10.1 jars over class files and prints what ASM does,
 * so that oxasm's output can be compared against it. See vault-docs/oxasm-verification.md.
 *
 * Launched from source, with no build tool (normally through `cargo xtask oracle`):
 *
 *   java --class-path asm.jar:asm-tree.jar:asm-analysis.jar:asm-commons.jar:asm-util.jar \
 *       oracle/Oracle.java <command> [options] <inputs...>
 *
 * The vendored org/objectweb/asm/test/ClassFile.java (from asm-test at tag ASM_9_10_1) is found
 * by the source launcher's multi-file mode, relative to this file.
 *
 * Inputs are class files (.class, .clazz), directories (walked recursively, in sorted order) and
 * jars. A record's id is the path of the file as given, the path relative to a directory input,
 * or "<jar>!/<entry>".
 *
 * Batch output is one record per input class, on stdout:
 *
 *   #oracle <status> <payload length in bytes> <id>\n<payload>\n
 *
 * where <status> is "ok", "err" (the payload is "<exception class>: <message>") or "skip".
 * Text payloads are encoded as semi-UTF-8: UTF-8, except that unpaired surrogates are encoded as
 * 3-byte sequences instead of being replaced. This is the encoding of oxasm's JavaStr, so the
 * output of both sides can be compared byte for byte, even for obfuscated names.
 *
 * Commands:
 *
 *   textify [--opts O]           ClassReader.accept(TraceClassVisitor(Textifier), O)
 *   tree-textify [--opts O]      as textify, but through a ClassNode: ClassReader.accept(ClassNode,
 *                                O), then ClassNode.accept(TraceClassVisitor(Textifier))
 *   dump                         new ClassFile(bytes).toString()
 *   roundtrip --mode M [--copy-pool] [--opts O] [--hierarchy F]
 *                                ClassReader -> ClassWriter(flags); M is 0, maxs or frames;
 *                                frames adds SKIP_FRAMES to the reader options. The payload is
 *                                the written class file.
 *   tree-roundtrip ...           as roundtrip, but through a ClassNode
 *   analyze --interp I [--opts O] [--hierarchy F]
 *                                Analyzer.analyze for every method, with I one of basic,
 *                                basicverifier, simpleverifier (needs --hierarchy) or source
 *   verify                       defines every input class in a fresh class loader (one per
 *                                input argument, resolving siblings from the same argument) and
 *                                forces linking; run the JVM with -XX:+UnlockDiagnosticVMOptions
 *                                -XX:+BytecodeVerificationLocal (xtask does)
 *   hierarchy                    prints "name \t super \t is_interface(0|1)" for every input
 *                                class, sorted by name (no records)
 *   serve                        persistent mode, see below
 *   gen <scenario...>            writes the T3 synthetic classes (one record per scenario: the
 *                                class file, or the exception of ClassWriter.toByteArray); oxasm's
 *                                crates/oxasm/tests/t3_synthetic_test.rs generates the same
 *   version                      prints the ASM version the oracle is pinned to
 *
 * O is a comma-separated list of SKIP_CODE, SKIP_DEBUG, SKIP_FRAMES and EXPAND_FRAMES, or 0.
 *
 * serve reads requests from stdin until EOF, and answers each one on stdout, in order:
 *
 *   request:  u32 n, n bytes of UTF-8 command line ("textify --opts SKIP_DEBUG"),
 *             u32 m, m bytes of class file
 *   response: u8 status (0 ok, 1 err), u32 k, k bytes of payload
 *
 * All integers are big-endian. Only the per-class commands (textify, tree-textify, dump,
 * roundtrip, tree-roundtrip, analyze) are served.
 *
 * The hierarchy model (--hierarchy F, a file in the format printed by `hierarchy`) replaces class
 * loading for COMPUTE_FRAMES and SimpleVerifier, so that results never depend on which classes
 * this JVM can load. It knows each class's super class and whether it is an interface, and
 * nothing about implemented interfaces. This is exactly what oxasm's ClassHierarchy trait (ADR-07)
 * exposes, and oxasm's MapHierarchy must answer identically:
 *
 *   - a is assignable from b iff a == b, a == java/lang/Object, or a is on b's super class chain;
 *   - common_super_class(a, b) is ClassWriter.getCommonSuperClass over that relation;
 *   - a name missing from F throws TypeNotPresentException(name), as class loading would.
 */

import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.function.Function;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.test.ClassFile;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.ClassNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.analysis.Analyzer;
import org.objectweb.asm.tree.analysis.AnalyzerException;
import org.objectweb.asm.tree.analysis.BasicInterpreter;
import org.objectweb.asm.tree.analysis.BasicValue;
import org.objectweb.asm.tree.analysis.BasicVerifier;
import org.objectweb.asm.tree.analysis.Frame;
import org.objectweb.asm.tree.analysis.Interpreter;
import org.objectweb.asm.tree.analysis.SimpleVerifier;
import org.objectweb.asm.tree.analysis.SourceInterpreter;
import org.objectweb.asm.tree.analysis.SourceValue;
import org.objectweb.asm.tree.analysis.Value;
import org.objectweb.asm.util.Textifier;
import org.objectweb.asm.util.TraceClassVisitor;
import org.objectweb.asm.util.TraceMethodVisitor;

public final class Oracle {

  /** The ASM release this oracle is pinned to; xtask checks it against the jars it passes. */
  static final String ASM_VERSION = "9.10.1";

  private Oracle() {}

  public static void main(final String[] args) throws Exception {
    if (args.length == 0) {
      usage("missing command");
    }
    String command = args[0];
    Args options = Args.parse(args, 1);
    OutputStream out = new BufferedOutputStream(new FileOutputStream(FileDescriptor.out), 1 << 16);
    try {
      switch (command) {
        case "version" -> out.write((ASM_VERSION + "\n").getBytes(StandardCharsets.UTF_8));
        case "serve" -> serve(new DataInputStream(System.in), new DataOutputStream(out));
        case "hierarchy" -> hierarchy(options, out);
        case "verify" -> verify(options, out);
        case "gen" -> gen(options, out);
        default -> {
          ClassCommand classCommand = ClassCommand.create(command, options);
          forEachInput(
              options.inputs,
              input -> {
                byte[] payload;
                String status;
                try {
                  payload = classCommand.apply(input.bytes());
                  status = "ok";
                } catch (Throwable t) { // NOPMD: the oracle reports whatever ASM throws.
                  payload = utf8(describe(t));
                  status = "err";
                }
                writeRecord(out, status, input.id(), payload);
              });
        }
      }
    } catch (UsageException e) {
      out.flush();
      usage(e.getMessage());
    }
    out.flush();
  }

  private static void usage(final String message) {
    System.err.println("oracle: " + message);
    System.err.println(
        "usage: Oracle {textify|tree-textify|dump|roundtrip|tree-roundtrip|analyze|verify|hierarchy|gen|serve"
            + "|version} [options] <inputs...>");
    System.exit(2);
  }

  // -----------------------------------------------------------------------------------------------
  // Per-class commands
  // -----------------------------------------------------------------------------------------------

  /** A command that maps one class file to one payload. */
  @FunctionalInterface
  interface ClassCommand {

    byte[] apply(byte[] classFile) throws Exception;

    static ClassCommand create(final String command, final Args args) {
      return switch (command) {
        case "textify", "tree-textify" -> {
          args.allow("--opts");
          boolean tree = command.equals("tree-textify");
          int opts = parseReaderOptions(args.get("--opts"));
          yield classFile -> textify(classFile, tree, opts);
        }
        case "dump" -> {
          args.allow();
          yield classFile -> semiUtf8(new ClassFile(classFile).toString());
        }
        case "roundtrip", "tree-roundtrip" -> {
          args.allow("--mode", "--opts", "--hierarchy", "--copy-pool");
          boolean tree = command.equals("tree-roundtrip");
          String mode = args.require("--mode");
          int flags =
              switch (mode) {
                case "0" -> 0;
                case "maxs" -> ClassWriter.COMPUTE_MAXS;
                case "frames" -> ClassWriter.COMPUTE_FRAMES;
                default -> throw new UsageException("unknown --mode " + mode);
              };
          int opts = parseReaderOptions(args.get("--opts"));
          if (flags == ClassWriter.COMPUTE_FRAMES) {
            opts |= ClassReader.SKIP_FRAMES;
          }
          int readerOptions = opts;
          boolean copyPool = args.flag("--copy-pool");
          Hierarchy hierarchy = Hierarchy.load(args.get("--hierarchy"));
          yield classFile -> roundtrip(classFile, tree, flags, readerOptions, copyPool, hierarchy);
        }
        case "analyze" -> {
          args.allow("--interp", "--opts", "--hierarchy");
          String interp = args.require("--interp");
          int opts = parseReaderOptions(args.get("--opts"));
          Hierarchy hierarchy = Hierarchy.load(args.get("--hierarchy"));
          switch (interp) {
            case "basic", "basicverifier", "source" -> {}
            case "simpleverifier" -> {
              if (hierarchy == null) {
                throw new UsageException("--interp simpleverifier needs --hierarchy");
              }
            }
            default -> throw new UsageException("unknown --interp " + interp);
          }
          yield classFile -> analyze(classFile, interp, opts, hierarchy);
        }
        default -> throw new UsageException("unknown command " + command);
      };
    }
  }

  static byte[] textify(final byte[] classFile, final boolean tree, final int readerOptions) {
    StringWriter text = new StringWriter();
    PrintWriter printWriter = new PrintWriter(text);
    TraceClassVisitor traceClassVisitor = new TraceClassVisitor(null, new Textifier(), printWriter);
    if (tree) {
      ClassNode classNode = new ClassNode();
      new ClassReader(classFile).accept(classNode, readerOptions);
      classNode.accept(traceClassVisitor);
    } else {
      new ClassReader(classFile).accept(traceClassVisitor, readerOptions);
    }
    printWriter.flush();
    return semiUtf8(text.toString());
  }

  static byte[] roundtrip(
      final byte[] classFile,
      final boolean tree,
      final int writerFlags,
      final int readerOptions,
      final boolean copyPool,
      final Hierarchy hierarchy) {
    ClassReader classReader = new ClassReader(classFile);
    ClassWriter classWriter =
        new HierarchyClassWriter(copyPool ? classReader : null, writerFlags, hierarchy);
    if (tree) {
      ClassNode classNode = new ClassNode();
      classReader.accept(classNode, readerOptions);
      classNode.accept(classWriter);
    } else {
      classReader.accept(classWriter, readerOptions);
    }
    return classWriter.toByteArray();
  }

  /** A ClassWriter whose getCommonSuperClass resolves through a {@link Hierarchy}. */
  static final class HierarchyClassWriter extends ClassWriter {

    private final Hierarchy hierarchy;

    HierarchyClassWriter(final ClassReader classReader, final int flags, final Hierarchy hierarchy) {
      super(classReader, flags);
      this.hierarchy = hierarchy;
    }

    @Override
    protected String getCommonSuperClass(final String type1, final String type2) {
      if (hierarchy == null) {
        throw new IllegalStateException(
            "no --hierarchy to compute the common super class of " + type1 + " and " + type2);
      }
      return hierarchy.commonSuperClass(type1, type2);
    }
  }

  // -----------------------------------------------------------------------------------------------
  // analyze
  // -----------------------------------------------------------------------------------------------

  /**
   * Prints, for each method, "name+descriptor", then either "! <insn index> <message>" if the
   * analysis failed, or one "<insn index> <frame> | <insn text>" line per instruction, where
   * <frame> is Frame.toString() ("null" for unreachable instructions). SourceValues print as
   * "<size>{<sorted insn indices>}" because SourceValue has no toString, and the iteration order
   * of its HashSet is not deterministic. <insn text> is the instruction as printed by a Textifier
   * shared by the whole method (so labels are named L0, L1, ... in instruction order), with its
   * lines trimmed and joined by a space.
   */
  static byte[] analyze(
      final byte[] classFile, final String interp, final int readerOptions, final Hierarchy h) {
    ClassNode classNode = new ClassNode();
    new ClassReader(classFile).accept(classNode, readerOptions);
    StringBuilder text = new StringBuilder();
    for (MethodNode method : classNode.methods) {
      text.append(method.name).append(method.desc).append('\n');
      switch (interp) {
        case "basic" -> analyzeMethod(classNode, method, new BasicInterpreter(), null, text);
        case "basicverifier" -> analyzeMethod(classNode, method, new BasicVerifier(), null, text);
        case "simpleverifier" ->
            analyzeMethod(classNode, method, new HierarchySimpleVerifier(classNode, h), null, text);
        case "source" -> {
          InsnList insns = method.instructions;
          Function<SourceValue, String> format =
              value ->
                  value.size
                      + value.insns.stream()
                          .map(insns::indexOf)
                          .sorted()
                          .map(String::valueOf)
                          .collect(Collectors.joining(",", "{", "}"));
          analyzeMethod(classNode, method, new SourceInterpreter(), format, text);
        }
        default -> throw new IllegalStateException(interp);
      }
    }
    return semiUtf8(text.toString());
  }

  private static <V extends Value> void analyzeMethod(
      final ClassNode classNode,
      final MethodNode method,
      final Interpreter<V> interpreter,
      final Function<V, String> valueFormat,
      final StringBuilder text) {
    Frame<V>[] frames;
    try {
      frames = new Analyzer<>(interpreter).analyze(classNode.name, method);
    } catch (AnalyzerException e) {
      int index = e.node == null ? -1 : method.instructions.indexOf(e.node);
      text.append("! ").append(index).append(' ').append(e.getMessage()).append('\n');
      return;
    }
    Textifier textifier = new Textifier();
    TraceMethodVisitor traceMethodVisitor = new TraceMethodVisitor(textifier);
    List<Object> printed = textifier.getText();
    for (int i = 0; i < method.instructions.size(); ++i) {
      AbstractInsnNode insn = method.instructions.get(i);
      int start = printed.size();
      insn.accept(traceMethodVisitor);
      StringBuilder insnText = new StringBuilder();
      for (Object item : printed.subList(start, printed.size())) {
        flatten(item, insnText);
      }
      String insnLine =
          insnText
              .toString()
              .lines()
              .map(String::trim)
              .filter(line -> !line.isEmpty())
              .collect(Collectors.joining(" "));
      // An abstract or native method has no frames, even if a malformed class gives it code.
      Frame<V> frame = i < frames.length ? frames[i] : null;
      text.append(i).append(' ');
      if (frame == null) {
        text.append("null");
      } else if (valueFormat == null) {
        text.append(frame);
      } else {
        // Same layout as Frame.toString(), with a custom value format.
        for (int j = 0; j < frame.getLocals(); ++j) {
          text.append(valueFormat.apply(frame.getLocal(j)));
        }
        text.append(' ');
        for (int j = 0; j < frame.getStackSize(); ++j) {
          text.append(valueFormat.apply(frame.getStack(j)));
        }
      }
      text.append(" | ").append(insnLine).append('\n');
    }
  }

  private static void flatten(final Object item, final StringBuilder out) {
    if (item instanceof List<?> list) {
      for (Object child : list) {
        flatten(child, out);
      }
    } else {
      out.append(item);
    }
  }

  /** A SimpleVerifier that resolves classes through a {@link Hierarchy} instead of loading them. */
  static final class HierarchySimpleVerifier extends SimpleVerifier {

    private final Type currentClass;
    private final Type currentSuperClass;
    private final List<Type> currentClassInterfaces;
    private final boolean isInterface;
    private final Hierarchy hierarchy;

    HierarchySimpleVerifier(final ClassNode classNode, final Hierarchy hierarchy) {
      this(
          Type.getObjectType(classNode.name),
          classNode.superName == null ? null : Type.getObjectType(classNode.superName),
          classNode.interfaces.stream().map(Type::getObjectType).toList(),
          (classNode.access & Opcodes.ACC_INTERFACE) != 0,
          hierarchy);
    }

    private HierarchySimpleVerifier(
        final Type currentClass,
        final Type currentSuperClass,
        final List<Type> currentClassInterfaces,
        final boolean isInterface,
        final Hierarchy hierarchy) {
      super(Opcodes.ASM9, currentClass, currentSuperClass, currentClassInterfaces, isInterface);
      this.currentClass = currentClass;
      this.currentSuperClass = currentSuperClass;
      this.currentClassInterfaces = currentClassInterfaces;
      this.isInterface = isInterface;
      this.hierarchy = hierarchy;
    }

    @Override
    protected boolean isInterface(final Type type) {
      if (currentClass.equals(type)) {
        return isInterface;
      }
      return hierarchy.isInterface(type.getInternalName());
    }

    @Override
    protected Type getSuperClass(final Type type) {
      if (currentClass.equals(type)) {
        return currentSuperClass;
      }
      String superName = hierarchy.superName(type.getInternalName());
      return superName == null ? null : Type.getObjectType(superName);
    }

    /** SimpleVerifier.isAssignableFrom, with the final class loading step replaced. */
    @Override
    protected boolean isAssignableFrom(final Type type1, final Type type2) {
      if (type1.equals(type2)) {
        return true;
      }
      if (currentClass.equals(type1)) {
        Type superType2 = getSuperClass(type2);
        if (superType2 == null) {
          return false;
        }
        if (isInterface) {
          return type2.getSort() == Type.OBJECT || type2.getSort() == Type.ARRAY;
        }
        return isAssignableFrom(type1, superType2);
      }
      if (currentClass.equals(type2)) {
        if (isAssignableFrom(type1, currentSuperClass)) {
          return true;
        }
        for (Type currentClassInterface : currentClassInterfaces) {
          if (isAssignableFrom(type1, currentClassInterface)) {
            return true;
          }
        }
        return false;
      }
      return hierarchy.isAssignableFrom(type1.getInternalName(), type2.getInternalName());
    }

    /**
     * Only reached from SimpleVerifier.isSubTypeOf, as getClass(expected).isInterface() and
     * Object.class.isAssignableFrom(getClass(type)). Returns a stand-in class with the same
     * isInterface() answer, after checking that the class exists.
     */
    @Override
    protected Class<?> getClass(final Type type) {
      return hierarchy.isInterface(type.getInternalName()) ? Runnable.class : Object.class;
    }
  }

  // -----------------------------------------------------------------------------------------------
  // Hierarchy model
  // -----------------------------------------------------------------------------------------------

  /** The class hierarchy read from a hierarchy.tsv file. See the file comment for its model. */
  static final class Hierarchy {

    private static final String OBJECT = "java/lang/Object";
    private static final int MAX_DEPTH = 1 << 16;
    private static final Map<String, Hierarchy> LOADED = new HashMap<>();

    private record Entry(String superName, boolean isInterface) {}

    private final Map<String, Entry> entries = new HashMap<>();

    static synchronized Hierarchy load(final String path) {
      if (path == null) {
        return null;
      }
      return LOADED.computeIfAbsent(path, Hierarchy::read);
    }

    private static Hierarchy read(final String path) {
      Hierarchy hierarchy = new Hierarchy();
      try {
        for (String line : Files.readAllLines(Path.of(path), StandardCharsets.UTF_8)) {
          if (line.isEmpty()) {
            continue;
          }
          String[] fields = line.split("\t", -1);
          if (fields.length != 3) {
            throw new UsageException("malformed hierarchy line: " + line);
          }
          hierarchy.entries.putIfAbsent(
              fields[0],
              new Entry(fields[1].isEmpty() ? null : fields[1], fields[2].equals("1")));
        }
      } catch (IOException e) {
        throw new UsageException("cannot read " + path + ": " + e);
      }
      return hierarchy;
    }

    private Entry entry(final String name) {
      Entry entry = entries.get(name);
      if (entry == null) {
        throw new TypeNotPresentException(name, null);
      }
      return entry;
    }

    String superName(final String name) {
      return entry(name).superName();
    }

    boolean isInterface(final String name) {
      return entry(name).isInterface();
    }

    boolean isAssignableFrom(final String type1, final String type2) {
      entry(type1);
      entry(type2);
      if (type1.equals(type2) || type1.equals(OBJECT)) {
        return true;
      }
      String type = type2;
      for (int depth = 0; type != null && depth < MAX_DEPTH; ++depth) {
        if (type.equals(type1)) {
          return true;
        }
        type = superName(type);
      }
      return false;
    }

    /** ClassWriter.getCommonSuperClass, over this model. */
    String commonSuperClass(final String type1, final String type2) {
      if (isAssignableFrom(type1, type2)) {
        return type1;
      }
      if (isAssignableFrom(type2, type1)) {
        return type2;
      }
      if (isInterface(type1) || isInterface(type2)) {
        return OBJECT;
      }
      String type = type1;
      int depth = 0;
      do {
        type = superName(type);
        if (type == null || ++depth > MAX_DEPTH) {
          throw new IllegalStateException("broken super class chain from " + type1);
        }
      } while (!isAssignableFrom(type, type2));
      return type;
    }
  }

  static void hierarchy(final Args args, final OutputStream out) throws IOException {
    args.allow();
    TreeMap<String, String> lines = new TreeMap<>();
    forEachInput(
        args.inputs,
        input -> {
          try {
            ClassReader classReader = new ClassReader(input.bytes());
            int access = classReader.getAccess();
            if ((access & Opcodes.ACC_MODULE) != 0) {
              return;
            }
            String superName = classReader.getSuperName();
            lines.putIfAbsent(
                classReader.getClassName(),
                (superName == null ? "" : superName)
                    + '\t'
                    + ((access & Opcodes.ACC_INTERFACE) != 0 ? '1' : '0'));
          } catch (RuntimeException e) {
            System.err.println("oracle: skipping " + input.id() + ": " + describe(e));
          }
        });
    for (Map.Entry<String, String> line : lines.entrySet()) {
      out.write(semiUtf8(line.getKey() + '\t' + line.getValue() + '\n'));
    }
  }

  // -----------------------------------------------------------------------------------------------
  // verify
  // -----------------------------------------------------------------------------------------------

  static void verify(final Args args, final OutputStream out) throws IOException {
    args.allow();
    List<LinkedHashMap<String, Input>> groups = new ArrayList<>();
    List<Input> unreadable = new ArrayList<>();
    forEachInput(
        args.inputs,
        input -> {
          while (groups.size() <= input.group()) {
            groups.add(new LinkedHashMap<>());
          }
          try {
            String name = new ClassReader(input.bytes()).getClassName().replace('/', '.');
            groups.get(input.group()).putIfAbsent(name, input);
          } catch (RuntimeException e) {
            unreadable.add(input);
          }
        });
    for (Input input : unreadable) {
      writeRecord(out, "err", input.id(), utf8("unreadable class file"));
    }
    for (LinkedHashMap<String, Input> group : groups) {
      GroupClassLoader loader = new GroupClassLoader(group);
      for (Map.Entry<String, Input> entry : group.entrySet()) {
        String name = entry.getKey();
        String id = entry.getValue().id();
        if (name.equals("module-info") || name.endsWith(".module-info")) {
          writeRecord(out, "skip", id, utf8("module-info cannot be defined"));
          continue;
        }
        try {
          Class<?> c = Class.forName(name, false, loader);
          // Reflecting on the methods links the class, and so verifies it, without running
          // <clinit>.
          c.getDeclaredMethods();
          writeRecord(out, "ok", id, utf8("ok"));
        } catch (Throwable t) { // NOPMD: report whatever the JVM throws.
          writeRecord(out, "err", id, utf8(describe(t)));
        }
      }
    }
  }

  /** Defines the classes of one input argument, and nothing else beyond the platform. */
  static final class GroupClassLoader extends ClassLoader {

    private final Map<String, Input> classes;

    GroupClassLoader(final Map<String, Input> classes) {
      super("oracle-verify", ClassLoader.getPlatformClassLoader());
      this.classes = classes;
    }

    @Override
    protected Class<?> findClass(final String name) throws ClassNotFoundException {
      Input input = classes.get(name);
      if (input == null) {
        throw new ClassNotFoundException(name);
      }
      byte[] bytes = input.bytes();
      return defineClass(name, bytes, 0, bytes.length);
    }
  }

  // -----------------------------------------------------------------------------------------------
  // gen: the T3 synthetic classes (keep in sync with crates/oxasm/tests/t3_synthetic_test.rs)
  // -----------------------------------------------------------------------------------------------

  static void gen(final Args args, final OutputStream out) throws IOException {
    args.allow();
    for (String scenario : args.inputs) {
      byte[] payload;
      String status;
      try {
        payload = genScenario(scenario);
        status = "ok";
      } catch (UsageException e) {
        throw e;
      } catch (Throwable t) { // NOPMD: the oracle reports whatever ASM throws.
        payload = utf8(describe(t));
        status = "err";
      }
      writeRecord(out, status, scenario, payload);
    }
  }

  /** The number of NOPs that makes a jump over them too far for a 2 bytes offset. */
  static final int FAR = 40000;

  static byte[] genScenario(final String scenario) {
    switch (scenario) {
      case "jump-forward-v5":
        return jumpForward(Opcodes.V1_5, false);
      case "jump-forward-v8":
        return jumpForward(Opcodes.V1_8, true);
      case "jump-backward-v5":
        return jumpBackward();
      case "jump-forward-locals-v8":
        return jumpForwardLocals();
      case "method-too-large":
        {
          ClassWriter classWriter = newClass(Opcodes.V1_8, "t3/MethodTooLarge");
          MethodVisitor methodVisitor =
              classWriter.visitMethod(Opcodes.ACC_STATIC, "m", "()V", null, null);
          methodVisitor.visitCode();
          for (int i = 0; i < 70000; ++i) {
            methodVisitor.visitInsn(Opcodes.NOP);
          }
          methodVisitor.visitInsn(Opcodes.RETURN);
          methodVisitor.visitMaxs(0, 0);
          methodVisitor.visitEnd();
          return classWriter.toByteArray();
        }
      case "class-too-large":
        {
          ClassWriter classWriter = newClass(Opcodes.V1_8, "t3/ClassTooLarge");
          for (int i = 0; i < 65535; ++i) {
            classWriter.newConst(i);
          }
          return classWriter.toByteArray();
        }
      case "pool-65535":
        {
          ClassWriter classWriter = newClass(Opcodes.V1_8, "t3/Pool65535");
          int i = 0;
          while (classWriter.newConst(i) < 65534) {
            ++i;
          }
          return classWriter.toByteArray();
        }
      default:
        throw new UsageException("unknown scenario " + scenario);
    }
  }

  static ClassWriter newClass(final int version, final String name) {
    ClassWriter classWriter = new ClassWriter(0);
    classWriter.visit(version, Opcodes.ACC_PUBLIC, name, null, "java/lang/Object", null);
    return classWriter;
  }

  /** static m(I)V: IFEQ and GOTO forward over FAR NOPs (with frames if requested). */
  static byte[] jumpForward(final int version, final boolean frames) {
    ClassWriter classWriter = newClass(version, version == Opcodes.V1_5 ? "t3/JumpForward5" : "t3/JumpForward8");
    MethodVisitor methodVisitor = classWriter.visitMethod(Opcodes.ACC_STATIC, "m", "(I)V", null, null);
    methodVisitor.visitCode();
    Label target = new Label();
    Label end = new Label();
    methodVisitor.visitVarInsn(Opcodes.ILOAD, 0);
    methodVisitor.visitJumpInsn(Opcodes.IFEQ, target);
    methodVisitor.visitJumpInsn(Opcodes.GOTO, end);
    methodVisitor.visitLabel(target);
    if (frames) {
      methodVisitor.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
    }
    for (int i = 0; i < FAR; ++i) {
      methodVisitor.visitInsn(Opcodes.NOP);
    }
    methodVisitor.visitLabel(end);
    if (frames) {
      methodVisitor.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
    }
    methodVisitor.visitInsn(Opcodes.RETURN);
    methodVisitor.visitMaxs(1, 1);
    methodVisitor.visitEnd();
    return classWriter.toByteArray();
  }

  /** static m(I)V: IFNE and GOTO backward over FAR NOPs (GOTO_W, and IFEQ + GOTO_W). */
  static byte[] jumpBackward() {
    ClassWriter classWriter = newClass(Opcodes.V1_5, "t3/JumpBackward5");
    MethodVisitor methodVisitor = classWriter.visitMethod(Opcodes.ACC_STATIC, "m", "(I)V", null, null);
    methodVisitor.visitCode();
    Label start = new Label();
    Label end = new Label();
    methodVisitor.visitLabel(start);
    methodVisitor.visitVarInsn(Opcodes.ILOAD, 0);
    methodVisitor.visitJumpInsn(Opcodes.IFLT, end);
    for (int i = 0; i < FAR; ++i) {
      methodVisitor.visitInsn(Opcodes.NOP);
    }
    methodVisitor.visitIincInsn(0, -1);
    methodVisitor.visitVarInsn(Opcodes.ILOAD, 0);
    methodVisitor.visitJumpInsn(Opcodes.IFNE, start);
    methodVisitor.visitJumpInsn(Opcodes.GOTO, start);
    methodVisitor.visitLabel(end);
    methodVisitor.visitInsn(Opcodes.RETURN);
    methodVisitor.visitMaxs(1, 1);
    methodVisitor.visitEnd();
    return classWriter.toByteArray();
  }

  /**
   * m(ILjava/lang/String;)J, V1_8: a forward IFNULL over FAR bytes of instructions that change the
   * locals and the stack, then expanded frames, so that the inserted frames of the re-read are
   * computed by simulating these instructions.
   */
  static byte[] jumpForwardLocals() {
    ClassWriter classWriter = newClass(Opcodes.V1_8, "t3/JumpForwardLocals8");
    MethodVisitor methodVisitor =
        classWriter.visitMethod(0, "m", "(ILjava/lang/String;)J", null, null);
    methodVisitor.visitCode();
    Label target = new Label();
    methodVisitor.visitInsn(Opcodes.LCONST_1);
    methodVisitor.visitVarInsn(Opcodes.LSTORE, 3);
    methodVisitor.visitTypeInsn(Opcodes.NEW, "java/lang/StringBuilder");
    methodVisitor.visitInsn(Opcodes.DUP);
    methodVisitor.visitMethodInsn(
        Opcodes.INVOKESPECIAL, "java/lang/StringBuilder", "<init>", "()V", false);
    methodVisitor.visitVarInsn(Opcodes.ASTORE, 5);
    methodVisitor.visitVarInsn(Opcodes.ALOAD, 2);
    methodVisitor.visitJumpInsn(Opcodes.IFNULL, target);
    for (int i = 0; i < FAR / 8; ++i) {
      methodVisitor.visitVarInsn(Opcodes.ALOAD, 5);
      methodVisitor.visitVarInsn(Opcodes.ALOAD, 2);
      methodVisitor.visitMethodInsn(
          Opcodes.INVOKEVIRTUAL,
          "java/lang/StringBuilder",
          "append",
          "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
          false);
      methodVisitor.visitInsn(Opcodes.POP);
    }
    methodVisitor.visitVarInsn(Opcodes.LLOAD, 3);
    methodVisitor.visitInsn(Opcodes.L2I);
    methodVisitor.visitVarInsn(Opcodes.ISTORE, 1);
    methodVisitor.visitLabel(target);
    methodVisitor.visitFrame(
        Opcodes.F_NEW,
        5,
        new Object[] {
          "t3/JumpForwardLocals8", Opcodes.INTEGER, "java/lang/String", Opcodes.LONG,
          "java/lang/StringBuilder"
        },
        0,
        null);
    methodVisitor.visitVarInsn(Opcodes.LLOAD, 3);
    methodVisitor.visitInsn(Opcodes.LRETURN);
    methodVisitor.visitMaxs(3, 6);
    methodVisitor.visitEnd();
    return classWriter.toByteArray();
  }

  // -----------------------------------------------------------------------------------------------
  // serve
  // -----------------------------------------------------------------------------------------------

  static void serve(final DataInputStream in, final DataOutputStream out) throws IOException {
    Map<String, ClassCommand> commands = new HashMap<>();
    while (true) {
      int commandLength;
      try {
        commandLength = in.readInt();
      } catch (EOFException e) {
        break;
      }
      String commandLine = new String(readExactly(in, commandLength), StandardCharsets.UTF_8);
      byte[] classFile = readExactly(in, in.readInt());
      byte[] payload;
      int status;
      try {
        ClassCommand command = commands.get(commandLine);
        if (command == null) {
          String[] words = commandLine.trim().split("\\s+");
          Args args = Args.parse(words, 1);
          if (!args.inputs.isEmpty()) {
            throw new UsageException("serve requests take no inputs: " + commandLine);
          }
          command = ClassCommand.create(words[0], args);
          commands.put(commandLine, command);
        }
        payload = command.apply(classFile);
        status = 0;
      } catch (Throwable t) { // NOPMD: the oracle reports whatever ASM throws.
        payload = utf8(describe(t));
        status = 1;
      }
      out.writeByte(status);
      out.writeInt(payload.length);
      out.write(payload);
      if (in.available() == 0) {
        out.flush();
      }
    }
    out.flush();
  }

  private static byte[] readExactly(final DataInputStream in, final int length)
      throws IOException {
    if (length < 0) {
      throw new IOException("negative frame length " + length);
    }
    byte[] bytes = new byte[length];
    in.readFully(bytes);
    return bytes;
  }

  // -----------------------------------------------------------------------------------------------
  // Inputs, records and options
  // -----------------------------------------------------------------------------------------------

  /** One class file; group is the index of the input argument it came from. */
  record Input(String id, int group, byte[] bytes) {}

  @FunctionalInterface
  interface InputConsumer {
    void accept(Input input) throws IOException;
  }

  static void forEachInput(final List<String> paths, final InputConsumer consumer)
      throws IOException {
    if (paths.isEmpty()) {
      throw new UsageException("no inputs");
    }
    for (int group = 0; group < paths.size(); ++group) {
      String arg = paths.get(group);
      Path path = Path.of(arg);
      if (Files.isDirectory(path)) {
        List<Path> files;
        try (Stream<Path> walk = Files.walk(path)) {
          files = walk.filter(Files::isRegularFile).filter(Oracle::isClassFile).sorted().toList();
        }
        for (Path file : files) {
          String id = path.relativize(file).toString().replace('\\', '/');
          consumer.accept(new Input(id, group, Files.readAllBytes(file)));
        }
      } else if (arg.endsWith(".jar") || arg.endsWith(".zip")) {
        try (ZipFile zip = new ZipFile(path.toFile())) {
          List<? extends ZipEntry> entries =
              zip.stream()
                  .filter(e -> !e.isDirectory() && e.getName().endsWith(".class"))
                  .sorted((a, b) -> a.getName().compareTo(b.getName()))
                  .toList();
          for (ZipEntry entry : entries) {
            byte[] bytes;
            try (var stream = zip.getInputStream(entry)) {
              bytes = stream.readAllBytes();
            }
            consumer.accept(new Input(arg + "!/" + entry.getName(), group, bytes));
          }
        }
      } else if (Files.isRegularFile(path)) {
        consumer.accept(new Input(arg, group, Files.readAllBytes(path)));
      } else {
        throw new UsageException("no such input: " + arg);
      }
    }
  }

  private static boolean isClassFile(final Path path) {
    String name = path.getFileName().toString();
    return name.endsWith(".class") || name.endsWith(".clazz");
  }

  static void writeRecord(
      final OutputStream out, final String status, final String id, final byte[] payload)
      throws IOException {
    out.write(semiUtf8("#oracle " + status + " " + payload.length + " " + id + "\n"));
    out.write(payload);
    out.write('\n');
  }

  static String describe(final Throwable t) {
    return t.getClass().getName() + ": " + t.getMessage();
  }

  static int parseReaderOptions(final String value) {
    if (value == null || value.equals("0")) {
      return 0;
    }
    int opts = 0;
    for (String name : value.split(",")) {
      opts |=
          switch (name.trim()) {
            case "SKIP_CODE" -> ClassReader.SKIP_CODE;
            case "SKIP_DEBUG" -> ClassReader.SKIP_DEBUG;
            case "SKIP_FRAMES" -> ClassReader.SKIP_FRAMES;
            case "EXPAND_FRAMES" -> ClassReader.EXPAND_FRAMES;
            default -> throw new UsageException("unknown reader option " + name);
          };
    }
    return opts;
  }

  private static byte[] utf8(final String text) {
    return text.getBytes(StandardCharsets.UTF_8);
  }

  /** Encodes text as UTF-8, except that unpaired surrogates become 3-byte sequences. */
  static byte[] semiUtf8(final String text) {
    int length = text.length();
    var out = new java.io.ByteArrayOutputStream(length + 16);
    for (int i = 0; i < length; ++i) {
      char c = text.charAt(i);
      if (c < 0x80) {
        out.write(c);
      } else if (c < 0x800) {
        out.write(0xC0 | (c >> 6));
        out.write(0x80 | (c & 0x3F));
      } else if (Character.isHighSurrogate(c)
          && i + 1 < length
          && Character.isLowSurrogate(text.charAt(i + 1))) {
        int codePoint = Character.toCodePoint(c, text.charAt(++i));
        out.write(0xF0 | (codePoint >> 18));
        out.write(0x80 | ((codePoint >> 12) & 0x3F));
        out.write(0x80 | ((codePoint >> 6) & 0x3F));
        out.write(0x80 | (codePoint & 0x3F));
      } else {
        out.write(0xE0 | (c >> 12));
        out.write(0x80 | ((c >> 6) & 0x3F));
        out.write(0x80 | (c & 0x3F));
      }
    }
    return out.toByteArray();
  }

  static final class UsageException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    UsageException(final String message) {
      super(message);
    }
  }

  /** Command line options: "--name value" pairs, "--flag"s, and positional inputs. */
  static final class Args {

    private static final Set<String> FLAGS = Set.of("--copy-pool");

    final Map<String, String> values = new LinkedHashMap<>();
    final List<String> inputs = new ArrayList<>();

    static Args parse(final String[] words, final int start) {
      Args args = new Args();
      for (int i = start; i < words.length; ++i) {
        String word = words[i];
        if (word.startsWith("--")) {
          if (FLAGS.contains(word)) {
            args.values.put(word, "true");
          } else if (i + 1 < words.length) {
            args.values.put(word, words[++i]);
          } else {
            throw new UsageException("missing value for " + word);
          }
        } else {
          args.inputs.add(word);
        }
      }
      return args;
    }

    void allow(final String... names) {
      List<String> allowed = List.of(names);
      for (String name : values.keySet()) {
        if (!allowed.contains(name)) {
          throw new UsageException("unexpected option " + name);
        }
      }
    }

    String get(final String name) {
      return values.get(name);
    }

    String require(final String name) {
      String value = values.get(name);
      if (value == null) {
        throw new UsageException("missing " + name);
      }
      return value;
    }

    boolean flag(final String name) {
      return values.containsKey(name);
    }
  }
}
