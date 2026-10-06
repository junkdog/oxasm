//! Driver for `oracle/Oracle.java`: locates Java and the pinned ASM jars, runs batch commands and
//! talks to `Oracle serve`.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

use crate::args::Args;
use crate::pin::{ASM_JARS, ASM_VERSION};
use crate::{root_path, sha256_hex, walk_files};

/// JVM options for every oracle run. `verify` needs the verifier on for all class loaders. The
/// heap is capped so that a malformed class making ASM allocate a huge array (e.g. a `tableswitch`
/// with 2^31 entries) fails with a catchable `OutOfMemoryError` (reported as ASM failing) instead
/// of growing a dozen parallel `serve` JVMs into the kernel's OOM killer.
const JVM_OPTIONS: [&str; 3] = [
    "-XX:+UnlockDiagnosticVMOptions",
    "-XX:+BytecodeVerificationLocal",
    "-Xmx2g",
];

/// The oldest JDK whose source launcher compiles multi-file programs (JEP 458).
const MIN_JAVA: u32 = 22;

pub fn cli(mut args: Args) -> Result<()> {
    let Some(command) = args.next_positional() else {
        bail!("usage: cargo xtask oracle <command> [options] <inputs...>");
    };
    let oracle = Oracle::locate()?;
    let mut words = vec![command];
    words.extend(args.rest());
    let status = oracle.command(&words).status().context("cannot run java")?;
    ensure!(status.success(), "oracle exited with {status}");
    Ok(())
}

/// One `#oracle` record of a batch run.
#[derive(Debug)]
pub struct Record {
    pub status: Status,
    pub id: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Err,
    Skip,
}

