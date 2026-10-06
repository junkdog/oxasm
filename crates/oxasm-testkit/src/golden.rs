//! Loading the committed T0 goldens (`goldens/`), which `cargo xtask golden --regen` writes from
//! the ASM 9.10.1 oracle.
//!
//! A golden is named after the class (`jdk3.AllStructures$1`, `invalid.InvalidOpcode`,
//! `resources.asm.Issue307600`), with a `.txt` extension when ASM succeeded and `.err` (holding
//! `<exception class>: <message>`) when it threw. Text is semi-UTF-8, the encoding of `JavaStr`.

use std::path::{Path, PathBuf};

/// The `ClassReader.accept` options a Textifier golden was produced with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReaderOptions {
    Default,
    SkipCode,
    SkipDebug,
    SkipFrames,
    ExpandFrames,
}

impl ReaderOptions {
    pub const ALL: [ReaderOptions; 5] = [
        ReaderOptions::Default,
        ReaderOptions::SkipCode,
        ReaderOptions::SkipDebug,
        ReaderOptions::SkipFrames,
        ReaderOptions::ExpandFrames,
    ];

    fn dir(self) -> &'static str {
        match self {
            ReaderOptions::Default => "textifier/default",
            ReaderOptions::SkipCode => "textifier/skip_code",
            ReaderOptions::SkipDebug => "textifier/skip_debug",
            ReaderOptions::SkipFrames => "textifier/skip_frames",
            ReaderOptions::ExpandFrames => "textifier/expand_frames",
        }
    }
}

/// What ASM produced for one class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Golden {
    /// The output, as bytes (semi-UTF-8 text).
    Ok(Vec<u8>),
    /// ASM threw; `<exception class>: <message>`.
    Err(String),
}

impl Golden {
    /// The output; panics if ASM threw.
    #[track_caller]
    pub fn unwrap(self) -> Vec<u8> {
        match self {
            Golden::Ok(bytes) => bytes,
            Golden::Err(e) => panic!("the golden is an ASM error: {e}"),
        }
    }

    /// The output as UTF-8 text; panics if ASM threw or the text is not valid UTF-8.
    #[track_caller]
    pub fn unwrap_text(self) -> String {
        String::from_utf8(self.unwrap()).expect("golden is not valid UTF-8")
    }
}

/// The directory holding all goldens.
pub fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("goldens")
}

/// ASM's Textifier output for the class `name` read with `options`; panics if there is none.
#[track_caller]
pub fn textifier(name: &str, options: ReaderOptions) -> Golden {
    load(options.dir(), name)
}

/// The `ClassFile` structural dump of the class `name`; panics if there is none.
#[track_caller]
pub fn dump(name: &str) -> Golden {
    load("dump", name)
}

#[track_caller]
fn load(set: &str, name: &str) -> Golden {
    let dir = goldens_dir().join(set);
    if let Ok(bytes) = std::fs::read(dir.join(format!("{name}.txt"))) {
        return Golden::Ok(bytes);
    }
    match std::fs::read_to_string(dir.join(format!("{name}.err"))) {
        Ok(error) => Golden::Err(error),
        Err(_) => panic!(
            "no golden {set}/{name}.{{txt,err}} under {}; run `cargo xtask golden --regen`",
            goldens_dir().display()
        ),
    }
}
