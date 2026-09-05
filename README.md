# fastexport-jsonl

`git fast-export` is the closest thing git has to a portable, complete
representation of a repository's history: every blob, commit, and ref update
as a flat stream of commands. It's great for feeding into `git fast-import`,
and useless for almost anything else. You can't grep it sensibly (paths and
messages are mixed in with binary-length-prefixed data blocks), you can't
filter it with `jq`, and writing a script against it means hand-rolling a
parser for a format that has more edge cases than its size suggests.

This converts a fast-export stream into JSON Lines — one JSON object per
event — and back again. Once history is JSON Lines, rewriting author emails,
grepping commit messages, or piping through `jq` before feeding the result
back into `git fast-import` all become normal text-processing tasks.

## Usage

```
git fast-export --all | fastexport-jsonl to-jsonl > history.jsonl
```

Each line of `history.jsonl` is one event:

```json
{"type":"commit","branch":"refs/heads/main","mark":3,"author":{"name":"A U Thor","email":"author@example.com","timestamp":1112911993,"tz_offset":"-0700"},"committer":{...},"message_base64":"Zml4IHRoZSB0aGluZw==","from":":2","merges":[],"file_changes":[{"op":"M","mode":"100644","dataref":{"kind":"mark","value":4},"path":"src/lib.rs"}]}
```

Blob content and commit messages are base64-encoded (`data_base64` /
`message_base64`) so binary blobs round-trip exactly instead of being mangled
by a UTF-8 conversion.

To rewrite something and rebuild a repo from the result:

```
sed 's/old@example\.com/new@example.com/g' history.jsonl > rewritten.jsonl
fastexport-jsonl to-fastexport rewritten.jsonl | git fast-import
```

## Strict by default

The point of going through an intermediate format is trust: if the converter
silently drops or mangles something, every downstream script is now working
from a lie. So by default this refuses to guess:

- an `author`/`committer` line must have `Name <email> <unix-ts> <tz-offset>`
  with a valid `+HHMM`/`-HHMM` offset
- `from`/`merge` references to a mark (`:N`) must point at a mark already
  seen earlier in the stream
- file modes must be one of the modes git actually uses
  (`100644`, `100755`, `120000`, `160000`, `040000`)
- sha1 datarefs must be 40 hex characters
- unrecognized top-level commands are a hard error, not a skip
- a `data <len>` payload must be followed by the newline separator the format
  expects (when there's more stream left to read)

Any of these being wrong is treated as a sign the input is either corrupt or
from something that doesn't speak the format correctly, and conversion stops
with a line-numbered error rather than emitting a partial or guessed result.

Pass `--lenient` to relax all of the above with defined fallbacks instead of
failing: missing timezone becomes `+0000`, missing email becomes
`unknown@invalid`, unresolvable mark references are passed through as-is,
unrecognized commands are skipped, and a missing trailing newline after a
data block is tolerated. Useful when the input came from a hand-edited
stream, an older git, or a third-party tool that's close enough to spec but
not exact.

```
fastexport-jsonl to-jsonl --lenient < weird-export.stream > history.jsonl
```

## Building

```
cargo build --release
```

No third-party dependencies; the JSON encoder/decoder and base64 codec used
for blob data are both hand-rolled in `src/`.

## Current limitations

- `ls` and `checkpoint` commands aren't parsed yet (strict mode errors on
  them, lenient mode skips the line)
- the whole stream is read into memory rather than processed incrementally

Both `data` forms are accepted on input — the exact byte count form and the
delimited `data <<DELIM` form some tools emit — but output always uses the
byte count form, since it's unambiguous.

## License

MIT, see LICENSE.
