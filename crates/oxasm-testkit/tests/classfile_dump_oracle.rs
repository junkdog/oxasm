//! Cross-check of the `ClassFile` dumper against a live oracle run over any set of class files
//! (not run by default: it needs the oracle's output). Usage:
//!
//! ```text
//! cargo xtask oracle dump <class files or directories> > /tmp/records
//! ORACLE_DUMP_RECORDS=/tmp/records [ORACLE_INPUT_ROOT=<dir>] cargo test -p oxasm-testkit --test classfile_dump_oracle \
//!     -- --ignored --nocapture
//! ```
//!
//! Record ids of classes found in a directory input are relative to that directory, which
//! `ORACLE_INPUT_ROOT` names.

use oxasm_testkit::classfile_dump;
use oxasm_testkit::oracle_records::{OracleStatus, parse_oracle_records};

#[test]
#[ignore = "needs ORACLE_DUMP_RECORDS, the output of `cargo xtask oracle dump`"]
fn dump_equals_the_oracle_dump() {
    let path = std::env::var("ORACLE_DUMP_RECORDS").expect("ORACLE_DUMP_RECORDS is not set");
    let records = parse_oracle_records(&std::fs::read(path).unwrap());
    let root = std::path::PathBuf::from(std::env::var("ORACLE_INPUT_ROOT").unwrap_or_default());
    let (mut compared, mut errors) = (0, 0);
    let mut mismatches = Vec::new();
    for record in &records {
        if record.status == OracleStatus::Skip || record.id.contains("!/") {
            continue;
        }
        let bytes = std::fs::read(root.join(&record.id)).unwrap();
        let (status, payload) = match classfile_dump::dump(&bytes) {
            Ok(dump) => (OracleStatus::Ok, dump),
            Err(e) => (OracleStatus::Err, e.describe()),
        };
        compared += 1;
        if record.status == OracleStatus::Err {
            errors += 1;
        }
        if (status, &payload) != (record.status, &record.payload) {
            mismatches.push(format!(
                "{}\n  oracle: {:?} {}\n  oxasm:  {:?} {}",
                record.id,
                record.status,
                String::from_utf8_lossy(&record.payload[..record.payload.len().min(200)]),
                status,
                String::from_utf8_lossy(&payload[..payload.len().min(200)])
            ));
        }
    }
    println!(
        "{compared} classes compared ({errors} oracle errors), {} mismatched",
        mismatches.len()
    );
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
