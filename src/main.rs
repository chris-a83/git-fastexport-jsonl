mod fastexport;
mod json;
mod model;

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return ExitCode::from(2);
    }

    let command = args[1].as_str();
    if command == "-h" || command == "--help" {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let mut lenient = false;
    let mut redact_author: Option<(Option<String>, String)> = None;
    let mut input_path: Option<String> = None;
    for arg in &args[2..] {
        match arg.as_str() {
            "--lenient" => lenient = true,
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--redact-author=") => {
                let spec = &other["--redact-author=".len()..];
                match parse_redact_author(spec) {
                    Ok(parsed) => redact_author = Some(parsed),
                    Err(err) => {
                        eprintln!("{}", err);
                        return ExitCode::from(2);
                    }
                }
            }
            other if input_path.is_none() => input_path = Some(other.to_string()),
            other => {
                eprintln!("unexpected argument: {}", other);
                return ExitCode::from(2);
            }
        }
    }

    let input = match open_input(input_path.as_deref()) {
        Ok(reader) => reader,
        Err(err) => {
            eprintln!("failed to read input: {}", err);
            return ExitCode::FAILURE;
        }
    };

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let result = match command {
        "to-jsonl" => to_jsonl(input, lenient, redact_author.as_ref(), &mut out),
        "to-fastexport" => to_fastexport(input, lenient, redact_author.as_ref(), &mut out),
        other => {
            eprintln!("unknown command: {}", other);
            print_usage();
            return ExitCode::from(2);
        }
    };

    match result.and_then(|()| out.flush().map_err(|e| format!("failed to write output: {}", e))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{}", message);
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!("usage: fastexport-jsonl <to-jsonl|to-fastexport> [--lenient] [--redact-author=\"Name <email>\"] [<file>]");
    eprintln!();
    eprintln!("  to-jsonl        read a git fast-export stream, write JSON Lines");
    eprintln!("  to-fastexport   read JSON Lines, write a git fast-export/import stream");
    eprintln!("  --lenient       tolerate common format deviations instead of failing");
    eprintln!("  --redact-author replace every author/committer/tagger name and email");
    eprintln!("                  with the given identity; timestamps are left alone");
    eprintln!("  <file>          input file path, or omit it (or pass '-') for stdin");
}

fn parse_redact_author(spec: &str) -> Result<(Option<String>, String), String> {
    let open = spec.find('<');
    let close = spec.rfind('>');
    match (open, close) {
        (Some(o), Some(c)) if o < c => {
            let name = spec[..o].trim();
            let name = if name.is_empty() { None } else { Some(name.to_string()) };
            let email = spec[o + 1..c].trim();
            if email.is_empty() {
                return Err("--redact-author email must not be empty".to_string());
            }
            Ok((name, email.to_string()))
        }
        _ => Err(format!("--redact-author value must look like \"Name <email>\": {}", spec)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_redact_author_accepts_name_and_email() {
        let (name, email) = parse_redact_author("Anonymous <anon@example.com>").unwrap();
        assert_eq!(name.as_deref(), Some("Anonymous"));
        assert_eq!(email, "anon@example.com");
    }

    #[test]
    fn parse_redact_author_accepts_email_only() {
        let (name, email) = parse_redact_author("<anon@example.com>").unwrap();
        assert_eq!(name, None);
        assert_eq!(email, "anon@example.com");
    }

    #[test]
    fn parse_redact_author_rejects_missing_angle_brackets() {
        assert!(parse_redact_author("anon@example.com").is_err());
    }

    #[test]
    fn parse_redact_author_rejects_empty_email() {
        assert!(parse_redact_author("Anonymous <>").is_err());
    }
}

// The input is never read up front: the parsers pull from this reader as they
// go, so memory use doesn't scale with the size of the stream.
fn open_input(path: Option<&str>) -> io::Result<Box<dyn BufRead>> {
    match path {
        None | Some("-") => Ok(Box::new(io::BufReader::with_capacity(64 * 1024, io::stdin()))),
        Some(p) => Ok(Box::new(io::BufReader::with_capacity(64 * 1024, fs::File::open(p)?))),
    }
}

// Both directions hand each event to the writer as soon as it's ready
// instead of collecting the whole history into a Vec<Event> and then a
// second Vec<u8> of output: the two buffers used to be alive at once, each
// roughly the size of the full converted history.
fn to_jsonl(
    input: impl BufRead,
    lenient: bool,
    redact_author: Option<&(Option<String>, String)>,
    out: &mut impl Write,
) -> Result<(), String> {
    let opts = fastexport::ParseOptions { lenient };
    fastexport::parse_each(input, &opts, |mut event| {
        if let Some((name, email)) = redact_author {
            model::redact_author_event(&mut event, name, email);
        }
        let line = event.to_json().to_compact_string();
        out.write_all(line.as_bytes())
            .and_then(|()| out.write_all(b"\n"))
            .map_err(|e| fastexport::ParseError { line: 0, message: format!("failed to write output: {}", e) })
    })
    .map_err(|e| e.to_string())
}

fn to_fastexport(
    mut input: impl BufRead,
    lenient: bool,
    redact_author: Option<&(Option<String>, String)>,
    out: &mut impl Write,
) -> Result<(), String> {
    let mut buf = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    loop {
        text.clear();
        // read_line reports invalid UTF-8 as InvalidData, which keeps the
        // old "input is not valid UTF-8" diagnosis for that case.
        match input.read_line(&mut text) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return Err(format!("line {}: input is not valid UTF-8", i + 1));
            }
            Err(e) => return Err(format!("failed to read input: {}", e)),
        }
        // str::lines counted a \r\n or \n as one line; i tracks the same thing.
        i += 1;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = json::parse(trimmed).map_err(|e| format!("line {}: {}", i, e))?;
        let mut event = model::Event::from_json(&value, lenient).map_err(|e| format!("line {}: {}", i, e))?;
        if let Some((name, email)) = redact_author {
            model::redact_author_event(&mut event, name, email);
        }
        buf.clear();
        fastexport::write_event(&mut buf, &event);
        out.write_all(&buf).map_err(|e| format!("line {}: failed to write output: {}", i, e))?;
    }
    Ok(())
}
