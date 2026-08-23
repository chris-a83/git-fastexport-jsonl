// Reader and writer for the text stream produced by `git fast-export` and
// consumed by `git fast-import`. Only the subset of commands that shows up in
// an ordinary linear-to-moderately-branched export is handled: blob, commit,
// reset, and done. Tags, notes, copies/renames, and the delimited `data <<EOF`
// form are out of scope for now (see README).

use std::collections::HashSet;

use crate::model::{is_valid_tz_offset, Blob, Commit, DataRef, Event, FileChange, FileModify, PersonStamp, Reset};

pub struct ParseOptions {
    pub lenient: bool,
}

#[derive(Debug)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0, line: 1 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn read_line(&mut self) -> Option<(usize, &'a [u8])> {
        if self.is_eof() {
            return None;
        }
        let line_no = self.line;
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
            self.pos += 1;
        }
        let line = &self.data[start..self.pos];
        if self.pos < self.data.len() {
            self.pos += 1;
        }
        self.line += 1;
        Some((line_no, line))
    }

    fn peek_line(&self) -> Option<&'a [u8]> {
        if self.is_eof() {
            return None;
        }
        let mut end = self.pos;
        while end < self.data.len() && self.data[end] != b'\n' {
            end += 1;
        }
        Some(&self.data[self.pos..end])
    }

    fn read_exact(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        if self.pos + n > self.data.len() {
            return Err(ParseError {
                line: self.line,
                message: "unexpected end of input while reading a data payload".to_string(),
            });
        }
        let bytes = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(bytes)
    }

    fn consume_lf_if_present(&mut self) -> bool {
        if self.pos < self.data.len() && self.data[self.pos] == b'\n' {
            self.pos += 1;
            self.line += 1;
            true
        } else {
            false
        }
    }
}

fn strip_prefix<'a>(line: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    if line.starts_with(prefix) {
        Some(&line[prefix.len()..])
    } else {
        None
    }
}

fn starts_with_peek(cur: &Cursor, prefix: &[u8]) -> bool {
    match cur.peek_line() {
        Some(line) => line.starts_with(prefix),
        None => false,
    }
}

pub fn parse(input: &[u8], opts: &ParseOptions) -> Result<Vec<Event>, ParseError> {
    let mut cur = Cursor::new(input);
    let mut events = Vec::new();
    let mut known_marks: HashSet<u64> = HashSet::new();

    while let Some((line_no, line)) = cur.read_line() {
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        if line == b"blob" {
            let blob = parse_blob(&mut cur, opts)?;
            if let Some(mark) = blob.mark {
                known_marks.insert(mark);
            }
            events.push(Event::Blob(blob));
        } else if let Some(rest) = strip_prefix(line, b"commit ") {
            let commit = parse_commit(&mut cur, rest, opts, &known_marks)?;
            if let Some(mark) = commit.mark {
                known_marks.insert(mark);
            }
            events.push(Event::Commit(commit));
        } else if let Some(rest) = strip_prefix(line, b"reset ") {
            events.push(Event::Reset(parse_reset(&mut cur, rest)?));
        } else if line == b"done" {
            events.push(Event::Done);
            break;
        } else if opts.lenient {
            continue;
        } else {
            return Err(ParseError {
                line: line_no,
                message: format!("unsupported command: {}", String::from_utf8_lossy(line)),
            });
        }
    }

    Ok(events)
}

fn parse_blob(cur: &mut Cursor, opts: &ParseOptions) -> Result<Blob, ParseError> {
    let mark = if starts_with_peek(cur, b"mark :") {
        let (line_no, line) = cur.read_line().unwrap();
        Some(parse_mark_id(strip_prefix(line, b"mark :").unwrap(), line_no)?)
    } else {
        None
    };
    let data = parse_data(cur, opts)?;
    Ok(Blob { mark, data })
}

