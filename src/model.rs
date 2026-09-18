use crate::json::Value;

pub struct PersonStamp {
    pub name: Option<String>,
    pub email: String,
    pub timestamp: i64,
    pub tz_offset: String,
}

pub enum DataRef {
    Mark(u64),
    Sha1(String),
    Inline(Vec<u8>),
}

pub struct FileModify {
    pub mode: String,
    pub dataref: DataRef,
    pub path: String,
}

pub enum FileChange {
    Modify(FileModify),
    Delete { path: String },
    Copy { src: String, dst: String },
    Rename { src: String, dst: String },
    DeleteAll,
    Note(NoteModify),
    Ls { path: String },
}

pub struct NoteModify {
    pub dataref: DataRef,
    pub commitish: String,
}

pub struct Commit {
    pub branch: String,
    pub mark: Option<u64>,
    pub author: Option<PersonStamp>,
    pub committer: PersonStamp,
    pub message: Vec<u8>,
    pub from: Option<String>,
    pub merges: Vec<String>,
    pub file_changes: Vec<FileChange>,
}

pub struct Blob {
    pub mark: Option<u64>,
    pub data: Vec<u8>,
}

pub struct Reset {
    pub branch: String,
    pub from: Option<String>,
}

pub struct Tag {
    pub name: String,
    pub mark: Option<u64>,
    pub from: String,
    pub tagger: Option<PersonStamp>,
    pub message: Vec<u8>,
}

pub enum Event {
    Blob(Blob),
    Commit(Commit),
    Reset(Reset),
    Tag(Tag),
    Ls { dataref: String, path: String },
    Checkpoint,
    Done,
}

impl Event {
    pub fn to_json(&self) -> Value {
        match self {
            Event::Blob(blob) => Value::Object(vec![
                ("type".to_string(), Value::String("blob".to_string())),
                ("mark".to_string(), optional_mark(blob.mark)),
                ("data_base64".to_string(), Value::String(base64_encode(&blob.data))),
            ]),
            Event::Reset(reset) => Value::Object(vec![
                ("type".to_string(), Value::String("reset".to_string())),
                ("branch".to_string(), Value::String(reset.branch.clone())),
                ("from".to_string(), optional_string(&reset.from)),
            ]),
            Event::Done => Value::Object(vec![("type".to_string(), Value::String("done".to_string()))]),
            Event::Checkpoint => Value::Object(vec![("type".to_string(), Value::String("checkpoint".to_string()))]),
            Event::Ls { dataref, path } => Value::Object(vec![
                ("type".to_string(), Value::String("ls".to_string())),
                ("dataref".to_string(), Value::String(dataref.clone())),
                ("path".to_string(), Value::String(path.clone())),
            ]),
            Event::Tag(tag) => {
                let tagger = match &tag.tagger {
                    Some(person) => person.to_json(),
                    None => Value::Null,
                };
                Value::Object(vec![
                    ("type".to_string(), Value::String("tag".to_string())),
                    ("name".to_string(), Value::String(tag.name.clone())),
                    ("mark".to_string(), optional_mark(tag.mark)),
                    ("from".to_string(), Value::String(tag.from.clone())),
                    ("tagger".to_string(), tagger),
                    ("message_base64".to_string(), Value::String(base64_encode(&tag.message))),
                ])
            }
            Event::Commit(commit) => {
                let author = match &commit.author {
                    Some(person) => person.to_json(),
                    None => Value::Null,
                };
                let merges = Value::Array(commit.merges.iter().map(|m| Value::String(m.clone())).collect());
                let file_changes =
                    Value::Array(commit.file_changes.iter().map(file_change_to_json).collect());
                Value::Object(vec![
                    ("type".to_string(), Value::String("commit".to_string())),
                    ("branch".to_string(), Value::String(commit.branch.clone())),
                    ("mark".to_string(), optional_mark(commit.mark)),
                    ("author".to_string(), author),
                    ("committer".to_string(), commit.committer.to_json()),
                    ("message_base64".to_string(), Value::String(base64_encode(&commit.message))),
                    ("from".to_string(), optional_string(&commit.from)),
                    ("merges".to_string(), merges),
                    ("file_changes".to_string(), file_changes),
                ])
            }
        }
    }

