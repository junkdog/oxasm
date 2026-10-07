// Produces asmifier_hand_driven.txt with ASM 9.10.1's ASMifier, from the same events as
// asmifier_test.rs's test_hand_driven_class. Run with the oracle's ASM jars:
//
//   java -cp asm-9.10.1.jar:asm-util-9.10.1.jar:asm-tree-9.10.1.jar:asm-analysis-9.10.1.jar \
//       AsmifierHand.java > asmifier_hand_driven.txt

import java.io.PrintWriter;
import java.util.Map;
import org.objectweb.asm.AnnotationVisitor;
import org.objectweb.asm.Attribute;
import org.objectweb.asm.ConstantDynamic;
import org.objectweb.asm.FieldVisitor;
import org.objectweb.asm.Handle;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.ModuleVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.RecordComponentVisitor;
import org.objectweb.asm.Type;
import org.objectweb.asm.TypePath;
import org.objectweb.asm.TypeReference;
import org.objectweb.asm.util.ASMifier;
import org.objectweb.asm.util.ASMifierSupport;
import org.objectweb.asm.util.TraceClassVisitor;

public class AsmifierHand implements Opcodes {

  static class Comment extends Attribute implements ASMifierSupport {
    Comment() {
      super("Comment");
    }

    @Override
    public void asmify(
        final StringBuilder sb, final String varName, final Map<Label, String> labelNames) {
      sb.append("Attribute ").append(varName).append(" = new org.objectweb.asm.util.Comment();");
    }
  }

  static class CodeComment extends Attribute implements ASMifierSupport {
    final Label a;
    final Label b;

    CodeComment(final Label a, final Label b) {
      super("CodeComment");
      this.a = a;
      this.b = b;
    }

    @Override
    public boolean isCodeAttribute() {
      return true;
    }

    private static String name(final Map<Label, String> labelNames, final Label label) {
      String name = labelNames.get(label);
      if (name == null) {
        name = "label" + labelNames.size();
        labelNames.put(label, name);
      }
      return name;
    }

    @Override
    public void asmify(
        final StringBuilder sb, final String varName, final Map<Label, String> labelNames) {
      sb.append("Attribute ")
          .append(varName)
          .append(" = new CodeComment(")
          .append(name(labelNames, a))
          .append(", ")
          .append(name(labelNames, b))
          .append(");\n");
    }
  }