fn parse_commit(
    cur: &mut Cursor,
    branch_bytes: &[u8],
    opts: &ParseOptions,
    known_marks: &HashSet<u64>,
) -> Result<Commit, ParseError> {
    let branch = String::from_utf8_lossy(branch_bytes).trim().to_string();

    let mark = if starts_with_peek(cur, b"mark :") {
        let (line_no, line) = cur.read_line().unwrap();
        Some(parse_mark_id(strip_prefix(line, b"mark :").unwrap(), line_no)?)
    } else {
        None
    };

    let author = if starts_with_peek(cur, b"author ") {
        let (line_no, line) = cur.read_line().unwrap();
        Some(parse_person(strip_prefix(line, b"author ").unwrap(), line_no, opts)?)
    } else {
        None
    };

    let (line_no, line) = cur
        .read_line()
        .ok_or_else(|| ParseError { line: cur.line, message: "expected committer line, found end of input".to_string() })?;
    let committer_rest = strip_prefix(line, b"committer ").ok_or_else(|| ParseError {
        line: line_no,
        message: format!("expected committer line, found: {}", String::from_utf8_lossy(line)),
    })?;
    let committer = parse_person(committer_rest, line_no, opts)?;

    let message = parse_data(cur, opts)?;

    let from = if starts_with_peek(cur, b"from ") {
        let (line_no, line) = cur.read_line().unwrap();
        Some(resolve_commitish(strip_prefix(line, b"from ").unwrap(), opts, known_marks, line_no)?)
    } else {
        None
    };

    let mut merges = Vec::new();
    while starts_with_peek(cur, b"merge ") {
        let (line_no, line) = cur.read_line().unwrap();
        merges.push(resolve_commitish(strip_prefix(line, b"merge ").unwrap(), opts, known_marks, line_no)?);
    }

    let mut file_changes = Vec::new();
    loop {
        match cur.peek_line() {
            None => break,
            Some(line) if line.is_empty() => {
                cur.read_line();
                break;
            }
            Some(line) if strip_prefix(line, b"M ").is_some() => {
                let (line_no, line) = cur.read_line().unwrap();
                let rest = strip_prefix(line, b"M ").unwrap();
                file_changes.push(FileChange::Modify(parse_filemodify(rest, cur, opts, line_no)?));
            }
            Some(line) if strip_prefix(line, b"D ").is_some() => {
                let (line_no, line) = cur.read_line().unwrap();
                let rest = strip_prefix(line, b"D ").unwrap();
                let path = unquote_path(rest, opts, line_no)?;
                file_changes.push(FileChange::Delete { path });
            }
            _ => break,
        }
    }

    Ok(Commit { branch, mark, author, committer, message, from, merges, file_changes })
}

fn parse_reset(cur: &mut Cursor, branch_bytes: &[u8]) -> Result<Reset, ParseError> {
    let branch = String::from_utf8_lossy(branch_bytes).trim().to_string();
    let from = if starts_with_peek(cur, b"from ") {
        let (_, line) = cur.read_line().unwrap();
        Some(String::from_utf8_lossy(strip_prefix(line, b"from ").unwrap()).trim().to_string())
    } else {
        None
    };
    Ok(Reset { branch, from })
}

fn parse_data(cur: &mut Cursor, opts: &ParseOptions) -> Result<Vec<u8>, ParseError> {
    let (line_no, line) = cur
        .read_line()
        .ok_or_else(|| ParseError { line: cur.line, message: "expected data command, found end of input".to_string() })?;
    let rest = strip_prefix(line, b"data ").ok_or_else(|| ParseError {
        line: line_no,
        message: format!("expected data command, found: {}", String::from_utf8_lossy(line)),
    })?;
    if rest.starts_with(b"<<") {
        return Err(ParseError {
            line: line_no,
            message: "delimited data format (data <<DELIM) is not supported".to_string(),
        });
    }
    let count_str = std::str::from_utf8(rest)
        .map_err(|_| ParseError { line: line_no, message: "data length is not valid UTF-8".to_string() })?
        .trim();
    let count: usize = count_str
        .parse()
        .map_err(|_| ParseError { line: line_no, message: format!("invalid data length: {}", count_str) })?;
    let data = cur.read_exact(count)?.to_vec();

    let had_lf = cur.consume_lf_if_present();
    if !had_lf && !cur.is_eof() && !opts.lenient {
        return Err(ParseError {
            line: line_no,
            message: "data payload must be followed by a newline separator".to_string(),
        });
    }
    Ok(data)
}

