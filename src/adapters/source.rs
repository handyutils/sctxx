//! Reading session files off disk.
//!
//! Cold Codex rollouts are stored zstd-compressed, so every adapter goes
//! through here rather than opening files itself. Bytes are hashed after
//! decompression so the same logical session hashes the same whether or not it
//! has been compressed since.

use crate::error::{Error, Result};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// The decompressed text of one session file, split into lines.
#[derive(Debug)]
pub struct SourceText {
    pub path: PathBuf,
    /// Lines with terminators removed. Invalid UTF-8 is replaced, not rejected.
    pub lines: Vec<String>,
    /// sha256 of the decompressed bytes, hex encoded.
    pub hash: String,
    /// Size of the decompressed content in bytes.
    pub bytes: usize,
}

/// True when `path` looks like a zstd-compressed rollout.
pub fn is_compressed(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zst"))
}

/// Read and decompress a session file.
pub fn read(path: &Path) -> Result<SourceText> {
    let raw = std::fs::read(path).map_err(|source| Error::io(path, source))?;
    let content = if is_compressed(path) {
        decompress(path, &raw)?
    } else {
        raw
    };

    let hash = format!("{:x}", Sha256::digest(&content));
    let bytes = content.len();
    let lines = split_lines(&content);
    Ok(SourceText {
        path: path.to_path_buf(),
        lines,
        hash,
        bytes,
    })
}

#[cfg(feature = "zstd")]
fn decompress(path: &Path, raw: &[u8]) -> Result<Vec<u8>> {
    zstd::stream::decode_all(raw).map_err(|source| Error::io(path, source))
}

#[cfg(not(feature = "zstd"))]
fn decompress(path: &Path, _raw: &[u8]) -> Result<Vec<u8>> {
    Err(Error::Other(format!(
        "{} is zstd-compressed but this build has the `zstd` feature disabled",
        path.display()
    )))
}

fn split_lines(content: &[u8]) -> Vec<String> {
    content
        .split(|byte| *byte == b'\n')
        .map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            String::from_utf8_lossy(line).into_owned()
        })
        .collect()
}

/// Hash of the concatenated decompressed bytes of several sources, so a session
/// that spans a main file plus sidechains has one stable identity.
pub fn combined_hash(hashes: &[String]) -> String {
    let mut hasher = Sha256::new();
    for hash in hashes {
        hasher.update(hash.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// A cheap one-pass summary of a session file, used by `list` and `find` so
/// they never have to parse or retain a whole store.
#[derive(Debug, Clone)]
pub struct FileScan {
    pub lines: usize,
    pub bytes: u64,
    pub first_line: String,
    /// Up to [`SCAN_SAMPLE_LINES`] lines from the head of the file.
    pub head: Vec<String>,
    /// The last non-empty line, for the session end timestamp.
    pub last_line: String,
}

/// How many head lines a scan retains. Enough to find the session header, the
/// first human message, and a title without holding the file in memory.
pub const SCAN_SAMPLE_LINES: usize = 60;

/// Stream `path` once, counting lines and keeping only what discovery needs.
pub fn scan(path: &Path) -> Result<FileScan> {
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let file = std::fs::File::open(path).map_err(|source| Error::io(path, source))?;
    let reader: Box<dyn Read> = if is_compressed(path) {
        #[cfg(feature = "zstd")]
        {
            Box::new(
                zstd::stream::read::Decoder::new(file).map_err(|source| Error::io(path, source))?,
            )
        }
        #[cfg(not(feature = "zstd"))]
        {
            drop(file);
            return Err(Error::Other(format!(
                "{} is zstd-compressed but this build has the `zstd` feature disabled",
                path.display()
            )));
        }
    } else {
        Box::new(file)
    };

    let mut scan = FileScan {
        lines: 0,
        bytes,
        first_line: String::new(),
        head: Vec::new(),
        last_line: String::new(),
    };
    for line in BufReader::new(reader).lines() {
        let line = match line {
            Ok(line) => line,
            // A binary or truncated tail should not make a store unlistable.
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        scan.lines += 1;
        if scan.first_line.is_empty() {
            scan.first_line = line.clone();
        }
        if scan.head.len() < SCAN_SAMPLE_LINES {
            scan.head.push(line.clone());
        }
        scan.last_line = line;
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write fixture");
        path
    }

    #[test]
    fn crlf_and_lf_split_the_same() {
        assert_eq!(split_lines(b"a\r\nb\n"), vec!["a", "b", ""]);
        assert_eq!(split_lines(b"a\nb"), vec!["a", "b"]);
    }

    #[test]
    fn invalid_utf8_is_replaced_not_rejected() {
        let lines = split_lines(&[b'a', 0xff, b'\n']);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains('\u{fffd}'), "{:?}", lines[0]);
    }

    #[test]
    fn reading_reports_hash_and_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "s.jsonl", b"{\"a\":1}\n");
        let source = read(&path).expect("read");
        assert_eq!(source.bytes, 8);
        assert_eq!(source.hash.len(), 64);
        assert_eq!(source.lines[0], "{\"a\":1}");
    }

    #[test]
    fn compression_detection_is_extension_based() {
        assert!(is_compressed(Path::new("rollout-x.jsonl.zst")));
        assert!(!is_compressed(Path::new("rollout-x.jsonl")));
    }

    #[test]
    fn scanning_counts_lines_and_keeps_the_ends() {
        let dir = tempfile::tempdir().expect("tempdir");
        let body: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let path = write(dir.path(), "s.jsonl", body.as_bytes());
        let scan = scan(&path).expect("scan");
        assert_eq!(scan.lines, 100);
        assert_eq!(scan.first_line, "line 0");
        assert_eq!(scan.last_line, "line 99");
        assert_eq!(scan.head.len(), SCAN_SAMPLE_LINES);
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn a_compressed_rollout_reads_like_a_plain_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plain = b"{\"type\":\"session_meta\"}\n{\"type\":\"response_item\"}\n";
        let compressed = zstd::stream::encode_all(&plain[..], 3).expect("compress");
        let path = write(dir.path(), "rollout.jsonl.zst", &compressed);
        let source = read(&path).expect("read");
        assert_eq!(source.lines[0], "{\"type\":\"session_meta\"}");
        assert_eq!(scan(&path).expect("scan").lines, 2);
    }
}
