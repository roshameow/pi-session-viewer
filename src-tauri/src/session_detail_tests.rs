use super::*;
use serde_json::json;
use std::io::Write;
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "pi-bounded-synthetic-{}-{}.jsonl",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let f = Self(p);
        f.write(&[json!({"type":"session","id":"synthetic","cwd":"/synthetic","timestamp":"2026-10-07T00:00:00Z"})]);
        f
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
    fn write(&self, values: &[Value]) {
        let mut f = File::create(&self.0).unwrap();
        for v in values {
            writeln!(f, "{v}").unwrap();
        }
    }
    fn append(&self, v: Value) {
        writeln!(
            fs::OpenOptions::new().append(true).open(&self.0).unwrap(),
            "{v}"
        )
        .unwrap();
    }
    fn page(&self, r: DetailPageRequest) -> SessionDetail {
        page(self.path(), r).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn msg(id: &str, parent: Option<&str>, role: &str, text: &str) -> Value {
    json!({"type":"message","id":id,"parentId":parent,"message":{"role":role,"content":[{"type":"text","text":text}]}})
}
fn req(filter: &str) -> DetailPageRequest {
    DetailPageRequest {
        filter: Some(filter.into()),
        ..Default::default()
    }
}
#[test]
fn recent_pages_full_search_selection_filters_labels_and_stats() {
    let f = Fixture::new();
    for n in 0..350 {
        let parent = format!("e{}", n - 1);
        f.append(msg(
            &format!("e{n}"),
            if n == 0 { None } else { Some(&parent) },
            if n % 2 == 0 { "user" } else { "assistant" },
            if n == 3 { "older-needle" } else { "synthetic" },
        ));
    }
    f.append(
        json!({"type":"label","id":"label","parentId":"e349","targetId":"e3","label":"important"}),
    );
    let d = f.page(req("all"));
    let p = d.page.unwrap();
    assert_eq!(p.branch_entries, 351);
    assert_eq!(p.counters.messages, 350);
    assert_eq!(d.stats.message_count, 350);
    assert_eq!(d.entries.len(), 100);
    assert_eq!(d.entries[0].id, "e251");
    assert!(p.payload_bytes <= MAX_BYTES);
    let d = f.page(DetailPageRequest {
        cursor: p.previous_cursor,
        filter: Some("all".into()),
        ..Default::default()
    });
    assert_eq!(d.entries[0].id, "e151");
    let d = f.page(DetailPageRequest {
        query: Some("OLDER-NEEDLE".into()),
        ..Default::default()
    });
    assert_eq!(d.entries[0].id, "e3");
    assert_eq!(d.page.unwrap().matched_entries, 1);
    let d = f.page(DetailPageRequest {
        entry_id: Some("e3".into()),
        filter: Some("all".into()),
        ..Default::default()
    });
    assert_eq!(d.entries.last().unwrap().id, "e3");
    let d = f.page(req("labeled-only"));
    assert_eq!(
        d.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["e3", "label"]
    );
    assert_eq!(d.entries[0].labeled, Some(true));
    let d = f.page(req("user-only"));
    assert_eq!(d.page.unwrap().matched_entries, 175);
}
#[test]
fn branch_compaction_duplicate_ids_and_cycles() {
    let f = Fixture::new();
    f.append(msg("root", None, "user", "root"));
    f.append(msg("old", Some("root"), "assistant", "offbranch"));
    f.append(json!({"type":"compaction","id":"compact","parentId":"root","summary":"preserved summary","usage":{"totalTokens":15}}));
    f.append(msg("new", Some("compact"), "assistant", "new branch"));
    let d = f.page(req("all"));
    assert_eq!(
        d.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["root", "compact", "new"]
    );
    assert_eq!(d.stats.token_count, 15);
    assert_eq!(d.stats.message_count, 3);
    assert_eq!(d.entries[1].summary.as_deref(), Some("preserved summary"));
    let d = f.page(DetailPageRequest {
        branch_leaf_id: Some("old".into()),
        ..req("all")
    });
    assert_eq!(d.entries.last().unwrap().id, "old");
    f.append(msg("new", Some("root"), "assistant", "last duplicate wins"));
    let d = f.page(req("all"));
    assert_eq!(d.entries.len(), 2);
    assert_eq!(d.entries[1].id, "new");
    f.append(msg("cycle", Some("cycle"), "user", "safe"));
    let d = f.page(req("all"));
    assert_eq!(d.entries.len(), 1);
}
#[test]
fn tool_pairing_crosses_page_boundaries_and_filter() {
    let f = Fixture::new();
    f.append(json!({"type":"message","id":"call","parentId":null,"message":{"role":"assistant","content":[{"type":"toolCall","id":"tool1","name":"codemode","arguments":{"code":"synthetic code"}}]}}));
    for n in 0..110 {
        let parent = if n == 0 {
            "call".to_string()
        } else {
            format!("between{}", n - 1)
        };
        f.append(msg(&format!("between{n}"), Some(&parent), "user", "gap"));
    }
    f.append(json!({"type":"message","id":"result","parentId":"between109","message":{"role":"toolResult","toolCallId":"tool1","toolName":"codemode","content":[{"type":"text","text":"answer"}]}}));
    let d = f.page(DetailPageRequest {
        limit: Some(1),
        ..req("all")
    });
    let p = d.page.unwrap();
    assert_eq!(d.entries[0].id, "result");
    assert_eq!(p.tool_pairs.len(), 1);
    assert_eq!(p.tool_pairs[0].call_entry_id, "call");
    assert_eq!(p.tool_pairs[0].result_entry_id, "result");
    let d = f.page(DetailPageRequest {
        entry_id: Some("call".into()),
        ..req("all")
    });
    assert_eq!(d.page.unwrap().tool_pairs[0].result_entry_id, "result");
    let d = f.page(req("no-tools"));
    assert!(d
        .entries
        .iter()
        .all(|e| e.role.as_deref() != Some("toolResult")));
}
#[test]
fn giant_body_lossless_chunks_utf8_and_full_suffix_search() {
    let f = Fixture::new();
    let text = format!("{} rare-suffix-🦀", "汉字🦀 code; ".repeat(80000));
    let v = msg("giant", None, "user", &text);
    let raw = v.to_string();
    f.append(v);
    let d = f.page(DetailPageRequest {
        max_bytes: Some(16384),
        query: Some("rare-suffix-🦀".into()),
        ..req("all")
    });
    assert_eq!(d.page.as_ref().unwrap().matched_entries, 1);
    assert!(serde_json::to_vec(&d).unwrap().len() <= 16384);
    let r = d.entries[0].body_ref.as_ref().expect("explicit preview");
    assert!(r.preview);
    let mut offset = 0;
    let mut collected = String::new();
    loop {
        let chunk = body(
            f.path(),
            &r.generation,
            "giant",
            Some(r.record_offset),
            offset,
            4097,
        )
        .unwrap();
        collected.push_str(&chunk.data);
        if let Some(n) = chunk.next_offset {
            assert!(n > offset);
            offset = n;
        } else {
            assert!(chunk.complete);
            break;
        }
    }
    assert_eq!(collected, raw);
    let roundtrip: Value = serde_json::from_str(&collected).unwrap();
    assert_eq!(roundtrip["message"]["content"][0]["text"], text);
}
#[test]
fn growth_rewrite_stale_cursors_body_and_partial_tail() {
    let f = Fixture::new();
    f.append(msg("a", None, "user", "alpha"));
    f.append(msg("b", Some("a"), "assistant", "beta"));
    let d = f.page(DetailPageRequest {
        limit: Some(1),
        ..req("all")
    });
    let p = d.page.unwrap();
    let cursor = p.previous_cursor.unwrap();
    let mut out = fs::OpenOptions::new().append(true).open(&f.0).unwrap();
    write!(
        out,
        "{{\"type\":\"message\",\"id\":\"partial\",\"message\":"
    )
    .unwrap();
    drop(out);
    assert!(page(
        f.path(),
        DetailPageRequest {
            cursor: Some(cursor),
            ..req("all")
        }
    )
    .err()
    .unwrap()
    .starts_with("STALE_DETAIL"));
    assert!(body(f.path(), &p.generation, "b", None, 0, 1024)
        .err()
        .unwrap()
        .starts_with("STALE_DETAIL"));
    let d = f.page(req("all"));
    assert!(d.page.unwrap().incomplete_tail);
    assert_eq!(d.stats.message_count, 2);
    f.write(&[
        json!({"type":"session","id":"synthetic","cwd":"/synthetic"}),
        msg("a", None, "user", "changed"),
    ]);
    let d = f.page(req("all"));
    assert_eq!(d.stats.message_count, 1);
    let ix = index(f.path()).unwrap();
    let file = File::open(&f.0).unwrap();
    f.append(msg("c", Some("a"), "user", "race"));
    assert!(validate(&ix.path, &file, &ix.fp)
        .unwrap_err()
        .starts_with("STALE_DETAIL"));
}
#[test]
fn malformed_utf8_and_unterminated_complete_json_are_explicit() {
    let f = Fixture::new();
    f.append(msg("good", None, "user", "valid"));
    let mut out = fs::OpenOptions::new().append(true).open(&f.0).unwrap();
    out.write_all(b"{bad json}\n{\"type\":\"message\",\"id\":\"bad\",\"message\":{\"role\":\"user\",\"content\":\"\xff\"}}\n").unwrap();
    write!(
        out,
        "{}",
        msg("no-newline", Some("good"), "user", "not committed line")
    )
    .unwrap();
    drop(out);
    let d = f.page(req("all"));
    let p = d.page.unwrap();
    assert!(p.incomplete_tail);
    assert_eq!(p.malformed_lines, 2);
    assert_eq!(d.entries.len(), 1);
}
#[test]
fn source_paths_cache_arc_and_ten_thousand_entry_plateau() {
    let a = Fixture::new();
    let b = Fixture::new();
    a.append(msg("same", None, "user", "source A"));
    b.append(msg("same", None, "user", "source B"));
    let ai = index(a.path()).unwrap();
    let again = index(a.path()).unwrap();
    assert!(Arc::ptr_eq(&ai, &again));
    let bi = index(b.path()).unwrap();
    assert_ne!(ai.generation, bi.generation);
    assert_eq!(a.page(req("all")).entries[0].id, "same");
    assert_eq!(b.page(req("all")).entries[0].id, "same");
    let f = Fixture::new();
    let mut out = fs::OpenOptions::new().append(true).open(&f.0).unwrap();
    for n in 0..10000 {
        let parent = format!("e{}", n - 1);
        writeln!(
            out,
            "{}",
            msg(
                &format!("e{n}"),
                if n == 0 { None } else { Some(&parent) },
                "user",
                &"x".repeat(512)
            )
        )
        .unwrap();
    }
    drop(out);
    let mut request = req("all");
    let mut total = 0;
    for _ in 0..100 {
        let d = f.page(request);
        let p = d.page.unwrap();
        assert!(p.payload_bytes <= MAX_BYTES);
        assert_eq!(p.total_entries, 10000);
        total += d.entries.len();
        assert!(indices().lock().unwrap().bytes <= INDEX_BYTES);
        assert!(parsed().lock().unwrap().bytes <= PARSED_BYTES);
        request = DetailPageRequest {
            cursor: p.previous_cursor,
            ..req("all")
        };
    }
    assert_eq!(total, 10000);
    let ix = index(f.path()).unwrap();
    assert!(ix.bytes < 8 * 1024 * 1024);
    assert!(ix.records.iter().all(|r| r.meta.content.is_empty()));
}
#[test]
fn byte_bound_is_entire_serialized_payload_not_count_only() {
    let f = Fixture::new();
    for n in 0..200 {
        let parent = format!("e{}", n - 1);
        f.append(msg(
            &format!("e{n}"),
            if n == 0 { None } else { Some(&parent) },
            "user",
            &"\"\\\n🦀".repeat(1200),
        ));
    }
    let d = f.page(DetailPageRequest {
        limit: Some(200),
        max_bytes: Some(16384),
        ..req("all")
    });
    let bytes = serde_json::to_vec(&d).unwrap().len();
    assert!(bytes <= 16384);
    assert_eq!(d.page.as_ref().unwrap().payload_bytes, bytes);
    assert!(d.entries.len() < 200);
}

#[test]
fn same_size_rewrite_and_atomic_replacement_invalidate_generation() {
    let f = Fixture::new();
    f.append(msg("a", None, "user", "alpha"));
    let old = index(f.path()).unwrap();
    let old_file = File::open(&f.0).unwrap();
    let bytes = fs::read(&f.0).unwrap();
    let text = String::from_utf8(bytes).unwrap().replace("alpha", "omega");
    assert_eq!(text.len() as u64, old.fp.len);
    fs::write(&f.0, text.as_bytes()).unwrap();
    assert!(validate(&old.path, &old_file, &old.fp)
        .err()
        .unwrap()
        .starts_with("STALE_DETAIL"));
    let newer = index(f.path()).unwrap();
    assert_ne!(old.generation, newer.generation);
    let replacement = f.0.with_extension("replacement");
    fs::write(&replacement, &text).unwrap();
    fs::rename(&replacement, &f.0).unwrap();
    assert!(validate(&newer.path, &old_file, &newer.fp)
        .err()
        .unwrap()
        .starts_with("STALE_DETAIL"));
    assert_ne!(newer.generation, index(f.path()).unwrap().generation);
}
#[test]
fn body_source_identity_duplicate_record_offsets_and_escaped_transport_budget() {
    let f = Fixture::new();
    let first = msg("dup", None, "user", &"\"\\".repeat(150000));
    f.append(first.clone());
    f.append(msg("dup", None, "user", "last duplicate"));
    let ix = index(f.path()).unwrap();
    let first_record = &ix.records[0];
    let mut at = 0;
    let mut raw = String::new();
    loop {
        let chunk = body(
            f.path(),
            &ix.generation,
            "dup",
            Some(first_record.offset),
            at,
            131072,
        )
        .unwrap();
        assert!(serde_json::to_vec(&chunk).unwrap().len() <= MAX_BYTES);
        raw.push_str(&chunk.data);
        if let Some(next) = chunk.next_offset {
            assert!(next > at);
            at = next;
        } else {
            break;
        }
    }
    assert_eq!(raw, first.to_string());
    let last = body(f.path(), &ix.generation, "dup", None, 0, 1024).unwrap();
    assert!(last.data.contains("last duplicate"));
    assert!(body(
        f.path(),
        &ix.generation,
        "different",
        Some(first_record.offset),
        0,
        1024
    )
    .is_err());
    let other = Fixture::new();
    other.append(msg("dup", None, "user", "cross-source"));
    assert!(body(other.path(), &ix.generation, "dup", None, 0, 1024)
        .err()
        .unwrap()
        .starts_with("STALE_DETAIL"));
}
#[test]
fn usage_stats_provider_model_and_parsed_cache_are_shared_without_bodies_in_index() {
    let f = Fixture::new();
    f.append(json!({"type":"model_change","id":"m","parentId":null,"modelId":"synthetic-model","provider":"synthetic-provider"}));
    f.append(
        json!({"type":"thinking_level_change","id":"t","parentId":"m","thinkingLevel":"high"}),
    );
    f.append(json!({"type":"message","id":"a","parentId":"t","usage":{"totalTokens":7},"message":{"role":"assistant","model":"final-model","provider":"final-provider","usage":{"totalTokens":55,"input":30,"cacheRead":12,"cost":{"total":0.25}},"content":[{"type":"thinking","thinking":"private synthetic reasoning"},{"type":"text","text":"synthetic"}]}}));
    let d = f.page(req("all"));
    assert_eq!(d.stats.token_count, 62);
    assert_eq!(d.stats.context_tokens, Some(42));
    assert_eq!(d.stats.cost_total, 0.25);
    assert_eq!(d.stats.model.as_deref(), Some("final-model"));
    assert_eq!(d.stats.provider.as_deref(), Some("final-provider"));
    assert_eq!(d.stats.thinking_level.as_deref(), Some("high"));
    let ix = index(f.path()).unwrap();
    let mut file = File::open(&f.0).unwrap();
    let a = entry(&ix, &mut file, 2).unwrap();
    let b = entry(&ix, &mut file, 2).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert!(ix.records[2].meta.content.is_empty());
    assert!(ix.records[2].meta.summary.is_none());
    // Header-only getters stop at the small first record, even when the body
    // is giant or contains malformed appended bytes.
    f.append(msg("huge", Some("a"), "user", &"x".repeat(1_000_000)));
    assert_eq!(
        super::super::session_header(f.path()).unwrap().cwd,
        "/synthetic"
    );
}

/// Main may run only after explicit approval. Never prints session content,
/// IDs, cwd or paths; default ignored to keep synthetic checks isolated.
#[test]
#[ignore = "requires explicit approval and PI_VIEWER_DETAIL_BENCH_PATH; aggregate-only readonly"]
fn bounded_detail_readonly_benchmark() {
    let path = std::env::var("PI_VIEWER_DETAIL_BENCH_PATH")
        .expect("explicit approved benchmark path required");
    let start = std::time::Instant::now();
    let cold = page(&path, req("all")).expect("cold bounded detail page");
    let cold_ms = start.elapsed().as_millis();
    let start = std::time::Instant::now();
    let hot = page(&path, req("all")).expect("hot bounded detail page");
    let hot_ms = start.elapsed().as_millis();
    let index = index(&path).unwrap();
    let bytes = serde_json::to_vec(&cold).unwrap().len();
    let p = hot.page.unwrap();
    assert!(bytes <= MAX_BYTES);
    assert_eq!(cold.entries.len(), p.returned_entries);
    println!("{{\"sourceBytes\":{},\"totalEntries\":{},\"branchEntries\":{},\"returnedEntries\":{},\"payloadBytes\":{},\"indexEstimatedBytes\":{},\"indexCacheBytes\":{},\"parsedCacheBytes\":{},\"coldMs\":{},\"hotMs\":{},\"incompleteTail\":{},\"malformedLines\":{}}}",
        cold.size,p.total_entries,p.branch_entries,p.returned_entries,bytes,index.bytes,indices().lock().unwrap().bytes,parsed().lock().unwrap().bytes,cold_ms,hot_ms,p.incomplete_tail,p.malformed_lines);
}