fn parse_mark_id(bytes: &[u8], line_no: usize) -> Result<u64, ParseError> {
    let s = std::str::from_utf8(bytes)
        .map_err(|_| ParseError { line: line_no, message: "mark id is not valid UTF-8".to_string() })?;
    s.trim().parse::<u64>().map_err(|_| ParseError { line: line_no, message: format!("invalid mark id: {}", s) })
}

fn resolve_commitish(
    rest: &[u8],
    opts: &ParseOptions,
    known_marks: &HashSet<u64>,
    line_no: usize,
) -> Result<String, ParseError> {
    let text = std::str::from_utf8(rest)
        .map_err(|_| ParseError { line: line_no, message: "commit-ish is not valid UTF-8".to_string() })?
        .trim()
        .to_string();
    if let Some(mark_str) = text.strip_prefix(':') {
        let mark_id: u64 = mark_str
            .parse()
            .map_err(|_| ParseError { line: line_no, message: format!("invalid mark reference: {}", text) })?;
        if !opts.lenient && !known_marks.contains(&mark_id) {
            return Err(ParseError { line: line_no, message: format!("reference to unknown mark: :{}", mark_id) });
        }
    }
    Ok(text)
}

fn parse_person(rest: &[u8], line_no: usize, opts: &ParseOptions) -> Result<PersonStamp, ParseError> {
    let text = std::str::from_utf8(rest)
        .map_err(|_| ParseError { line: line_no, message: "person line is not valid UTF-8".to_string() })?;

    let open = text.rfind('<');
    let close = text.rfind('>');

    let (name, email, remainder) = match (open, close) {
        (Some(o), Some(c)) if o < c => {
            let name = text[..o].trim();
            let name = if name.is_empty() { None } else { Some(name.to_string()) };
            (name, text[o + 1..c].to_string(), text[c + 1..].trim())
        }
        _ if opts.lenient => (None, "unknown@invalid".to_string(), text.trim()),
        _ => return Err(ParseError { line: line_no, message: format!("malformed person line: {}", text) }),
    };

    let mut fields = remainder.split_whitespace();
    let timestamp = match fields.next().and_then(|s| s.parse::<i64>().ok()) {
        Some(t) => t,
        None if opts.lenient => 0,
        None => return Err(ParseError { line: line_no, message: format!("invalid or missing timestamp in: {}", text) }),
    };
    let tz_offset = match fields.next() {
        Some(tz) if is_valid_tz_offset(tz) => tz.to_string(),
        _ if opts.lenient => "+0000".to_string(),
        _ => {
            return Err(ParseError {
                line: line_no,
                message: format!("invalid or missing timezone offset in: {}", text),
            })
        }
    };

    Ok(PersonStamp { name, email, timestamp, tz_offset })
}

fn parse_filemodify(
    rest: &[u8],
    cur: &mut Cursor,
    opts: &ParseOptions,
    line_no: usize,
) -> Result<FileModify, ParseError> {
    let mut parts = rest.splitn(3, |&b| b == b' ');
    let mode_bytes =
        parts.next().ok_or_else(|| ParseError { line: line_no, message: "filemodify missing mode".to_string() })?;
    let dataref_bytes = parts
        .next()
        .ok_or_else(|| ParseError { line: line_no, message: "filemodify missing dataref".to_string() })?;
    let path_bytes =
        parts.next().ok_or_else(|| ParseError { line: line_no, message: "filemodify missing path".to_string() })?;

    let mode = std::str::from_utf8(mode_bytes)
        .map_err(|_| ParseError { line: line_no, message: "filemodify mode is not valid UTF-8".to_string() })?
        .to_string();
    if !opts.lenient && !is_valid_mode(&mode) {
        return Err(ParseError { line: line_no, message: format!("invalid file mode: {}", mode) });
    }

    let path = unquote_path(path_bytes, opts, line_no)?;

    let dataref = if dataref_bytes == b"inline" {
        DataRef::Inline(parse_data(cur, opts)?)
    } else if dataref_bytes.starts_with(b":") {
        DataRef::Mark(parse_mark_id(&dataref_bytes[1..], line_no)?)
    } else {
        let sha = std::str::from_utf8(dataref_bytes)
            .map_err(|_| ParseError { line: line_no, message: "sha1 dataref is not valid UTF-8".to_string() })?;
        if !opts.lenient && !is_valid_sha1(sha) {
            return Err(ParseError { line: line_no, message: format!("invalid sha1: {}", sha) });
        }
        DataRef::Sha1(sha.to_string())
    };

    Ok(FileModify { mode, dataref, path })
}