    pub fn from_json(value: &Value, lenient: bool) -> Result<Event, String> {
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing \"type\" field".to_string())?;
        match event_type {
            "blob" => {
                let mark = match value.get("mark") {
                    Some(Value::Number(n)) => Some(*n as u64),
                    _ => None,
                };
                let data_b64 = value
                    .get("data_base64")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "blob missing \"data_base64\"".to_string())?;
                Ok(Event::Blob(Blob { mark, data: base64_decode(data_b64)? }))
            }
            "reset" => {
                let branch = value
                    .get("branch")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "reset missing \"branch\"".to_string())?
                    .to_string();
                let from = value.get("from").and_then(Value::as_str).map(|s| s.to_string());
                Ok(Event::Reset(Reset { branch, from }))
            }
            "done" => Ok(Event::Done),
            "checkpoint" => Ok(Event::Checkpoint),
            "ls" => {
                let dataref = value
                    .get("dataref")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "ls missing \"dataref\"".to_string())?
                    .to_string();
                let path = value
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "ls missing \"path\"".to_string())?
                    .to_string();
                Ok(Event::Ls { dataref, path })
            }
            "tag" => {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "tag missing \"name\"".to_string())?
                    .to_string();
                let mark = match value.get("mark") {
                    Some(Value::Number(n)) => Some(*n as u64),
                    _ => None,
                };
                let from = value
                    .get("from")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "tag missing \"from\"".to_string())?
                    .to_string();
                let tagger = match value.get("tagger") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(PersonStamp::from_json(v, lenient)?),
                };
                let message_b64 = value
                    .get("message_base64")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "tag missing \"message_base64\"".to_string())?;
                let message = base64_decode(message_b64)?;
                Ok(Event::Tag(Tag { name, mark, from, tagger, message }))
            }
            "commit" => {
                let branch = value
                    .get("branch")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "commit missing \"branch\"".to_string())?
                    .to_string();
                let mark = match value.get("mark") {
                    Some(Value::Number(n)) => Some(*n as u64),
                    _ => None,
                };
                let author = match value.get("author") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(PersonStamp::from_json(v, lenient)?),
                };
                let committer_value = value
                    .get("committer")
                    .ok_or_else(|| "commit missing \"committer\"".to_string())?;
                let committer = PersonStamp::from_json(committer_value, lenient)?;
                let message_b64 = value
                    .get("message_base64")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "commit missing \"message_base64\"".to_string())?;
                let message = base64_decode(message_b64)?;
                let from = value.get("from").and_then(Value::as_str).map(|s| s.to_string());
                let merges = match value.get("merges") {
                    Some(Value::Array(items)) => items
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .ok_or_else(|| "merge entry must be a string".to_string())
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => Vec::new(),
                };
                let file_changes = match value.get("file_changes") {
                    Some(Value::Array(items)) => {
                        items.iter().map(file_change_from_json).collect::<Result<Vec<_>, _>>()?
                    }
                    _ => Vec::new(),
                };
                Ok(Event::Commit(Commit {
                    branch,
                    mark,
                    author,
                    committer,
                    message,
                    from,
                    merges,
                    file_changes,
                }))
            }
            other => Err(format!("unknown event type: {}", other)),
        }
    }
}

fn optional_mark(mark: Option<u64>) -> Value {
    match mark {
        Some(m) => Value::Number(m as i64),
        None => Value::Null,
    }
}

fn optional_string(value: &Option<String>) -> Value {
    match value {
        Some(s) => Value::String(s.clone()),
        None => Value::Null,
    }
}

