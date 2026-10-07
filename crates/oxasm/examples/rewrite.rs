//! Reads class files and writes them back with a `ClassWriter`: `cargo run -p oxasm --example
//! rewrite -- [--copy-pool] [--expand-frames] <in.class> <out.class>...` (pairs of paths).
//! A debugging aid for the writer's parity sweeps and fuzz findings.

use oxasm::{ClassReader, ClassWriter, ClassWriterFlags, ParsingOptions};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let copy_pool = take_flag(&mut args, "--copy-pool");
    let options = if take_flag(&mut args, "--expand-frames") {
        ParsingOptions::EXPAND_FRAMES
    } else {
        ParsingOptions::empty()
    };
    for pair in args.chunks(2) {
        let [input, output] = pair else {
            panic!("usage: rewrite [--copy-pool] [--expand-frames] <in> <out>...");
        };
        let bytes = std::fs::read(input).expect("read the input");
        let result = ClassReader::new(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|reader| {
                let mut writer = if copy_pool {
                    ClassWriter::from_reader(&reader, ClassWriterFlags::empty())
                        .map_err(|e| e.to_string())?
                } else {
                    ClassWriter::new(ClassWriterFlags::empty())
                };
                reader
                    .accept(&mut writer, options)
                    .map_err(|e| e.to_string())?;
                writer.to_bytes().map_err(|e| e.to_string())
            });
        match result {
            Ok(written) => std::fs::write(output, written).expect("write the output"),
            Err(error) => eprintln!("{input}: {error}"),
        }
    }
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let present = args.iter().any(|a| a == flag);
    args.retain(|a| a != flag);
    present
}
