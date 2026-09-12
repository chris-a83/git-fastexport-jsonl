mod fastexport;
mod json;
mod model;

use std::env;
use std::fs;
use std::io::{self, Read, Write};
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

    let input = match read_input(input_path.as_deref()) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("failed to read input: {}", err);
            return ExitCode::FAILURE;
        }
    };

    let result = match command {
        "to-jsonl" => to_jsonl(&input, lenient, redact_author.as_ref()),
        "to-fastexport" => to_fastexport(&input, lenient, redact_author.as_ref()),
        other => {
            eprintln!("unknown command: {}", other);
            print_usage();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(output) => match io::stdout().write_all(&output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("failed to write output: {}", err);
                ExitCode::FAILURE
            }
        },
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

fn read_input(path: Option<&str>) -> io::Result<Vec<u8>> {
    match path {
        None | Some("-") => {
            let mut buf = Vec::new();
            io::stdin().read_to_end(&mut buf)?;
            Ok(buf)
        }
        Some(p) => fs::read(p),
    }
}

fn to_jsonl(input: &[u8], lenient: bool, redact_author: Option<&(Option<String>, String)>) -> Result<Vec<u8>, String> {
    let opts = fastexport::ParseOptions { lenient };
    let mut events = fastexport::parse(input, &opts).map_err(|e| e.to_string())?;
    if let Some((name, email)) = redact_author {
        model::redact_authors(&mut events, name, email);
    }
    let mut out = Vec::new();
    for event in &events {
        out.extend_from_slice(event.to_json().to_compact_string().as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

fn to_fastexport(input: &[u8], lenient: bool, redact_author: Option<&(Option<String>, String)>) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(input).map_err(|_| "input is not valid UTF-8".to_string())?;
    let mut events = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = json::parse(trimmed).map_err(|e| format!("line {}: {}", i + 1, e))?;
        let event = model::Event::from_json(&value, lenient).map_err(|e| format!("line {}: {}", i + 1, e))?;
        events.push(event);
    }
    if let Some((name, email)) = redact_author {
        model::redact_authors(&mut events, name, email);
    }
    Ok(fastexport::write(&events))
}