impl Status {
    fn parse(word: &str) -> Result<Status> {
        Ok(match word {
            "ok" => Status::Ok,
            "err" => Status::Err,
            "skip" => Status::Skip,
            other => bail!("unknown record status {other:?}"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Oracle {
    java: PathBuf,
    /// The ASM jars, then the compiled oracle.
    class_path: String,
}

impl Oracle {
    /// Finds `java` and the ASM jars, checking the jars against the pinned hashes and the
    /// oracle source against the pinned version, and compiles the oracle if it changed.
    ///
    /// The oracle is runnable straight from source (see `oracle/Oracle.java`), but compiling it
    /// once saves a few CPU-seconds per JVM, which adds up over parity sweeps.
    pub fn locate() -> Result<Oracle> {
        let source = root_path("oracle/Oracle.java");
        let text = std::fs::read_to_string(&source)
            .with_context(|| format!("cannot read {}", source.display()))?;
        let pin = format!("ASM_VERSION = \"{ASM_VERSION}\"");
        ensure!(
            text.contains(&pin),
            "{} is not pinned to ASM {ASM_VERSION} (expected `{pin}`)",
            source.display()
        );

        let java = jdk_tool("java");
        check_java_version(&java)?;

        let mut jars = Vec::new();
        for (artifact, sha256) in ASM_JARS {
            let jar = asm_jar(artifact)?;
            let bytes =
                std::fs::read(&jar).with_context(|| format!("cannot read {}", jar.display()))?;
            let actual = sha256_hex(&bytes);
            ensure!(
                actual == sha256,
                "{} has SHA-256 {actual}, but the released {artifact}-{ASM_VERSION}.jar has \
                 {sha256}",
                jar.display()
            );
            jars.push(jar.to_string_lossy().into_owned());
        }
        let separator = if cfg!(windows) { ";" } else { ":" };
        let jars = jars.join(separator);
        let classes = compile(&jars)?;
        Ok(Oracle {
            java,
            class_path: format!("{jars}{separator}{}", classes.display()),
        })
    }

    /// A `java` command running the oracle with `words` (command, options, inputs).
    pub fn command<S: AsRef<std::ffi::OsStr>>(&self, words: &[S]) -> Command {
        let mut command = Command::new(&self.java);
        command
            .args(JVM_OPTIONS)
            .arg("--class-path")
            .arg(&self.class_path)
            .arg("Oracle")
            .args(words);
        command
    }

    /// Runs a batch command and parses its records.
    pub fn batch(&self, words: &[String]) -> Result<Vec<Record>> {
        let output = self
            .command(words)
            .stderr(Stdio::inherit())
            .output()
            .context("cannot run java")?;
        ensure!(
            output.status.success(),
            "oracle {} exited with {}",
            words.join(" "),
            output.status
        );
        parse_records(&output.stdout)
    }

    /// Runs a batch command whose output is plain text rather than records.
    pub fn text(&self, words: &[String]) -> Result<Vec<u8>> {
        let output = self
            .command(words)
            .stderr(Stdio::inherit())
            .output()
            .context("cannot run java")?;
        ensure!(
            output.status.success(),
            "oracle {} exited with {}",
            words.join(" "),
            output.status
        );
        Ok(output.stdout)
    }

    /// Sends every request to one `Oracle serve` process, and calls `on_response` with each
    /// request's index and the response, in request order.
    pub fn serve(
        &self,
        requests: &[ServeRequest<'_>],
        mut on_response: impl FnMut(usize, Response) -> Result<()>,
    ) -> Result<()> {
        let mut child: Child = self
            .command(&["serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("cannot run java")?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        std::thread::scope(|scope| -> Result<()> {
            // Requests are written from a second thread, so that neither side blocks on a full
            // pipe while the other waits.
            let writer = scope.spawn(move || -> std::io::Result<()> {
                let mut out = BufWriter::with_capacity(1 << 16, stdin);
                for request in requests {
                    write_frame(&mut out, request.command.as_bytes())?;
                    write_frame(&mut out, request.class_file)?;
                }
                out.flush()
                // Dropping `out` closes stdin, which ends the server's loop.
            });

            let mut input = BufReader::with_capacity(1 << 16, stdout);
            for index in 0..requests.len() {
                let response = read_response(&mut input)
                    .with_context(|| format!("reading oracle response {index}"))?;
                on_response(index, response)?;
            }
            writer
                .join()
                .expect("oracle writer thread panicked")
                .context("writing oracle requests")
        })?;

        let status = child.wait()?;
        ensure!(status.success(), "oracle serve exited with {status}");
        Ok(())
    }
}

/// One `serve` request: a command line (no inputs) and a class file.
#[derive(Debug)]
pub struct ServeRequest<'a> {
    pub command: &'a str,
    pub class_file: &'a [u8],
}

#[derive(Debug)]
pub struct Response {
    pub ok: bool,
    pub payload: Vec<u8>,
}

fn write_frame(out: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| std::io::Error::other("oracle frame larger than 4 GiB"))?;
    out.write_all(&length.to_be_bytes())?;
    out.write_all(bytes)
}

fn read_response(input: &mut impl Read) -> Result<Response> {
    let mut header = [0u8; 5];
    input.read_exact(&mut header)?;
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
    let mut payload = vec![0; usize::try_from(length)?];
    input.read_exact(&mut payload)?;
    Ok(Response {
        ok: header[0] == 0,
        payload,
    })
}

pub fn parse_records(mut bytes: &[u8]) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    while !bytes.is_empty() {
        let newline = bytes
            .iter()
            .position(|&b| b == b'\n')
            .context("truncated record header")?;
        let header = std::str::from_utf8(&bytes[..newline]).context("record header")?;
        let mut words = header.splitn(4, ' ');
        let (Some("#oracle"), Some(status), Some(length), Some(id)) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            bail!("malformed record header {header:?}");
        };
        let length: usize = length.parse().context("record length")?;
        let rest = &bytes[newline + 1..];
        ensure!(
            rest.len() > length && rest[length] == b'\n',
            "truncated record {id}"
        );
        records.push(Record {
            status: Status::parse(status)?,
            id: id.to_owned(),
            payload: rest[..length].to_vec(),
        });
        bytes = &rest[length + 1..];
    }
    Ok(records)
}

/// `$JAVA_HOME/bin/<tool>`, else `<tool>` from `PATH`.
fn jdk_tool(tool: &str) -> PathBuf {
    match std::env::var_os("JAVA_HOME") {
        Some(home) if !home.is_empty() => Path::new(&home).join("bin").join(tool),
        _ => PathBuf::from(tool),
    }
}

/// Compiles `oracle/**/*.java` into `target/oracle/classes`, unless the sources and class path
/// are unchanged since the last compilation.
fn compile(class_path: &str) -> Result<PathBuf> {
    let sources: Vec<PathBuf> = walk_files(&root_path("oracle"))?
        .into_iter()
        .filter(|f| f.extension().is_some_and(|e| e == "java"))
        .collect();
    let mut fingerprint = Vec::from(class_path.as_bytes());
    for source in &sources {
        fingerprint.extend_from_slice(source.as_os_str().as_encoded_bytes());
        fingerprint.extend(std::fs::read(source)?);
    }
    let fingerprint = sha256_hex(&fingerprint);

    let classes = root_path("target/oracle/classes");
    let stamp = classes.join(".fingerprint");
    if std::fs::read_to_string(&stamp).is_ok_and(|s| s == fingerprint) {
        return Ok(classes);
    }
    if classes.exists() {
        std::fs::remove_dir_all(&classes)?;
    }
    let status = Command::new(jdk_tool("javac"))
        .args(["-encoding", "UTF-8", "-nowarn", "-d"])
        .arg(&classes)
        .arg("--class-path")
        .arg(class_path)
        .args(&sources)
        .status()
        .context("cannot run javac")?;
    ensure!(status.success(), "javac failed on oracle/");
    std::fs::write(&stamp, fingerprint)?;
    Ok(classes)
}

/// `$JAVA_HOME`, else the installation `java` on `PATH` belongs to.
pub fn java_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|h| !h.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    let output = Command::new("java")
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .context("cannot run java")?;
    let settings = String::from_utf8_lossy(&output.stderr);
    settings
        .lines()
        .find_map(|line| line.trim().strip_prefix("java.home = "))
        .map(PathBuf::from)
        .context("cannot determine java.home; set JAVA_HOME")
}

fn check_java_version(java: &Path) -> Result<()> {
    let output = Command::new(java)
        .arg("-version")
        .output()
        .with_context(|| format!("cannot run {}", java.display()))?;
    let banner = String::from_utf8_lossy(&output.stderr);
    let major = banner
        .split('"')
        .nth(1)
        .and_then(|version| version.split(['.', '-', '+']).next())
        .and_then(|major| major.parse::<u32>().ok())
        .with_context(|| format!("cannot parse the version of {}: {banner}", java.display()))?;
    ensure!(
        major >= MIN_JAVA,
        "{} is Java {major}; the oracle needs Java {MIN_JAVA} or newer (CI uses 25)",
        java.display()
    );
    Ok(())
}

/// The local Maven repository: `$OXASM_M2_REPO`, else `~/.m2/repository`.
pub fn maven_repository() -> Result<PathBuf> {
    if let Some(repo) = std::env::var_os("OXASM_M2_REPO") {
        return Ok(PathBuf::from(repo));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(Path::new(&home).join(".m2").join("repository"))
}

/// The path of `group:artifact:version` (a jar) in the local Maven repository, downloading it
/// with `mvn dependency:get` if it is missing.
pub fn maven_jar(group: &str, artifact: &str, version: &str) -> Result<PathBuf> {
    let mut jar = maven_repository()?;
    jar.extend(group.split('.'));
    jar.push(artifact);
    jar.push(version);
    jar.push(format!("{artifact}-{version}.jar"));
    if !jar.is_file() {
        let coordinate = format!("{group}:{artifact}:{version}");
        eprintln!("xtask: fetching {coordinate} with mvn");
        let status = Command::new("mvn")
            .args([
                "--batch-mode",
                "--quiet",
                "dependency:get",
                "-Dtransitive=false",
            ])
            .arg(format!("-Dartifact={coordinate}"))
            .status()
            .context("cannot run mvn")?;
        ensure!(status.success(), "mvn dependency:get {coordinate} failed");
        ensure!(
            jar.is_file(),
            "mvn fetched {coordinate}, but {} does not exist",
            jar.display()
        );
    }
    Ok(jar)
}

fn asm_jar(artifact: &str) -> Result<PathBuf> {
    maven_jar("org.ow2.asm", artifact, ASM_VERSION)
}
