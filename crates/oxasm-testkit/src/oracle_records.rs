//! Parsing the batch output of the oracle (`oracle/Oracle.java`): one record per input class,
//!
//! ```text
//! #oracle <ok|err|skip> <payload length in bytes> <id>\n<payload>\n
//! ```
//!
//! where an `err` payload is `<exception class>: <message>`. Used to cross-check testkit and
//! oxasm output against a live oracle run (`cargo xtask oracle <command> <inputs> > records`).

/// The status of an oracle record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleStatus {
    Ok,
    Err,
    Skip,
}

/// One record of an oracle batch run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleRecord {
    pub status: OracleStatus,
    /// The input: a file path, or `<jar>!/<entry>`.
    pub id: String,
    pub payload: Vec<u8>,
}

/// Parses the records of an oracle batch output; panics on malformed output.
#[track_caller]
pub fn parse_oracle_records(mut bytes: &[u8]) -> Vec<OracleRecord> {
    let mut records = Vec::new();
    while !bytes.is_empty() {
        let line_end = bytes
            .iter()
            .position(|b| *b == b'\n')
            .expect("oracle record header ends with a newline");
        let header = std::str::from_utf8(&bytes[..line_end]).expect("UTF-8 record header");
        let mut words = header.splitn(4, ' ');
        assert_eq!(
            words.next(),
            Some("#oracle"),
            "bad record header {header:?}"
        );
        let status = match words.next() {
            Some("ok") => OracleStatus::Ok,
            Some("err") => OracleStatus::Err,
            Some("skip") => OracleStatus::Skip,
            other => panic!("bad record status {other:?}"),
        };
        let length: usize = words
            .next()
            .and_then(|w| w.parse().ok())
            .expect("record payload length");
        let id = words.next().unwrap_or_default().to_owned();
        let payload_start = line_end + 1;
        let payload = bytes[payload_start..payload_start + length].to_vec();
        assert_eq!(
            bytes[payload_start + length],
            b'\n',
            "record {id} ends with a newline"
        );
        bytes = &bytes[payload_start + length + 1..];
        records.push(OracleRecord {
            status,
            id,
            payload,
        });
    }
    records
}