impl PersonStamp {
    fn to_json(&self) -> Value {
        Value::Object(vec![
            ("name".to_string(), optional_string(&self.name)),
            ("email".to_string(), Value::String(self.email.clone())),
            ("timestamp".to_string(), Value::Number(self.timestamp)),
            ("tz_offset".to_string(), Value::String(self.tz_offset.clone())),
        ])
    }

    fn from_json(value: &Value, lenient: bool) -> Result<PersonStamp, String> {
        let name = value.get("name").and_then(Value::as_str).map(|s| s.to_string());
        let email = match value.get("email").and_then(Value::as_str) {
            Some(e) => e.to_string(),
            None if lenient => "unknown@invalid".to_string(),
            None => return Err("person stamp missing \"email\"".to_string()),
        };
        let timestamp = match value.get("timestamp").and_then(Value::as_i64) {
            Some(t) => t,
            None if lenient => 0,
            None => return Err("person stamp missing \"timestamp\"".to_string()),
        };
        let tz_offset = match value.get("tz_offset").and_then(Value::as_str) {
            Some(tz) if is_valid_tz_offset(tz) => tz.to_string(),
            _ if lenient => "+0000".to_string(),
            _ => return Err("person stamp has a missing or invalid \"tz_offset\"".to_string()),
        };
        Ok(PersonStamp { name, email, timestamp, tz_offset })
    }
}

// Replaces every author, committer, and tagger identity in the stream with a
// single fixed name/email, leaving timestamps and tz offsets untouched. This
// is the common "scrub real names before publishing history" case; anything
// more selective (redact one address, map old to new) is still a job for
// sed on the JSON Lines output.
pub fn redact_authors(events: &mut [Event], name: &Option<String>, email: &str) {
    for event in events {
        redact_author_event(event, name, email);
    }
}

pub fn redact_author_event(event: &mut Event, name: &Option<String>, email: &str) {
    match event {
        Event::Commit(commit) => {
            if let Some(author) = &mut commit.author {
                author.name = name.clone();
                author.email = email.to_string();
            }
            commit.committer.name = name.clone();
            commit.committer.email = email.to_string();
        }
        Event::Tag(tag) => {
            if let Some(tagger) = &mut tag.tagger {
                tagger.name = name.clone();
                tagger.email = email.to_string();
            }
        }
        Event::Blob(_) | Event::Reset(_) | Event::Ls { .. } | Event::Checkpoint | Event::Done => {}
    }
}

pub fn is_valid_tz_offset(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 5 && (bytes[0] == b'+' || bytes[0] == b'-') && bytes[1..].iter().all(u8::is_ascii_digit)
}

fn dataref_to_json(dataref: &DataRef) -> Value {
    match dataref {
        DataRef::Mark(id) => Value::Object(vec![
            ("kind".to_string(), Value::String("mark".to_string())),
            ("value".to_string(), Value::Number(*id as i64)),
        ]),
        DataRef::Sha1(sha) => Value::Object(vec![
            ("kind".to_string(), Value::String("sha1".to_string())),
            ("value".to_string(), Value::String(sha.clone())),
        ]),
        DataRef::Inline(data) => Value::Object(vec![
            ("kind".to_string(), Value::String("inline".to_string())),
            ("data_base64".to_string(), Value::String(base64_encode(data))),
        ]),
    }
}

