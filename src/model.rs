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

pub enum Event {
    Blob(Blob),
    Commit(Commit),
    Reset(Reset),
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

pub fn is_valid_tz_offset(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 5 && (bytes[0] == b'+' || bytes[0] == b'-') && bytes[1..].iter().all(u8::is_ascii_digit)
}

fn file_change_to_json(change: &FileChange) -> Value {
    match change {
        FileChange::Modify(modify) => {
            let dataref = match &modify.dataref {
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
            };
            Value::Object(vec![
                ("op".to_string(), Value::String("M".to_string())),
                ("mode".to_string(), Value::String(modify.mode.clone())),
                ("dataref".to_string(), dataref),
                ("path".to_string(), Value::String(modify.path.clone())),
            ])
        }
        FileChange::Delete { path } => Value::Object(vec![
            ("op".to_string(), Value::String("D".to_string())),
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
            let kind = dataref_value
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| "dataref missing \"kind\"".to_string())?;
            let dataref = match kind {
                "mark" => {
                    let id = dataref_value
                        .get("value")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| "mark dataref missing \"value\"".to_string())?;
                    DataRef::Mark(id as u64)
                }
                "sha1" => {
                    let sha = dataref_value
                        .get("value")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "sha1 dataref missing \"value\"".to_string())?
                        .to_string();
                    DataRef::Sha1(sha)
                }
                "inline" => {
                    let data_b64 = dataref_value
                        .get("data_base64")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "inline dataref missing \"data_base64\"".to_string())?;
                    DataRef::Inline(base64_decode(data_b64)?)
                }
                other => return Err(format!("unknown dataref kind: {}", other)),
            };
            Ok(FileChange::Modify(FileModify { mode, dataref, path }))
        }
        other => Err(format!("unknown file change op: {}", other)),
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
