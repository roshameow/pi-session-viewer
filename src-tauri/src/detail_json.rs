//! Bounded JSONL projection. The original record remains the lossless body source.
//! No record/string is read wholesale: even discarded data is decoded and validated.
//! Limits are per record, including duplicate fields (not just the final map).
use serde_json::{Map, Value};
use std::io::{BufRead, BufReader, Read};

const BUFFER_BYTES: usize = 16 * 1024;
const STRING_BYTES: usize = 4096;
const BODY_BYTES: usize = 8192;
const METADATA_BYTES: usize = 64 * 1024;
const BODY_BLOCKS: usize = 16;
const MAX_DEPTH: usize = 128;

pub(super) struct Projection {
    pub value: Value,
    pub matched: bool,
    pub truncated: bool,
}

pub(super) fn project(
    reader: impl Read,
    preview: bool,
    query: Option<&str>,
) -> Result<Projection, String> {
    let mut parser = Parser {
        input: BufReader::with_capacity(BUFFER_BYTES, reader),
        preview,
        matcher: Matcher::new(query),
        body_bytes: 0,
        metadata_bytes: 0,
        body_blocks: 0,
        truncated: false,
    };
    parser.whitespace()?;
    if parser.peek()? != Some(b'{') {
        return Err("JSON record must be an object".into());
    }
    let value = parser.value(Mode::Root, 0)?.unwrap_or(Value::Null);
    parser.whitespace()?;
    if parser.peek()?.is_some() {
        return Err("trailing data after JSON record".into());
    }
    Ok(Projection {
        value,
        matched: parser.matcher.matched,
        truncated: parser.truncated,
    })
}