fn file_change_to_json(change: &FileChange) -> Value {
    match change {
        FileChange::Modify(modify) => Value::Object(vec![
            ("op".to_string(), Value::String("M".to_string())),
            ("mode".to_string(), Value::String(modify.mode.clone())),
            ("dataref".to_string(), dataref_to_json(&modify.dataref)),
            ("path".to_string(), Value::String(modify.path.clone())),
        ]),
        FileChange::Delete { path } => Value::Object(vec![
            ("op".to_string(), Value::String("D".to_string())),
            ("path".to_string(), Value::String(path.clone())),
        ]),
        FileChange::Copy { src, dst } => Value::Object(vec![
            ("op".to_string(), Value::String("C".to_string())),
            ("src".to_string(), Value::String(src.clone())),
            ("dst".to_string(), Value::String(dst.clone())),
        ]),
        FileChange::Rename { src, dst } => Value::Object(vec![
            ("op".to_string(), Value::String("R".to_string())),
            ("src".to_string(), Value::String(src.clone())),
            ("dst".to_string(), Value::String(dst.clone())),
        ]),
        FileChange::DeleteAll => Value::Object(vec![("op".to_string(), Value::String("deleteall".to_string()))]),
        FileChange::Note(note) => Value::Object(vec![
            ("op".to_string(), Value::String("N".to_string())),
            ("dataref".to_string(), dataref_to_json(&note.dataref)),
            ("commitish".to_string(), Value::String(note.commitish.clone())),
        ]),
        FileChange::Ls { path } => Value::Object(vec![
            ("op".to_string(), Value::String("ls".to_string())),
            ("path".to_string(), Value::String(path.clone())),
        ]),
    }
}

fn file_change_from_json(value: &Value) -> Result<FileChange, String> {
    let op = value.get("op").and_then(Value::as_str).ok_or_else(|| "file change missing \"op\"".to_string())?;
    match op {
        "D" => {
            let path = value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "filedelete missing \"path\"".to_string())?
                .to_string();
            Ok(FileChange::Delete { path })
        }
        "C" => {
            let src = value
                .get("src")
                .and_then(Value::as_str)
                .ok_or_else(|| "filecopy missing \"src\"".to_string())?
                .to_string();
            let dst = value
                .get("dst")
                .and_then(Value::as_str)
                .ok_or_else(|| "filecopy missing \"dst\"".to_string())?
                .to_string();
            Ok(FileChange::Copy { src, dst })
        }
        "R" => {
            let src = value
                .get("src")
                .and_then(Value::as_str)
                .ok_or_else(|| "filerename missing \"src\"".to_string())?
                .to_string();
            let dst = value
                .get("dst")
                .and_then(Value::as_str)
                .ok_or_else(|| "filerename missing \"dst\"".to_string())?
                .to_string();
            Ok(FileChange::Rename { src, dst })
        }
        "deleteall" => Ok(FileChange::DeleteAll),
        "M" => {
            let mode = value
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| "filemodify missing \"mode\"".to_string())?
                .to_string();
            let path = value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "filemodify missing \"path\"".to_string())?
                .to_string();
            let dataref_value =
                value.get("dataref").ok_or_else(|| "filemodify missing \"dataref\"".to_string())?;
            let dataref = dataref_from_json(dataref_value)?;
            Ok(FileChange::Modify(FileModify { mode, dataref, path }))
        }
        "N" => {
            let dataref_value =
                value.get("dataref").ok_or_else(|| "notemodify missing \"dataref\"".to_string())?;
            let dataref = dataref_from_json(dataref_value)?;
            let commitish = value
                .get("commitish")
                .and_then(Value::as_str)
                .ok_or_else(|| "notemodify missing \"commitish\"".to_string())?
                .to_string();
            Ok(FileChange::Note(NoteModify { dataref, commitish }))
        }
        "ls" => {
            let path = value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "ls missing \"path\"".to_string())?
                .to_string();
            Ok(FileChange::Ls { path })
        }
        other => Err(format!("unknown file change op: {}", other)),
    }
}

fn dataref_from_json(value: &Value) -> Result<DataRef, String> {
    let kind = value.get("kind").and_then(Value::as_str).ok_or_else(|| "dataref missing \"kind\"".to_string())?;
    match kind {
        "mark" => {
            let id = value
                .get("value")
                .and_then(Value::as_i64)
                .ok_or_else(|| "mark dataref missing \"value\"".to_string())?;
            Ok(DataRef::Mark(id as u64))
        }
        "sha1" => {
            let sha = value
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| "sha1 dataref missing \"value\"".to_string())?
                .to_string();
            Ok(DataRef::Sha1(sha))
        }
        "inline" => {
            let data_b64 = value
                .get("data_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| "inline dataref missing \"data_base64\"".to_string())?;
            Ok(DataRef::Inline(base64_decode(data_b64)?))
        }
        other => Err(format!("unknown dataref kind: {}", other)),
    }
}

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 { B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64_ALPHABET[(n & 0x3f) as usize] as char } else { '=' });
    }
    out
}

pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("base64 input length must be a multiple of 4".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for quad in bytes.chunks(4) {
        let mut vals = [0u32; 4];
        let mut pad = 0;
        for (i, &b) in quad.iter().enumerate() {
            if b == b'=' {
                pad += 1;
            } else {
                vals[i] = base64_decode_char(b)?;
            }
        }
        let n = (vals[0] << 18) | (vals[1] << 12) | (vals[2] << 6) | vals[3];
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
    }
    Ok(out)
}

fn base64_decode_char(b: u8) -> Result<u32, String> {
    match b {
        b'A'..=b'Z' => Ok((b - b'A') as u32),
        b'a'..=b'z' => Ok((b - b'a' + 26) as u32),
        b'0'..=b'9' => Ok((b - b'0' + 52) as u32),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(format!("invalid base64 character: {}", b as char)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(name: &str, email: &str) -> PersonStamp {
        PersonStamp {
            name: Some(name.to_string()),
            email: email.to_string(),
            timestamp: 1112911993,
            tz_offset: "-0700".to_string(),
        }
    }

    #[test]
    fn redact_authors_overwrites_commit_and_tag_identities_but_not_timestamps() {
        let mut events = vec![
            Event::Commit(Commit {
                branch: "refs/heads/main".to_string(),
                mark: Some(1),
                author: Some(stamp("A U Thor", "author@example.com")),
                committer: stamp("C O Mitter", "committer@example.com"),
                message: b"work".to_vec(),
                from: None,
                merges: Vec::new(),
                file_changes: Vec::new(),
            }),
            Event::Tag(Tag {
                name: "v1.0".to_string(),
                mark: None,
                from: ":1".to_string(),
                tagger: Some(stamp("T Agger", "tagger@example.com")),
                message: b"release".to_vec(),
            }),
        ];

        redact_authors(&mut events, &Some("Anonymous".to_string()), "anon@example.com");

        match &events[0] {
            Event::Commit(commit) => {
                let author = commit.author.as_ref().unwrap();
                assert_eq!(author.name.as_deref(), Some("Anonymous"));
                assert_eq!(author.email, "anon@example.com");
                assert_eq!(author.timestamp, 1112911993);
                assert_eq!(commit.committer.name.as_deref(), Some("Anonymous"));
                assert_eq!(commit.committer.email, "anon@example.com");
            }
            _ => panic!("expected a commit event"),
        }
        match &events[1] {
            Event::Tag(tag) => {
                let tagger = tag.tagger.as_ref().unwrap();
                assert_eq!(tagger.name.as_deref(), Some("Anonymous"));
                assert_eq!(tagger.email, "anon@example.com");
            }
            _ => panic!("expected a tag event"),
        }
    }

    #[test]
    fn redact_authors_leaves_commits_without_an_author_line_absent() {
        let mut events = vec![Event::Commit(Commit {
            branch: "refs/heads/main".to_string(),
            mark: None,
            author: None,
            committer: stamp("C O Mitter", "committer@example.com"),
            message: b"work".to_vec(),
            from: None,
            merges: Vec::new(),
            file_changes: Vec::new(),
        })];

        redact_authors(&mut events, &None, "anon@example.com");

        match &events[0] {
            Event::Commit(commit) => {
                assert!(commit.author.is_none());
                assert_eq!(commit.committer.name, None);
                assert_eq!(commit.committer.email, "anon@example.com");
            }
            _ => panic!("expected a commit event"),
        }
    }
}