fn is_valid_mode(mode: &str) -> bool {
    matches!(mode, "100644" | "100755" | "120000" | "160000" | "040000")
}

fn is_valid_sha1(sha: &str) -> bool {
    sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

fn unquote_path(bytes: &[u8], opts: &ParseOptions, line_no: usize) -> Result<String, ParseError> {
    if bytes.first() == Some(&b'"') {
        if bytes.len() < 2 || bytes.last() != Some(&b'"') {
            if opts.lenient {
                return Ok(String::from_utf8_lossy(bytes).to_string());
            }
            return Err(ParseError { line: line_no, message: "unterminated quoted path".to_string() });
        }
        return unquote_c_string(&bytes[1..bytes.len() - 1], opts, line_no);
    }
    std::str::from_utf8(bytes)
        .map(|s| s.to_string())
        .map_err(|_| ParseError { line: line_no, message: "path is not valid UTF-8".to_string() })
}

fn unquote_c_string(bytes: &[u8], opts: &ParseOptions, line_no: usize) -> Result<String, ParseError> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'"' => {
                    out.push(b'"');
                    i += 2;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                b'0'..=b'3' if i + 3 < bytes.len() && (b'0'..=b'7').contains(&bytes[i + 2]) && (b'0'..=b'7').contains(&bytes[i + 3]) =>
                {
                    out.push((next - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0'));
                    i += 4;
                }
                other => {
                    if opts.lenient {
                        out.push(other);
                        i += 2;
                    } else {
                        return Err(ParseError {
                            line: line_no,
                            message: format!("unsupported escape sequence: \\{}", other as char),
                        });
                    }
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out)
        .map_err(|_| ParseError { line: line_no, message: "quoted path is not valid UTF-8 after unescaping".to_string() })
}

pub fn write(events: &[Event]) -> Vec<u8> {
    let mut out = Vec::new();
    for event in events {
        match event {
            Event::Blob(blob) => write_blob(&mut out, blob),
            Event::Commit(commit) => write_commit(&mut out, commit),
            Event::Reset(reset) => write_reset(&mut out, reset),
            Event::Done => out.extend_from_slice(b"done\n"),
        }
    }
    out
}

fn write_blob(out: &mut Vec<u8>, blob: &Blob) {
    out.extend_from_slice(b"blob\n");
    if let Some(mark) = blob.mark {
        out.extend_from_slice(format!("mark :{}\n", mark).as_bytes());
    }
    write_data(out, &blob.data);
}

fn write_data(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(format!("data {}\n", data.len()).as_bytes());
    out.extend_from_slice(data);
    out.push(b'\n');
}

fn write_commit(out: &mut Vec<u8>, commit: &Commit) {
    out.extend_from_slice(format!("commit {}\n", commit.branch).as_bytes());
    if let Some(mark) = commit.mark {
        out.extend_from_slice(format!("mark :{}\n", mark).as_bytes());
    }
    if let Some(author) = &commit.author {
        out.extend_from_slice(b"author ");
        write_person(out, author);
    }
    out.extend_from_slice(b"committer ");
    write_person(out, &commit.committer);
    write_data(out, &commit.message);
    if let Some(from) = &commit.from {
        out.extend_from_slice(format!("from {}\n", from).as_bytes());
    }
    for merge in &commit.merges {
        out.extend_from_slice(format!("merge {}\n", merge).as_bytes());
    }
    for change in &commit.file_changes {
        write_file_change(out, change);
    }
    out.push(b'\n');
}

fn write_person(out: &mut Vec<u8>, person: &PersonStamp) {
    if let Some(name) = &person.name {
        out.extend_from_slice(name.as_bytes());
        out.push(b' ');
    }
    out.push(b'<');
    out.extend_from_slice(person.email.as_bytes());
    out.extend_from_slice(b"> ");
    out.extend_from_slice(person.timestamp.to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(person.tz_offset.as_bytes());
    out.push(b'\n');
}

fn write_file_change(out: &mut Vec<u8>, change: &FileChange) {
    match change {
        FileChange::Modify(modify) => {
            out.extend_from_slice(b"M ");
            out.extend_from_slice(modify.mode.as_bytes());
            out.push(b' ');
            match &modify.dataref {
                DataRef::Mark(id) => out.extend_from_slice(format!(":{}", id).as_bytes()),
                DataRef::Sha1(sha) => out.extend_from_slice(sha.as_bytes()),
                DataRef::Inline(_) => out.extend_from_slice(b"inline"),
            }
            out.push(b' ');
            out.extend_from_slice(quote_path_if_needed(&modify.path).as_bytes());
            out.push(b'\n');
            if let DataRef::Inline(data) = &modify.dataref {
                write_data(out, data);
            }
        }
        FileChange::Delete { path } => {
            out.extend_from_slice(b"D ");
            out.extend_from_slice(quote_path_if_needed(path).as_bytes());
            out.push(b'\n');
        }
    }
}

fn write_reset(out: &mut Vec<u8>, reset: &Reset) {
    out.extend_from_slice(format!("reset {}\n", reset.branch).as_bytes());
    if let Some(from) = &reset.from {
        out.extend_from_slice(format!("from {}\n", from).as_bytes());
    }
}

fn quote_path_if_needed(path: &str) -> String {
    let needs_quoting = path.chars().any(|c| c == '"' || c == '\\' || (c as u32) < 0x20);
    if !needs_quoting {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for c in path.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\{:03o}", c as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// These tests parse a hand-written stream, round it all the way through the
// JSON Lines representation (to_json -> compact string -> json::parse ->
// from_json), write it back out, and check the result is byte-for-byte
// identical to the input. That's the property the whole tool exists for: if
// this ever breaks, going through JSON is lossy and nothing downstream can
// be trusted.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    fn round_trip(stream: &str) -> Vec<u8> {
        let opts = ParseOptions { lenient: false };
        let events = parse(stream.as_bytes(), &opts).expect("stream should parse");

        let mut jsonl = String::new();
        for event in &events {
            jsonl.push_str(&event.to_json().to_compact_string());
            jsonl.push('\n');
        }

        let mut rebuilt = Vec::new();
        for line in jsonl.lines() {
            let value = json::parse(line).expect("jsonl line should parse");
            rebuilt.push(Event::from_json(&value, false).expect("event should reparse"));
        }

        write(&rebuilt)
    }

    #[test]
    fn round_trips_blob_commit_merge_reset_done() {
        let stream = "blob\n\
mark :1\n\
data 5\n\
hello\n\
commit refs/heads/main\n\
mark :2\n\
author A U Thor <author@example.com> 1112911993 -0700\n\
committer C O Mitter <committer@example.com> 1112911993 -0700\n\
data 10\n\
first work\n\
M 100644 :1 file.txt\n\
\n\
commit refs/heads/main\n\
mark :3\n\
committer C O Mitter <committer@example.com> 1112912000 -0700\n\
data 11\n\
second work\n\
from :2\n\
merge :2\n\
M 100755 inline exec.sh\n\
data 6\n\
binary\n\
D old.txt\n\
\n\
reset refs/heads/main\n\
from :3\n\
done\n";

        assert_eq!(round_trip(stream), stream.as_bytes());
    }

    #[test]
    fn round_trips_minimal_commit_with_sha1_dataref() {
        let stream = "commit refs/heads/main\n\
committer C O Mitter <committer@example.com> 1112911993 -0700\n\
data 4\n\
fix!\n\
M 100644 da39a3ee5e6b4b0d3255bfef95601890afd80709 file.txt\n\
\n\
done\n";

        assert_eq!(round_trip(stream), stream.as_bytes());
    }
}