// A KMP matcher over decoded, lowercased Unicode scalars. State resets between
// fields; a match cannot accidentally join two unrelated strings/argument keys.
// Space is O(query), not O(record). Unicode lowercase expansions are supported.
struct Matcher {
    needle: Vec<char>,
    failure: Vec<usize>,
    state: usize,
    matched: bool,
}
impl Matcher {
    fn new(query: Option<&str>) -> Self {
        let needle: Vec<char> = query
            .unwrap_or("")
            .chars()
            .flat_map(char::to_lowercase)
            .collect();
        let mut failure = vec![0; needle.len()];
        for i in 1..needle.len() {
            let mut j = failure[i - 1];
            while j > 0 && needle[i] != needle[j] {
                j = failure[j - 1];
            }
            if needle[i] == needle[j] {
                j += 1;
            }
            failure[i] = j;
        }
        Self {
            needle,
            failure,
            state: 0,
            matched: false,
        }
    }
    fn reset(&mut self) {
        self.state = 0;
    }
    fn feed(&mut self, ch: char) {
        if self.matched || self.needle.is_empty() {
            return;
        }
        for ch in ch.to_lowercase() {
            while self.state > 0 && self.needle[self.state] != ch {
                self.state = self.failure[self.state - 1];
            }
            if self.needle[self.state] == ch {
                self.state += 1;
            }
            if self.state == self.needle.len() {
                self.matched = true;
                return;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Root,
    Message,
    Block(bool),
    Content(bool),
    Stats,
    Metadata(bool), // searchable string or scalar, never arbitrary containers
    Bulk(bool),
    Skip(bool), // recursively search only inside arguments
}
#[derive(Clone, Copy)]
enum Keep {
    Key, // only known schema keys need recognition; arbitrary keys are skipped
    Metadata,
    Bulk(bool),
    None,
}
struct Parser<R> {
    input: BufReader<R>,
    preview: bool,
    matcher: Matcher,
    body_bytes: usize,
    metadata_bytes: usize,
    body_blocks: usize,
    truncated: bool,
}
impl<R: Read> Parser<R> {
    fn peek(&mut self) -> Result<Option<u8>, String> {
        self.input
            .fill_buf()
            .map(|b| b.first().copied())
            .map_err(|e| format!("JSON read: {e}"))
    }
    fn take(&mut self) -> Result<u8, String> {
        let byte = self
            .peek()?
            .ok_or_else(|| "truncated JSON record".to_string())?;
        self.input.consume(1);
        Ok(byte)
    }
    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.take()? != byte {
            return Err(format!("invalid JSON: expected '{}'", byte as char));
        }
        Ok(())
    }
    fn whitespace(&mut self) -> Result<(), String> {
        while matches!(self.peek()?, Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.input.consume(1);
        }
        Ok(())
    }
    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(bytes)
            .ok_or_else(|| "METADATA_LIMIT: JSON metadata exceeds 65536 bytes".to_string())?;
        if self.metadata_bytes > METADATA_BYTES {
            return Err(
                "METADATA_LIMIT: JSON metadata exceeds 65536 bytes; identities cannot be truncated"
                    .into(),
            );
        }
        Ok(())
    }
    fn value(&mut self, mode: Mode, depth: usize) -> Result<Option<Value>, String> {
        self.whitespace()?;
        match self.peek()? {
            Some(b'{') => {
                if depth >= MAX_DEPTH {
                    return Err("JSON nesting exceeds 128".into());
                }
                match mode {
                    Mode::Root | Mode::Message | Mode::Block(_) | Mode::Stats | Mode::Skip(_) => {
                        self.object(mode, depth + 1)
                    }
                    _ => {
                        self.truncated = true;
                        self.object(Mode::Skip(false), depth + 1)
                    }
                }
            }
            Some(b'[') => {
                if depth >= MAX_DEPTH {
                    return Err("JSON nesting exceeds 128".into());
                }
                let mode = match mode {
                    Mode::Content(_) | Mode::Skip(_) => mode,
                    _ => {
                        self.truncated = true;
                        Mode::Skip(false)
                    }
                };
                self.array(mode, depth + 1)
            }
            Some(b'"') => {
                let (keep, search) = match mode {
                    Mode::Metadata(search) => (Keep::Metadata, search),
                    Mode::Content(body) | Mode::Bulk(body) => (Keep::Bulk(body), true),
                    Mode::Skip(search) => (Keep::None, search),
                    _ => (Keep::None, false),
                };
                let s = self.string(keep, search)?;
                Ok(if matches!(keep, Keep::None) {
                    None
                } else {
                    Some(Value::String(s))
                })
            }
            Some(b'-' | b'0'..=b'9') => self.number(mode),
            Some(b't') => {
                self.literal(b"true")?;
                self.scalar(mode, Value::Bool(true))
            }
            Some(b'f') => {
                self.literal(b"false")?;
                self.scalar(mode, Value::Bool(false))
            }
            Some(b'n') => {
                self.literal(b"null")?;
                self.scalar(mode, Value::Null)
            }
            _ => Err("invalid JSON value".into()),
        }
    }
    fn scalar(&mut self, mode: Mode, value: Value) -> Result<Option<Value>, String> {
        if matches!(mode, Mode::Metadata(_) | Mode::Stats) {
            self.charge(16)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
    fn literal(&mut self, bytes: &[u8]) -> Result<(), String> {
        for &byte in bytes {
            self.expect(byte)?;
        }
        Ok(())
    }
    fn field(&self, mode: Mode, key: &str) -> Mode {
        match mode {
            Mode::Skip(search) => Mode::Skip(search),
            Mode::Stats => {
                if key == "cost" {
                    Mode::Stats
                } else {
                    Mode::Metadata(false)
                }
            }
            Mode::Root | Mode::Message | Mode::Block(_) => {
                let body = match mode {
                    Mode::Block(body) => body,
                    _ => self.preview,
                };
                match key {
                    "text" | "thinking" | "summary" | "output" | "command" => Mode::Bulk(body),
                    "content" => Mode::Content(body),
                    "arguments" => Mode::Skip(true),
                    "name" | "label" => Mode::Metadata(true),
                    "id" | "parentId" | "type" | "timestamp" | "cwd" | "role" | "model"
                    | "provider" | "toolCallId" | "toolName" | "isError" | "modelId"
                    | "thinkingLevel" | "targetId" | "tokensBefore" | "customType" | "exitCode"
                    | "truncated" | "stopReason" => Mode::Metadata(false),
                    "message" if matches!(mode, Mode::Root) => Mode::Message,
                    "usage" | "cost" => Mode::Stats,
                    _ => Mode::Skip(false),
                }
            }
            _ => Mode::Skip(false),
        }
    }
    fn object(&mut self, mode: Mode, depth: usize) -> Result<Option<Value>, String> {
        self.expect(b'{')?;
        let retain = !matches!(mode, Mode::Skip(_));
        if retain {
            self.charge(32)?;
        }
        let mut map = Map::new();
        self.whitespace()?;
        if self.peek()? == Some(b'}') {
            self.take()?;
            return Ok(retain.then_some(Value::Object(map)));
        }
        loop {
            self.whitespace()?;
            let key_keep = if matches!(mode, Mode::Stats) {
                Keep::Metadata
            } else {
                Keep::Key
            };
            let key = self.string(key_keep, matches!(mode, Mode::Skip(true)))?;
            self.whitespace()?;
            self.expect(b':')?;
            let field = self.field(mode, &key);
            // Stats are numeric, with a nested cost object. Strings in usage are
            // unknown bulk, not metadata to accidentally retain (or allocate).
            self.whitespace()?;
            let field = if matches!(mode, Mode::Stats)
                && !matches!(self.peek()?, Some(b'-' | b'0'..=b'9' | b'{'))
            {
                Mode::Skip(false)
            } else {
                field
            };
            if retain && matches!(field, Mode::Skip(_)) {
                self.truncated = true;
            }
            if let Some(value) = self.value(field, depth)? {
                self.charge(key.len() + 32)?;
                map.insert(key, value);
            } else if retain {
                // Match JSON's last-key-wins semantics even when the final
                // occurrence has an unsupported shape and must be discarded.
                map.remove(&key);
                self.truncated = true;
            }
            self.whitespace()?;
            match self.take()? {
                b'}' => break,
                b',' => {}
                _ => return Err("invalid JSON object separator".into()),
            }
        }
        Ok(retain.then_some(Value::Object(map)))
    }
    fn array(&mut self, mode: Mode, depth: usize) -> Result<Option<Value>, String> {
        self.expect(b'[')?;
        let retain = matches!(mode, Mode::Content(_));
        let mut values = Vec::new();
        self.whitespace()?;
        if self.peek()? == Some(b']') {
            self.take()?;
            return Ok(retain.then_some(Value::Array(values)));
        }
        loop {
            self.whitespace()?;
            if let Mode::Content(body) = mode {
                let body = body && self.body_blocks < BODY_BLOCKS;
                let item_mode = if self.peek()? == Some(b'{') {
                    Mode::Block(body)
                } else {
                    Mode::Bulk(body)
                };
                // Block metadata is temporary until its type is known. Only
                // retained blocks consume the record's metadata budget.
                let metadata_before = self.metadata_bytes;
                let value = self.value(item_mode, depth)?;
                let mut kept = false;
                let tool = value
                    .as_ref()
                    .and_then(|v| v.get("type"))
                    .and_then(Value::as_str)
                    == Some("toolCall");
                if tool || body {
                    if let Some(value) = value {
                        // Images/unknown block types carry no useful preview.
                        let useful = tool
                            || value.is_string()
                            || matches!(
                                value.get("type").and_then(Value::as_str),
                                Some("text" | "thinking")
                            );
                        if useful {
                            self.charge(16)?; // bounds even empty tool-call slots
                            values.push(value);
                            kept = true;
                            if !tool {
                                self.body_blocks += 1;
                            }
                        } else {
                            self.truncated = true;
                        }
                    }
                } else {
                    self.truncated = true;
                }
                if !kept {
                    self.metadata_bytes = metadata_before;
                }
            } else {
                self.value(mode, depth)?;
            }
            self.whitespace()?;
            match self.take()? {
                b']' => break,
                b',' => {}
                _ => return Err("invalid JSON array separator".into()),
            }
        }
        Ok(retain.then_some(Value::Array(values)))
    }
    fn hex4(&mut self) -> Result<u16, String> {
        let mut code = 0;
        for _ in 0..4 {
            let digit = match self.take()? {
                b @ b'0'..=b'9' => b - b'0',
                b @ b'a'..=b'f' => b - b'a' + 10,
                b @ b'A'..=b'F' => b - b'A' + 10,
                _ => return Err("invalid JSON Unicode escape".into()),
            };
            code = (code << 4) | u16::from(digit);
        }
        Ok(code)
    }
    fn string(&mut self, keep: Keep, search: bool) -> Result<String, String> {
        self.expect(b'"')?;
        if search {
            self.matcher.reset();
        }
        let mut out = String::new();
        let mut clipped = false;
        loop {
            let byte = self.take()?;
            let ch = match byte {
                b'"' => break,
                b'\\' => match self.take()? {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        let high = self.hex4()?;
                        let code = match high {
                            0xd800..=0xdbff => {
                                self.expect(b'\\')?;
                                self.expect(b'u')?;
                                let low = self.hex4()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err("invalid JSON surrogate pair".into());
                                }
                                0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(low)
                                    - 0xdc00
                            }
                            0xdc00..=0xdfff => return Err("unpaired JSON low surrogate".into()),
                            _ => u32::from(high),
                        };
                        char::from_u32(code)
                            .ok_or_else(|| "invalid JSON Unicode scalar".to_string())?
                    }
                    _ => return Err("invalid JSON escape".into()),
                },
                0..=0x1f => return Err("unescaped control character in JSON string".into()),
                0x20..=0x7f => byte as char,
                _ => {
                    let length = match byte {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => return Err("invalid UTF-8 in JSON string".into()),
                    };
                    let mut bytes = [0; 4];
                    bytes[0] = byte;
                    for slot in &mut bytes[1..length] {
                        *slot = self.take()?;
                    }
                    std::str::from_utf8(&bytes[..length])
                        .map_err(|_| "invalid UTF-8 in JSON string")?
                        .chars()
                        .next()
                        .unwrap()
                }
            };
            if search {
                self.matcher.feed(ch);
            }
            let size = ch.len_utf8();
            match keep {
                Keep::Metadata => {
                    if out.len() + size > STRING_BYTES {
                        return Err(
                            "METADATA_LIMIT: JSON metadata string exceeds 4096 bytes".into()
                        );
                    }
                    self.charge(size)?;
                    out.push(ch);
                }
                Keep::Bulk(body) => {
                    if body && !clipped && self.body_bytes + size <= BODY_BYTES {
                        self.body_bytes += size;
                        out.push(ch);
                    } else {
                        clipped = true;
                        self.truncated = true;
                    }
                }
                Keep::Key => {
                    if !clipped && out.len() + size <= 64 {
                        out.push(ch);
                    } else {
                        clipped = true;
                    }
                }
                Keep::None => {}
            }
        }
        // An oversized unknown key cannot alias a recognized prefix.
        if matches!(keep, Keep::Key) && clipped {
            out.clear();
        }
        Ok(out)
    }
    fn number(&mut self, mode: Mode) -> Result<Option<Value>, String> {
        let retain = matches!(mode, Mode::Metadata(_) | Mode::Stats);
        let search = matches!(mode, Mode::Skip(true));
        if search {
            self.matcher.reset();
        }
        let mut out = String::new();
        if self.peek()? == Some(b'-') {
            self.number_byte(retain, search, &mut out)?;
        }
        match self.peek()? {
            Some(b'0') => {
                self.number_byte(retain, search, &mut out)?;
            }
            Some(b'1'..=b'9') => self.digits(retain, search, &mut out)?,
            _ => return Err("invalid JSON number".into()),
        }
        if self.peek()? == Some(b'.') {
            self.number_byte(retain, search, &mut out)?;
            self.digits(retain, search, &mut out)?;
        }
        if matches!(self.peek()?, Some(b'e' | b'E')) {
            self.number_byte(retain, search, &mut out)?;
            if matches!(self.peek()?, Some(b'+' | b'-')) {
                self.number_byte(retain, search, &mut out)?;
            }
            self.digits(retain, search, &mut out)?;
        }
        if retain {
            let number = serde_json::from_str::<serde_json::Number>(&out)
                .map_err(|e| format!("METADATA_LIMIT: JSON metadata number: {e}"))?;
            self.charge(out.len() + 16)?;
            Ok(Some(Value::Number(number)))
        } else {
            Ok(None)
        }
    }
    fn digits(&mut self, retain: bool, search: bool, out: &mut String) -> Result<(), String> {
        if !matches!(self.peek()?, Some(b'0'..=b'9')) {
            return Err("invalid JSON number digits".into());
        }
        while matches!(self.peek()?, Some(b'0'..=b'9')) {
            self.number_byte(retain, search, out)?;
        }
        Ok(())
    }
    fn number_byte(&mut self, retain: bool, search: bool, out: &mut String) -> Result<(), String> {
        let byte = self.take()?;
        if search {
            self.matcher.feed(byte as char);
        }
        if retain {
            if out.len() >= STRING_BYTES {
                return Err("METADATA_LIMIT: JSON metadata number exceeds 4096 bytes".into());
            }
            out.push(byte as char);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Verified standalone method: compile a #[path] wrapper with rustc --test,
    // -L dependency=target/release/deps and --extern serde_json=<matching rlib>.
    // Use the same toolchain as the rlib: Homebrew rustc 1.86 reports E0514 for
    // this repository's artifacts, while ~/.cargo/bin/rustc 1.98.1 works.
    // All fixtures are synthetic; tests do not discover or read session files.
    use super::*;
    use serde_json::json;
    fn parse(input: &str, preview: bool, query: Option<&str>) -> Projection {
        project(input.as_bytes(), preview, query).unwrap()
    }
    #[test]
    fn metadata_and_stats_survive_without_bodies() {
        let source = json!({"type":"message","id":"entry","parentId":"parent","timestamp":"now","cwd":"/synthetic",
            "modelId":"model","thinkingLevel":"high","name":"Name","label":"Label","targetId":"target",
            "tokensBefore":42,"customType":"custom","message":{"role":"assistant","model":"m","provider":"p",
            "toolCallId":"result","toolName":"codemode","isError":false,"command":"cmd","output":"output",
            "usage":{"input":10,"output":20,"cacheRead":30,"cacheWrite":40,"totalTokens":100,
            "cost":{"input":0.1,"output":0.2,"total":0.3}},"content":[{"type":"text","text":"discard"},
            {"type":"toolCall","id":"call","name":"codemode","arguments":{"code":"return 123"}}]}}).to_string();
        let p = parse(&source, false, Some("RETURN"));
        assert!(p.matched && p.truncated);
        assert_eq!(p.value["id"], "entry");
        assert_eq!(p.value["tokensBefore"], 42);
        assert_eq!(p.value["message"]["usage"]["cost"]["total"], 0.3);
        assert_eq!(p.value["message"]["content"].as_array().unwrap().len(), 1);
        assert_eq!(p.value["message"]["content"][0]["id"], "call");
        assert!(p.value["message"]["content"][0].get("arguments").is_none());
        assert_eq!(p.value["message"]["command"], "");
        let header = parse(
            r#"{"type":"session","id":"session-id","cwd":"/synthetic"}"#,
            false,
            None,
        );
        assert_eq!(header.value["id"], "session-id");
    }
    #[test]
    fn giant_suffix_search_and_total_preview_budget() {
        let text = "界".repeat(100_000);
        let source = format!(
            r#"{{"id":"x","summary":"{text}SUFFIX","message":{{"content":[{{"type":"text","text":"{text}"}},{{"type":"thinking","thinking":"end"}}]}}}}"#
        );
        for preview in [false, true] {
            let p = parse(&source, preview, Some("suffix"));
            assert!(p.matched && p.truncated);
            let summary = p.value["summary"].as_str().unwrap();
            assert!(summary.len() <= BODY_BYTES);
            if preview {
                assert_eq!(summary.len(), 8190);
                let blocks = p.value["message"]["content"].as_array().unwrap();
                let total = summary.len()
                    + blocks[0]["text"].as_str().unwrap().len()
                    + blocks[1]["thinking"].as_str().unwrap().len();
                assert!(total <= BODY_BYTES);
            } else {
                assert!(summary.is_empty());
            }
        }
    }
    #[test]
    fn unicode_escapes_and_split_reader() {
        struct Tiny<'a>(&'a [u8]);
        impl Read for Tiny<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() || out.is_empty() {
                    return Ok(0);
                }
                out[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        let source = r#"{"content":"界\uD83D\uDE03\u0041\u0130\\\n"}"#;
        let p = project(Tiny(source.as_bytes()), true, Some("界😃ai\u{307}")).unwrap();
        assert!(p.matched);
        assert_eq!(p.value["content"], "界😃Aİ\\\n");
        assert!(!p.truncated);
    }
    #[test]
    fn search_arguments_keys_numbers_and_discarded_blocks() {
        let source = r#"{"message":{"content":[{"type":"toolCall","id":"x","name":"tool","arguments":{"NestedCode":{"items":[false,null,12345,"\u0053uffix"]}}}]},"unknown":"secret","image":"secret"}"#;
        for query in ["nestedcode", "12345", "suffix", "TOOL"] {
            assert!(parse(source, false, Some(query)).matched, "{query}");
        }
        assert!(!parse(source, false, Some("secret")).matched);
        assert!(!parse(r#"{"text":"ab","thinking":"cd"}"#, true, Some("bc")).matched);
        assert!(!parse(source, true, Some("")).matched);
        assert!(!parse(source, true, None).matched);
    }
    #[test]
    fn many_blocks_keep_all_tool_identities_and_search_tail() {
        let mut blocks: Vec<Value> = (0..1000)
            .map(|_| json!({"type":"text","text":"body"}))
            .collect();
        for i in 0..100 {
            // Type deliberately comes after identity and arguments on the wire.
            blocks.push(json!({"id":format!("call-{i}"),"name":"codemode","arguments":{"code":"needle"},"type":"toolCall"}));
        }
        let source = json!({"message":{"content":blocks}}).to_string();
        for preview in [false, true] {
            let p = parse(&source, preview, Some("needle"));
            let retained = p.value["message"]["content"].as_array().unwrap();
            assert_eq!(retained.len(), 100 + if preview { BODY_BLOCKS } else { 0 });
            assert_eq!(retained.last().unwrap()["id"], "call-99");
            assert!(p.matched && p.truncated);
        }
    }
    #[test]
    fn oversized_metadata_is_an_error_not_a_clipped_identity() {
        assert!(project(
            format!(r#"{{"id":"{}"}}"#, "x".repeat(4097)).as_bytes(),
            true,
            None
        )
        .err()
        .unwrap()
        .contains("metadata"));
        assert_eq!(
            parse(&format!(r#"{{"id":"{}"}}"#, "x".repeat(4096)), false, None).value["id"]
                .as_str()
                .unwrap()
                .len(),
            4096
        );
        let blocks: Vec<Value> = (0..1000)
            .map(|i| json!({"type":"toolCall","id":i.to_string(),"name":"tool"}))
            .collect();
        assert!(project(
            json!({"message":{"content":blocks}}).to_string().as_bytes(),
            false,
            None
        )
        .err()
        .unwrap()
        .contains("metadata"));
    }
    #[test]
    fn strict_json_even_inside_discarded_fields() {
        let invalid = [
            r#"{"unknown":"\x"}"#,
            r#"{"unknown":"\uD800"}"#,
            r#"{"unknown":"\uDC00"}"#,
            r#"{"unknown":"\uD800\u0041"}"#,
            r#"{"unknown":"\uZZZZ"}"#,
            "{\"unknown\":\"raw\nnewline\"}",
            r#"{"unknown":[1,]}"#,
            r#"{"unknown":01}"#,
            r#"{"unknown":1.}"#,
            r#"{"unknown":1e+}"#,
            r#"{"unknown":-}"#,
            r#"{"unknown":truex}"#,
            r#"{"unknown":NaN}"#,
            r#"{"id":"x",}"#,
            r#"{"id":"x"} {}"#,
            r#"{"unknown":{"a":1 "b":2}}"#,
            r#"{"id":"unfinished"#,
        ];
        for source in invalid {
            assert!(
                project(source.as_bytes(), false, Some("unfinished")).is_err(),
                "accepted {source:?}"
            );
        }
        for bytes in [
            vec![0xc0, 0xaf],
            vec![0xed, 0xa0, 0x80],
            vec![0xf4, 0x90, 0x80, 0x80],
            vec![0xe2, 0x82],
        ] {
            let mut source = b"{\"unknown\":\"".to_vec();
            source.extend(bytes);
            source.extend(b"\"}");
            assert!(project(source.as_slice(), false, None).is_err());
        }
        let p = parse(
            " {\"unknown\":[1e999999, -0.5e-10, true, false, null]}\r\n",
            true,
            None,
        );
        assert!(p.truncated);
    }
    #[test]
    fn streaming_source_never_requests_more_than_16k() {
        use std::cell::Cell;
        use std::rc::Rc;
        struct Observed<R> {
            inner: R,
            largest: Rc<Cell<usize>>,
        }
        impl<R: Read> Read for Observed<R> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.largest.set(self.largest.get().max(out.len()));
                self.inner.read(out)
            }
        }
        for preview in [false, true] {
            // Produces 4 MiB without allocating a giant source string, and
            // searches an escaped suffix beyond every retained preview byte.
            let source = std::io::Cursor::new(br#"{"content":""#)
                .chain(std::io::repeat(b'x').take(4 * 1024 * 1024))
                .chain(std::io::Cursor::new(br#"\u754cTAIL","id":"after-body"}"#));
            let largest = Rc::new(Cell::new(0));
            let p = project(
                Observed {
                    inner: source,
                    largest: largest.clone(),
                },
                preview,
                Some("界tail"),
            )
            .unwrap();
            assert!(p.matched && p.truncated);
            assert_eq!(p.value["id"], "after-body");
            assert_eq!(
                p.value["content"].as_str().unwrap().len(),
                if preview { BODY_BYTES } else { 0 }
            );
            assert!(largest.get() <= BUFFER_BYTES);
        }
    }
    #[test]
    fn long_unknown_and_argument_keys_are_decoded_not_retained() {
        let key = "x".repeat(100_000);
        let source =
            format!(r#"{{"{key}name":"not-searchable","arguments":{{"{key}KEYTAIL":0}}}}"#);
        let p = parse(&source, true, Some("keytail"));
        assert!(p.matched && p.truncated);
        assert!(p.value.as_object().unwrap().is_empty());
        assert!(!parse(&source, false, Some("not-searchable")).matched);
    }
    #[test]
    fn truncation_at_every_boundary_and_io_errors_reject() {
        let source = br#"{"unknown":[{"escaped":"\uD83D\uDE03"},1.25e-10,true,null],"id":"ok"}"#;
        for end in 0..source.len() {
            assert!(
                project(&source[..end], false, None).is_err(),
                "accepted prefix {end}"
            );
        }
        assert!(project(source.as_slice(), false, None).is_ok());
        struct Failed;
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("synthetic failure"))
            }
        }
        assert!(project(
            std::io::Cursor::new(br#"{"id":"ok"}"#).chain(Failed),
            true,
            None
        )
        .err()
        .unwrap()
        .contains("synthetic failure"));
        let p = parse(
            r#"{"name":"old","name":[],"id":"first","id":"last"}"#,
            true,
            None,
        );
        assert!(p.value.get("name").is_none());
        assert_eq!(p.value["id"], "last");
    }
    #[test]
    fn truncated_giant_and_depth_limit() {
        let source = format!(r#"{{"unknown":"{}"#, "x".repeat(100_000));
        assert!(project(source.as_bytes(), false, None).is_err());
        for (depth, valid) in [(127, true), (128, false)] {
            let source = format!(
                r#"{{"unknown":{}0{}}}"#,
                "[".repeat(depth),
                "]".repeat(depth)
            );
            assert_eq!(project(source.as_bytes(), false, None).is_ok(), valid);
        }
    }
}
