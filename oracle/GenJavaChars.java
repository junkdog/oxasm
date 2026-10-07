/** Prints {@code crates/oxasm-util/src/check/java_chars.rs}: the Character predicates of the check adapters. */
import java.util.function.IntPredicate;
public class GenJavaChars {
  static void table(String name, String doc, IntPredicate p, int max) {
    System.out.println("/// " + doc);
    System.out.println("pub(crate) const " + name + ": &[(u32, u32)] = &[");
    StringBuilder line = new StringBuilder("   ");
    int start = -1;
    for (int cp = 0; cp <= max + 1; ++cp) {
      boolean in = cp <= max && p.test(cp);
      if (in && start < 0) start = cp;
      if (!in && start >= 0) {
        String item = String.format(" (0x%x, 0x%x),", start, cp - 1);
        if (line.length() + item.length() > 100) { System.out.println(line); line = new StringBuilder("   "); }
        line.append(item);
        start = -1;
      }
    }
    if (line.length() > 3) System.out.println(line);
    System.out.println("];");
    System.out.println();
  }
  public static void main(String[] a) {
    System.out.println("//! Java's `Character` predicates that the check adapters use, as code point ranges: generated");
    System.out.println("//! with Java " + System.getProperty("java.version") + " (Unicode as of that version) by `oracle/GenJavaChars.java`:");
    System.out.println("//! `java oracle/GenJavaChars.java > crates/oxasm-util/src/check/java_chars.rs`.");
    System.out.println();
    table("JAVA_IDENTIFIER_START", "`Character.isJavaIdentifierStart(int)`.", Character::isJavaIdentifierStart, 0x10FFFF);
    table("JAVA_IDENTIFIER_PART", "`Character.isJavaIdentifierPart(int)`.", Character::isJavaIdentifierPart, 0x10FFFF);
    table("DIGIT", "`Character.isDigit(char)` (UTF-16 code units).", c -> Character.isDigit((char) c), 0xFFFF);
  }
}