  public static void main(final String[] args) {
    PrintWriter out = new PrintWriter(System.out);
    TraceClassVisitor cv = new TraceClassVisitor(null, new ASMifier(), out);
    cv.visit(
        V1_8,
        ACC_PUBLIC | ACC_SUPER | ACC_DEPRECATED | ACC_ABSTRACT,
        "pkg/sub/C-é(1)",
        "<T:Ljava/lang/Object;>Ljava/lang/Object;Ljava/lang/Comparable<TT;>;",
        "java/lang/Object",
        new String[] {"java/lang/Comparable", "java/io/Serializable"});
    cv.visitSource("C.java", "dbg\n");
    {
      ModuleVisitor mv = cv.visitModule("m", ACC_OPEN | ACC_SYNTHETIC, "1.0");
      mv.visitMainClass("pkg/Main");
      mv.visitPackage("pkg");
      mv.visitRequire("java.base", ACC_TRANSITIVE | ACC_STATIC_PHASE | ACC_MANDATED, null);
      mv.visitRequire("other", 0, "2");
      mv.visitExport("pkg", 0);
      mv.visitExport("pkg/a", ACC_SYNTHETIC, "x", "y");
      mv.visitOpen("pkg/b", 0, "z");
      mv.visitUse("pkg/S");
      mv.visitProvide("pkg/S", "pkg/I1", "pkg/I2");
      mv.visitEnd();
    }
    cv.visitNestHost("pkg/H");
    cv.visitOuterClass("pkg/O", "m", "()V");
    {
      AnnotationVisitor av = cv.visitAnnotation("LA;", true);
      av.visit("s", "x\"\n\u0001€");
      av.visit("b", (byte) -3);
      av.visit("z", true);
      av.visit("zf", false);
      av.visit("sh", (short) -300);
      av.visit("c", 'é');
      av.visit("i", Integer.MIN_VALUE);
      av.visit("j", Long.MAX_VALUE);
      av.visit("f", 1.0e10f);
      av.visit("d", Double.NEGATIVE_INFINITY);
      av.visit("t", Type.getType("[[Ljava/lang/String;"));
      av.visit("ab", new byte[] {1, -1});
      av.visit("az", new boolean[] {true, false});
      av.visit("as", new short[] {2, -2});
      av.visit("ac", new char[] {'a', '￿'});
      av.visit("ai", new int[] {});
      av.visit("aj", new long[] {-1, 5});
      av.visit("af", new float[] {0.1f, Float.NaN});
      av.visit("ad", new double[] {0.1, Double.MIN_VALUE});
      av.visitEnum("e", "LE;", "V");
      {
        AnnotationVisitor arr = av.visitArray("arr");
        arr.visit(null, 1L);
        {
          AnnotationVisitor nested = arr.visitAnnotation(null, "LB;");
          nested.visitArray("deep").visitEnd();
          nested.visitEnd();
        }
        arr.visitEnd();
      }
      av.visitEnd();
    }
    cv.visitTypeAnnotation(
            TypeReference.newSuperTypeReference(-1).getValue(),
            TypePath.fromString("[.*1;"),
            "LTA;",
            false)
        .visitEnd();
    cv.visitAttribute(new Comment());
    cv.visitAttribute(new Attribute("Rawé") {});
    cv.visitNestMember("pkg/N");
    cv.visitPermittedSubclass("pkg/P");
    cv.visitInnerClass(
        "pkg/C$1",
        null,
        null,
        ACC_PRIVATE | ACC_STATIC | ACC_ENUM | ACC_ANNOTATION | ACC_INTERFACE | ACC_ABSTRACT);
    cv.visitInnerClass("pkg/C$I", "pkg/C", "I", 0);
    {
      RecordComponentVisitor rv = cv.visitRecordComponent("rc", "I", "TT;");
      rv.visitAnnotation("LR;", false).visitEnd();
      rv.visitTypeAnnotation(
              TypeReference.newTypeReference(TypeReference.FIELD).getValue(), null, "LRT;", true)
          .visitEnd();
      rv.visitAttribute(new Comment());
      rv.visitEnd();
    }
    {
      FieldVisitor fv =
          cv.visitField(
              ACC_PROTECTED
                  | ACC_STATIC
                  | ACC_FINAL
                  | ACC_VOLATILE
                  | ACC_TRANSIENT
                  | ACC_ENUM
                  | ACC_SYNTHETIC
                  | ACC_DEPRECATED,
              "F",
              "F",
              null,
              Float.NaN);
      fv.visitAnnotation("LFA;", true).visitEnd();
      fv.visitTypeAnnotation(
              TypeReference.newTypeReference(TypeReference.FIELD).getValue(),
              TypePath.fromString("0;"),
              "LFT;",
              false)
          .visitEnd();
      fv.visitAttribute(new Comment());
      fv.visitEnd();
    }
    cv.visitField(0, "J", "J", "TT;", -7L).visitEnd();
    cv.visitField(ACC_PUBLIC, "S", "Ljava/lang/String;", null, "\\\t").visitEnd();
    cv.visitField(ACC_PRIVATE, "D", "D", null, 1.0e-300).visitEnd();
    cv.visitField(ACC_PRIVATE, "I", "I", null, 42).visitEnd();
    {
      MethodVisitor mv =
          cv.visitMethod(
              ACC_PUBLIC
                  | ACC_SYNCHRONIZED
                  | ACC_BRIDGE
                  | ACC_VARARGS
                  | ACC_NATIVE
                  | ACC_STRICT
                  | ACC_SYNTHETIC,
              "m",
              "(I)V",
              "(TT;)V",
              new String[] {"java/io/IOException", "java/lang/Error"});
      mv.visitParameter(null, ACC_FINAL);
      mv.visitParameter("p€", ACC_MANDATED | ACC_SYNTHETIC);
      {
        AnnotationVisitor dv = mv.visitAnnotationDefault();
        dv.visit(null, 1);
        dv.visitEnd();
      }
      mv.visitAnnotation("LM;", true).visitEnd();
      mv.visitTypeAnnotation(
              TypeReference.newTypeReference(TypeReference.METHOD_RETURN).getValue(),
              null,
              "LMT;",
              true)
          .visitEnd();
      mv.visitAnnotableParameterCount(1, false);
      mv.visitParameterAnnotation(0, "LP;", false).visitEnd();
      mv.visitAttribute(new Comment());
      mv.visitCode();
      Label l0 = new Label();
      Label l1 = new Label();
      Label l2 = new Label();
      Label l3 = new Label();
      Label l4 = new Label();
      mv.visitTryCatchBlock(l0, l1, l2, null);
      mv.visitTryCatchBlock(l0, l1, l3, "java/lang/Exception");
      mv.visitTryCatchAnnotation(
              TypeReference.newTryCatchReference(1).getValue(), null, "LTC;", true)
          .visitEnd();
      mv.visitLabel(l0);
      mv.visitLineNumber(7, l0);
      mv.visitInsn(NOP);
      mv.visitIntInsn(NEWARRAY, T_INT);
      mv.visitIntInsn(BIPUSH, -5);
      mv.visitIntInsn(SIPUSH, 1000);
      mv.visitVarInsn(ILOAD, 1);
      mv.visitVarInsn(ASTORE, 300);
      mv.visitTypeInsn(NEW, "java/lang/Object");
      mv.visitFieldInsn(GETSTATIC, "pkg/C", "F", "F");
      mv.visitMethodInsn(
          INVOKEINTERFACE, "java/lang/Comparable", "compareTo", "(Ljava/lang/Object;)I", true);
      mv.visitMethodInsn(INVOKESTATIC, "pkg/C", "s", "()V", false);
      mv.visitLdcInsn(-1);
      mv.visitLdcInsn(-0.0f);
      mv.visitLdcInsn(1L << 40);
      mv.visitLdcInsn(-0.0);
      mv.visitLdcInsn("ldc\u0000");
      mv.visitLdcInsn(Type.getType("Ljava/lang/String;"));
      mv.visitLdcInsn(Type.getMethodType("(I)V"));
      mv.visitLdcInsn(new Handle(H_GETSTATIC, "o", "f", "I", false));
      mv.visitLdcInsn(new ConstantDynamic("c", "I", new Handle(H_INVOKESTATIC, "B", "c", "()I", true)));
      mv.visitIincInsn(3, -1);
      mv.visitTableSwitchInsn(-1, 0, l2, l1, l4);
      mv.visitLookupSwitchInsn(l1, new int[] {5, -6}, new Label[] {l2, l3});
      mv.visitJumpInsn(IFNULL, l3);
      mv.visitInvokeDynamicInsn(
          "run",
          "()Ljava/lang/Runnable;",
          new Handle(H_INVOKESTATIC, "B", "bsm", "()V", true),
          Type.getMethodType("()V"),
          Type.getType("LX;"),
          new Handle(H_NEWINVOKESPECIAL, "X", "<init>", "()V", false),
          new ConstantDynamic(
              "c",
              "I",
              new Handle(H_INVOKESTATIC, "B", "c", "()I", false),
              1,
              "s",
              new ConstantDynamic(
                  "d", "J", new Handle(H_INVOKESTATIC, "B", "d", "()J", false), 2L)));
      mv.visitInvokeDynamicInsn(
          "none", "()V", new Handle(H_INVOKESTATIC, "B", "bsm", "()V", false));
      mv.visitMultiANewArrayInsn("[[I", 2);
      mv.visitInsnAnnotation(
              TypeReference.newTypeReference(TypeReference.NEW).getValue(),
              TypePath.fromString("["),
              "LI;",
              false)
          .visitEnd();
      mv.visitLabel(l1);
      mv.visitFrame(F_FULL, 3, new Object[] {"pkg/C", INTEGER, TOP}, 2, new Object[] {l0, l4});
      mv.visitFrame(
          F_NEW, 5, new Object[] {FLOAT, DOUBLE, LONG, NULL, UNINITIALIZED_THIS}, 0, new Object[0]);
      mv.visitFrame(F_APPEND, 1, new Object[] {"[I"}, 0, null);
      mv.visitFrame(F_CHOP, 2, null, 0, null);
      mv.visitFrame(F_SAME, 0, null, 0, null);
      mv.visitFrame(F_SAME1, 0, null, 1, new Object[] {"[I"});
      mv.visitInsn(RETURN);
      mv.visitLabel(l2);
      mv.visitLabel(l3);
      mv.visitLabel(l4);
      mv.visitLocalVariable("x", "I", null, l0, l2, 1);
      mv.visitLocalVariable("y", "TT;", "TT;", l1, l3, 2);
      mv.visitLocalVariableAnnotation(
              TypeReference.newTypeReference(TypeReference.LOCAL_VARIABLE).getValue(),
              TypePath.fromString("*"),
              new Label[] {l0, l1},
              new Label[] {l2, l3},
              new int[] {1, 2},
              "LLV;",
              true)
          .visitEnd();
      mv.visitAttribute(new CodeComment(l2, new Label()));
      mv.visitMaxs(1, 2);
      mv.visitEnd();
    }
    cv.visitMethod(ACC_ABSTRACT, "a", "()V", null, null).visitEnd();
    cv.visitEnd();
    out.flush();
  }
}
