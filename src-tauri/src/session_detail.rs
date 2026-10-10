//! Read-only, generation-bound JSONL offset index and byte-bounded IPC pages.
use super::{ContentBlock, Entry, SessionDetail, Stats};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, Metadata};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[path = "detail_json.rs"]
mod json;
const INDEX_BYTES: usize = 16 * 1024 * 1024;
const PARSED_BYTES: usize = 4 * 1024 * 1024;
const MAX_BYTES: usize = 256 * 1024;
const SMALL_RECORD: u64 = 64 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyRef {
    pub generation: String,
    pub entry_id: String,
    pub byte_length: u64,
    pub record_offset: u64,
    pub preview: bool,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    pub messages: usize,
    pub user: usize,
    pub assistant: usize,
    pub tool_result: usize,
    pub labeled: usize,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPair {
    pub tool_call_id: String,
    pub call_entry_id: String,
    pub result_entry_id: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMeta {
    pub generation: String,
    pub previous_cursor: Option<String>,
    pub total_entries: usize,
    pub branch_entries: usize,
    pub matched_entries: usize,
    pub returned_entries: usize,
    pub has_more: bool,
    pub incomplete_tail: bool,
    pub malformed_lines: usize,
    pub counters: Counters,
    pub tool_pairs: Vec<ToolPair>,
    pub payload_bytes: usize,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetailPageRequest {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub max_bytes: Option<usize>,
    pub query: Option<String>,
    pub filter: Option<String>,
    pub entry_id: Option<String>,
    pub branch_leaf_id: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryBodyChunk {
    pub generation: String,
    pub entry_id: String,
    pub record_offset: u64,
    pub offset: u64,
    pub next_offset: Option<u64>,
    pub total_bytes: u64,
    pub data: String,
    pub encoding: &'static str,
    pub complete: bool,
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    generation: String,
    end: usize,
    query: String,
    filter: String,
    leaf: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Fingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_ns: i64,
}
impl Fingerprint {
    fn of(md: Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            len: md.len(),
            modified: md.modified().ok(),
            #[cfg(unix)]
            dev: md.dev(),
            #[cfg(unix)]
            ino: md.ino(),
            #[cfg(unix)]
            ctime: md.ctime(),
            #[cfg(unix)]
            ctime_ns: md.ctime_nsec(),
        }
    }
    fn path(path: &Path) -> Result<Self, String> {
        fs::metadata(path).map(Self::of).map_err(|e| e.to_string())
    }
}
fn stale() -> String {
    "STALE_DETAIL: session changed; reload latest page".into()
}
fn validate(path: &Path, file: &File, fp: &Fingerprint) -> Result<(), String> {
    if Fingerprint::path(path)? != *fp
        || Fingerprint::of(file.metadata().map_err(|e| e.to_string())?) != *fp
    {
        return Err(stale());
    }
    Ok(())
}
struct Record {
    offset: u64,
    len: u64,
    meta: Entry,
}
struct Index {
    path: PathBuf,
    fp: Fingerprint,
    generation: String,
    id: String,
    cwd: String,
    created: String,
    task_id: Option<String>,
    stats: Stats,
    records: Vec<Record>,
    by_id: HashMap<String, usize>,
    labels: HashSet<String>,
    incomplete: bool,
    malformed: usize,
    bytes: usize,
}
struct ByteCache<K, V> {
    items: VecDeque<(K, Arc<V>, usize)>,
    bytes: usize,
    budget: usize,
}
impl<K: PartialEq, V> ByteCache<K, V> {
    fn new(budget: usize) -> Self {
        Self {
            items: VecDeque::new(),
            bytes: 0,
            budget,
        }
    }
    fn get(&mut self, key: &K) -> Option<Arc<V>> {
        let at = self.items.iter().position(|(k, _, _)| k == key)?;
        let item = self.items.remove(at)?;
        let result = Arc::clone(&item.1);
        self.items.push_back(item);
        Some(result)
    }
    fn put(&mut self, key: K, value: Arc<V>, bytes: usize) {
        if let Some(at) = self.items.iter().position(|(k, _, _)| *k == key) {
            if let Some((_, _, n)) = self.items.remove(at) {
                self.bytes -= n;
            }
        }
        if bytes > self.budget {
            return;
        }
        while self.bytes + bytes > self.budget {
            if let Some((_, _, n)) = self.items.pop_front() {
                self.bytes -= n;
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.items.push_back((key, value, bytes));
    }
}
type IndexCache = ByteCache<PathBuf, Index>;
type ParsedCache = ByteCache<(String, usize), Entry>;
static INDEX_BUILD: Mutex<()> = Mutex::new(());
static INDEX_CACHE: OnceLock<Mutex<IndexCache>> = OnceLock::new();
static PARSED_CACHE: OnceLock<Mutex<ParsedCache>> = OnceLock::new();
fn indices() -> &'static Mutex<IndexCache> {
    INDEX_CACHE.get_or_init(|| Mutex::new(ByteCache::new(INDEX_BYTES)))
}
fn parsed() -> &'static Mutex<ParsedCache> {
    PARSED_CACHE.get_or_init(|| Mutex::new(ByteCache::new(PARSED_BYTES)))
}
fn current_generation(path: &str) -> Result<String, String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    Ok(generation(&path, &Fingerprint::path(&path)?))
}
fn generation(path: &Path, fp: &Fingerprint) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    fp.hash(&mut h);
    format!("{:016x}", h.finish())
}
fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").into()
}
fn u64_field(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}
fn record_projection(
    file: &mut File,
    offset: u64,
    len: u64,
    preview: bool,
    query: Option<&str>,
) -> Result<json::Projection, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    json::project(file.take(len), preview, query)
}
fn index(path: &str) -> Result<Arc<Index>, String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let fp = Fingerprint::path(&path)?;
    if let Some(cached) = indices()
        .lock()
        .map_err(|_| "Index cache poisoned")?
        .get(&path)
    {
        if cached.fp == fp {
            let file = File::open(&path).map_err(|e| e.to_string())?;
            validate(&path, &file, &fp)?;
            return Ok(cached);
        }
    }
    // Single cold build at a time: concurrent polls must not retain duplicate
    // indexes for the same multi-MB file. Cache-hit pages never take this lock.
    let _building = INDEX_BUILD
        .lock()
        .map_err(|_| "Index build lock poisoned")?;
    if let Some(cached) = indices()
        .lock()
        .map_err(|_| "Index cache poisoned")?
        .get(&path)
    {
        if cached.fp == fp {
            let file = File::open(&path).map_err(|e| e.to_string())?;
            validate(&path, &file, &fp)?;
            return Ok(cached);
        }
    }
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    validate(&path, &file, &fp)?;
    let mut reader =
        BufReader::with_capacity(16 * 1024, File::open(&path).map_err(|e| e.to_string())?);
    let mut ix = Index {
        path: path.clone(),
        fp: fp.clone(),
        generation: generation(&path, &fp),
        id: String::new(),
        cwd: String::new(),
        created: String::new(),
        task_id: None,
        stats: Stats {
            token_count: 0,
            message_count: 0,
            model: None,
            provider: None,
            thinking_level: None,
            context_tokens: None,
            context_limit: None,
            cost_total: 0.0,
        },
        records: Vec::new(),
        by_id: HashMap::new(),
        labels: HashSet::new(),
        incomplete: false,
        malformed: 0,
        bytes: 0,
    };
    let mut offset = 0u64;
    let mut start = 0u64;
    let mut non_whitespace = false;
    // Locate record boundaries without ever growing a line buffer. The second
    // descriptor projects each record directly from a bounded buffered reader.
    loop {
        let buf = reader.fill_buf().map_err(|e| e.to_string())?;
        if buf.is_empty() {
            break;
        }
        let used = buf
            .iter()
            .position(|b| *b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(buf.len());
        let ended = buf[used - 1] == b'\n';
        non_whitespace |= buf[..used]
            .iter()
            .any(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
        offset += used as u64;
        reader.consume(used);
        if !ended {
            continue;
        }
        if !non_whitespace {
            start = offset;
            continue;
        }
        non_whitespace = false;
        let len = offset - start - 1;
        let projection = record_projection(&mut file, start, len, false, None);
        match projection {
            Ok(p) => {
                let v = p.value;
                let kind = str_field(&v, "type");
                if kind == "session" {
                    ix.id = str_field(&v, "id");
                    ix.cwd = str_field(&v, "cwd");
                    ix.created = str_field(&v, "timestamp");
                } else if let Some(mut meta) = super::parse_entry(&kind, &v) {
                    if meta.model.is_some() {
                        ix.stats.model = meta.model.clone();
                    }
                    if kind == "model_change" {
                        ix.stats.provider =
                            v.get("provider").and_then(Value::as_str).map(str::to_owned);
                    }
                    if kind == "thinking_level_change" {
                        ix.stats.thinking_level = v
                            .get("thinkingLevel")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                    }
                    if kind == "message" {
                        ix.stats.message_count += 1;
                        if let Some(m) = v.get("message") {
                            if let Some(p) = m.get("provider").and_then(Value::as_str) {
                                ix.stats.provider = Some(p.into());
                            }
                            if let Some(u) = m.get("usage") {
                                ix.stats.token_count = ix
                                    .stats
                                    .token_count
                                    .saturating_add(u64_field(u, "totalTokens"));
                                ix.stats.cost_total += u
                                    .get("cost")
                                    .and_then(|c| c.get("total"))
                                    .and_then(Value::as_f64)
                                    .unwrap_or(0.0);
                                let context =
                                    u64_field(u, "input").saturating_add(u64_field(u, "cacheRead"));
                                if meta.role.as_deref() == Some("assistant") && context > 0 {
                                    ix.stats.context_tokens = Some(context);
                                }
                            }
                        }
                    }
                    if kind == "message" || kind == "compaction" {
                        ix.stats.token_count = ix.stats.token_count.saturating_add(
                            v.get("usage")
                                .map(|u| u64_field(u, "totalTokens"))
                                .unwrap_or(0),
                        );
                    }
                    // Only call identities are kept, never body text or arguments.
                    meta.content
                        .retain(|c| matches!(c, ContentBlock::ToolCall { .. }));
                    for c in &mut meta.content {
                        if let ContentBlock::ToolCall { arguments, .. } = c {
                            arguments.clear();
                        }
                    }
                    meta.summary = None;
                    if kind == "label" {
                        if let Some(target) = &meta.name {
                            ix.labels.insert(target.clone());
                        }
                    }
                    let n = ix.records.len();
                    ix.by_id.insert(meta.id.clone(), n);
                    ix.records.push(Record {
                        offset: start,
                        len,
                        meta,
                    });
                }
            }
            Err(e) if e.starts_with("METADATA_LIMIT") => return Err(e),
            Err(_) => {
                if len > 0 {
                    ix.malformed += 1;
                }
            }
        }
        start = offset;
    }
    ix.incomplete = start < offset;
    validate(&path, &file, &fp)?;
    if ix.id.is_empty() {
        return Err("No valid session header".into());
    }
    #[cfg(not(test))]
    {
        ix.task_id = super::subagent_index()
            .1
            .get(&ix.id)
            .and_then(|t| t.first().cloned());
        ix.stats.context_limit = ix
            .stats
            .model
            .as_deref()
            .and_then(super::model_context_window);
    }
    #[cfg(test)]
    {
        ix.task_id = None;
    }
    // Account capacities and duplicated map keys, not just record count.
    ix.bytes = 16 * 1024
        + std::mem::size_of::<Index>()
        + ix.path.as_os_str().len()
        + ix.id.capacity()
        + ix.cwd.capacity()
        + ix.created.capacity()
        + ix.records.capacity() * std::mem::size_of::<Record>()
        + ix.by_id.capacity() * (std::mem::size_of::<(String, usize)>() + 16)
        + ix.labels.capacity() * (std::mem::size_of::<String>() + 16);
    for r in &ix.records {
        ix.bytes += entry_heap(&r.meta);
    }
    ix.bytes += ix.by_id.keys().map(String::capacity).sum::<usize>()
        + ix.labels.iter().map(String::capacity).sum::<usize>();
    validate(&path, &file, &fp)?;
    let ix = Arc::new(ix);
    indices()
        .lock()
        .map_err(|_| "Index cache poisoned")?
        .put(path, Arc::clone(&ix), ix.bytes);
    Ok(ix)
}
fn entry_heap(e: &Entry) -> usize {
    e.kind.capacity()
        + e.id.capacity()
        + e.parent_id
            .iter()
            .chain(e.ts.iter())
            .chain(e.role.iter())
            .chain(e.model.iter())
            .chain(e.tool_name.iter())
            .chain(e.tool_call_id.iter())
            .chain(e.summary.iter())
            .chain(e.name.iter())
            .chain(e.label.iter())
            .map(String::capacity)
            .sum::<usize>()
        + e.content.capacity() * std::mem::size_of::<ContentBlock>()
        + e.content
            .iter()
            .map(|c| match c {
                ContentBlock::Text { text } => text.capacity(),
                ContentBlock::Thinking { thinking } => thinking.capacity(),
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                } => id.capacity() + name.capacity() + arguments.capacity(),
                ContentBlock::Bash {
                    command, output, ..
                } => command.capacity() + output.capacity(),
                ContentBlock::Image { mime_type } => mime_type.capacity(),
            })
            .sum::<usize>()
}
fn branch(ix: &Index, leaf: Option<&str>) -> Result<Vec<usize>, String> {
    let mut current = match leaf {
        Some(id) => Some(*ix.by_id.get(id).ok_or("Branch entry not found")?),
        None => ix.records.len().checked_sub(1),
    };
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(n) = current {
        if !seen.insert(n) {
            break;
        }
        result.push(n);
        current = ix.records[n]
            .meta
            .parent_id
            .as_ref()
            .and_then(|id| ix.by_id.get(id))
            .copied();
    }
    result.reverse();
    Ok(result)
}
fn entry(ix: &Index, file: &mut File, n: usize) -> Result<Arc<Entry>, String> {
    let key = (ix.generation.clone(), n);
    if let Some(e) = parsed()
        .lock()
        .map_err(|_| "Parsed cache poisoned")?
        .get(&key)
    {
        return Ok(e);
    }
    let r = &ix.records[n];
    let mut e = if r.len <= SMALL_RECORD {
        file.seek(SeekFrom::Start(r.offset))
            .map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_reader(file.take(r.len)).map_err(|e| e.to_string())?;
        super::parse_entry(&r.meta.kind, &v).ok_or("Invalid indexed entry")?
    } else {
        let p = record_projection(file, r.offset, r.len, true, None)?;
        let mut e = super::parse_entry(&r.meta.kind, &p.value).ok_or("Invalid indexed entry")?;
        e.body_ref = Some(BodyRef {
            generation: ix.generation.clone(),
            entry_id: e.id.clone(),
            byte_length: r.len,
            record_offset: r.offset,
            preview: p.truncated || r.len > SMALL_RECORD,
        });
        e
    };
    e.labeled = Some(ix.labels.contains(&e.id));
    let bytes = std::mem::size_of::<Entry>()
        + entry_heap(&e)
        + key.0.len()
        + 64
        + e.body_ref
            .as_ref()
            .map(|r| {
                r.generation.capacity() + r.entry_id.capacity() + std::mem::size_of::<BodyRef>()
            })
            .unwrap_or(0);
    let e = Arc::new(e);
    parsed()
        .lock()
        .map_err(|_| "Parsed cache poisoned")?
        .put(key, Arc::clone(&e), bytes);
    Ok(e)
}
fn accepted(e: &Entry, labeled: bool, filter: &str) -> bool {
    let settings = matches!(
        e.kind.as_str(),
        "label" | "custom" | "model_change" | "thinking_level_change" | "session_info"
    );
    match filter {
        "all" => true,
        "user-only" => e.role.as_deref() == Some("user"),
        "no-tools" => !settings && e.role.as_deref() != Some("toolResult"),
        "labeled-only" => e.kind == "label" || labeled,
        _ => !settings,
    }
}
fn serialized_size(value: &impl Serialize) -> Result<usize, String> {
    // Counting writer prevents another allocation of a full serialized payload.
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 += b.len();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut c = Counter(0);
    serde_json::to_writer(&mut c, value).map_err(|e| e.to_string())?;
    Ok(c.0)
}
fn finalize(detail: &mut SessionDetail) -> Result<usize, String> {
    for _ in 0..4 {
        let size = serialized_size(detail)?;
        let page = detail.page.as_mut().unwrap();
        if page.payload_bytes == size {
            return Ok(size);
        }
        page.payload_bytes = size;
    }
    serialized_size(detail)
}
pub fn page(path: &str, request: DetailPageRequest) -> Result<SessionDetail, String> {
    if let Some(cursor) = &request.cursor {
        let c: Cursor = serde_json::from_str(cursor).map_err(|_| "Invalid detail cursor")?;
        if c.generation != current_generation(path)? {
            return Err(stale());
        }
    }
    let ix = index(path)?;
    let mut file = File::open(&ix.path).map_err(|e| e.to_string())?;
    validate(&ix.path, &file, &ix.fp)?;
    let limit = request.limit.unwrap_or(100).clamp(1, 200);
    let budget = request
        .max_bytes
        .unwrap_or(MAX_BYTES)
        .clamp(16384, MAX_BYTES);
    let query = request.query.unwrap_or_default().trim().to_lowercase();
    if query.len() > 4096 {
        return Err("Search query exceeds 4096 bytes".into());
    }
    let filter = request.filter.unwrap_or_else(|| "default".into());
    if !matches!(
        filter.as_str(),
        "default" | "all" | "user-only" | "no-tools" | "labeled-only"
    ) {
        return Err("Unknown detail filter".into());
    }
    let chain = branch(&ix, request.branch_leaf_id.as_deref())?;
    let mut counters = Counters::default();
    let mut matches = Vec::new();
    let mut calls: HashMap<String, usize> = HashMap::new();
    let mut pairs: Vec<(usize, usize, String)> = Vec::new();
    for &n in &chain {
        let r = &ix.records[n];
        let e = &r.meta;
        let labeled = ix.labels.contains(&e.id);
        if e.kind == "message" {
            counters.messages += 1;
        }
        match e.role.as_deref() {
            Some("user") => counters.user += 1,
            Some("assistant") => counters.assistant += 1,
            Some("toolResult") => counters.tool_result += 1,
            _ => (),
        }
        if labeled {
            counters.labeled += 1;
        }
        for c in &e.content {
            if let ContentBlock::ToolCall { id, .. } = c {
                calls.insert(id.clone(), n);
            }
        }
        if let Some(id) = &e.tool_call_id {
            if let Some(&call) = calls.get(id) {
                pairs.push((call, n, id.clone()));
            }
        }
        if accepted(e, labeled, &filter)
            && (query.is_empty()
                || record_projection(&mut file, r.offset, r.len, false, Some(&query))?.matched)
        {
            matches.push(n);
        }
    }
    let mut end = matches.len();
    if let Some(cursor) = request.cursor {
        let c: Cursor = serde_json::from_str(&cursor).map_err(|_| "Invalid detail cursor")?;
        if c.generation != ix.generation {
            return Err(stale());
        }
        if c.query != query
            || c.filter != filter
            || c.leaf != request.branch_leaf_id
            || c.end > matches.len()
        {
            return Err("Cursor does not match query/filter/branch".into());
        }
        end = c.end;
    }
    if let Some(id) = request.entry_id {
        let selected = *ix.by_id.get(&id).ok_or("Entry not found")?;
        let pos = matches
            .iter()
            .position(|n| *n == selected)
            .ok_or("Entry does not match selected branch/filter/query")?;
        end = (pos + 1).min(matches.len());
    }
    let mut detail = SessionDetail {
        page: Some(PageMeta {
            generation: ix.generation.clone(),
            previous_cursor: None,
            total_entries: ix.records.len(),
            branch_entries: chain.len(),
            matched_entries: matches.len(),
            returned_entries: 0,
            has_more: false,
            incomplete_tail: ix.incomplete,
            malformed_lines: ix.malformed,
            counters,
            tool_pairs: Vec::new(),
            payload_bytes: 0,
        }),
        id: ix.id.clone(),
        cwd: ix.cwd.clone(),
        created_iso: ix.created.clone(),
        path: path.into(),
        task_id: ix.task_id.clone(),
        stats: ix.stats.clone(),
        entries: Vec::new(),
        active: Vec::new(),
        size: ix.fp.len,
        updated_at: ix
            .fp
            .modified
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    };
    let mut start = end;
    let mut selected = HashSet::new();
    let mut entries_bytes = 0usize;
    while start > 0 && end - start < limit {
        let n = matches[start - 1];
        let e = entry(&ix, &mut file, n)?;
        let bytes = serialized_size(e.as_ref())?;
        // Fast incremental preflight; final whole-response accounting below
        // includes cursors, counters, pair identities, escaping and byte count.
        if !detail.entries.is_empty() && entries_bytes + bytes + 1024 > budget {
            break;
        }
        detail.entries.insert(0, (*e).clone());
        selected.insert(n);
        start -= 1;
        entries_bytes += bytes;
    }
    loop {
        update_page(
            &mut detail,
            &ix,
            &pairs,
            &selected,
            start,
            &query,
            &filter,
            &request.branch_leaf_id,
        )?;
        if finalize(&mut detail)? <= budget {
            break;
        }
        if detail.entries.len() > 1 {
            selected.remove(&matches[start]);
            detail.entries.remove(0);
            start += 1;
            continue;
        }
        if let Some(e) = detail.entries.first_mut() {
            if e.content.is_empty() && e.summary.is_none() && e.body_ref.is_some() {
                return Err("METADATA_LIMIT: entry identities exceed page byte budget".into());
            }
            let n = matches[start];
            e.content.clear();
            e.summary = None;
            e.body_ref = Some(BodyRef {
                generation: ix.generation.clone(),
                entry_id: e.id.clone(),
                byte_length: ix.records[n].len,
                record_offset: ix.records[n].offset,
                preview: true,
            });
        } else {
            return Err("METADATA_LIMIT: page metadata exceeds byte budget".into());
        }
    }
    update_page(
        &mut detail,
        &ix,
        &pairs,
        &selected,
        start,
        &query,
        &filter,
        &request.branch_leaf_id,
    )?;
    if finalize(&mut detail)? > budget {
        return Err("METADATA_LIMIT: page metadata exceeds byte budget".into());
    }
    validate(&ix.path, &file, &ix.fp)?;
    if fs::canonicalize(path).map_err(|e| e.to_string())? != ix.path {
        return Err(stale());
    }
    Ok(detail)
}
fn update_page(
    detail: &mut SessionDetail,
    ix: &Index,
    pairs: &[(usize, usize, String)],
    selected: &HashSet<usize>,
    start: usize,
    query: &str,
    filter: &str,
    leaf: &Option<String>,
) -> Result<(), String> {
    detail.active = (0..detail.entries.len()).collect();
    let page = detail.page.as_mut().unwrap();
    page.returned_entries = detail.entries.len();
    page.has_more = start > 0;
    page.previous_cursor = if start > 0 {
        Some(
            serde_json::to_string(&Cursor {
                generation: ix.generation.clone(),
                end: start,
                query: query.into(),
                filter: filter.into(),
                leaf: leaf.clone(),
            })
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    page.tool_pairs = pairs
        .iter()
        .filter(|(call, result, _)| selected.contains(call) || selected.contains(result))
        .map(|(call, result, id)| ToolPair {
            tool_call_id: id.clone(),
            call_entry_id: ix.records[*call].meta.id.clone(),
            result_entry_id: ix.records[*result].meta.id.clone(),
        })
        .collect();
    Ok(())
}
pub fn body(
    path: &str,
    generation: &str,
    entry_id: &str,
    record_offset: Option<u64>,
    offset: u64,
    max_bytes: usize,
) -> Result<EntryBodyChunk, String> {
    if current_generation(path)? != generation {
        return Err(stale());
    }
    let ix = index(path)?;
    if ix.generation != generation {
        return Err(stale());
    }
    let n = if let Some(record_offset) = record_offset {
        ix.records
            .binary_search_by_key(&record_offset, |r| r.offset)
            .map_err(|_| "Body record offset not found")?
    } else {
        *ix.by_id.get(entry_id).ok_or("Entry not found")?
    };
    let r = &ix.records[n];
    if r.meta.id != entry_id {
        return Err("Body record offset does not match entry ID".into());
    }
    if offset > r.len {
        return Err("Body offset beyond end".into());
    }
    let mut file = File::open(&ix.path).map_err(|e| e.to_string())?;
    validate(&ix.path, &file, &ix.fp)?;
    file.seek(SeekFrom::Start(r.offset + offset))
        .map_err(|e| e.to_string())?;
    let count = (r.len - offset).min(max_bytes.clamp(4, 131072) as u64) as usize;
    let mut bytes = vec![0; count];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    // Offsets are bytes. Never replace invalid UTF-8 or split a code point.
    let valid = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => return Err("Invalid UTF-8 body offset/record".into()),
    };
    if valid == 0 && count > 0 {
        return Err("Body offset is not a UTF-8 boundary".into());
    }
    bytes.truncate(valid);
    let next = offset + valid as u64;
    let data = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    validate(&ix.path, &file, &ix.fp)?;
    let mut chunk = EntryBodyChunk {
        generation: ix.generation.clone(),
        entry_id: entry_id.into(),
        record_offset: r.offset,
        offset,
        next_offset: (next < r.len).then_some(next),
        total_bytes: r.len,
        data,
        encoding: "utf8-jsonl",
        complete: next == r.len,
    };
    // JSON escaping can inflate raw data; cap the whole IPC chunk, not just
    // its source bytes. The unused bytes remain available at nextOffset.
    while serialized_size(&chunk)? > MAX_BYTES {
        let mut keep = chunk.data.len() / 2;
        while !chunk.data.is_char_boundary(keep) {
            keep -= 1;
        }
        if keep == 0 {
            return Err("METADATA_LIMIT: body chunk identity exceeds transport budget".into());
        }
        chunk.data.truncate(keep);
        chunk.next_offset = Some(offset + keep as u64);
        chunk.complete = false;
    }
    validate(&ix.path, &file, &ix.fp)?;
    if fs::canonicalize(path).map_err(|e| e.to_string())? != ix.path {
        return Err(stale());
    }
    Ok(chunk)
}

#[cfg(test)]
#[path = "session_detail_tests.rs"]
mod tests;
