//! SqliteMemory 集成测试 -- 拆分前的安全网
//!
//! 覆盖核心路径: 构造 / CRUD / list / count / recall(FTS5+时间) /
//! agent 归因 / supersede / stats / export / namespace。
//!
//! 所有测试使用 tempfile 隔离工作目录, NoopEmbedding (dim=0) →
//! recall 退化为 FTS5 + LIKE 回退路径。

use shadow_core::kennel::memory::{
    ExportFilter, MemoryCategory, MemoryKind, StoreOptions,
};
use shadow_core::Memory;
use shadow_memory::sqlite::SqliteMemory;
use tempfile::TempDir;

// ── 构造与健康检查 ─────────────────────────────────────────────

fn make() -> (SqliteMemory, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let mem = SqliteMemory::new("sqlite", dir.path()).expect("open sqlite");
    (mem, dir)
}

#[tokio::test]
async fn new_and_health_check() {
    let (mem, _d) = make();
    assert_eq!(mem.name(), "sqlite");
    assert!(mem.health_check().await);
}

// ── 基础 CRUD ──────────────────────────────────────────────────

#[tokio::test]
async fn store_then_get_returns_entry() {
    let (mem, _d) = make();
    mem.store("k1", "hello world", MemoryCategory::Core, None)
        .await
        .expect("store");

    let got = mem.get("k1").await.expect("get").expect("present");
    assert_eq!(got.key, "k1");
    assert_eq!(got.content, "hello world");
    assert_eq!(got.category, MemoryCategory::Core);
    assert_eq!(got.namespace, "default");
    assert!(!got.pinned);
    assert!(got.superseded_by.is_none());
}

#[tokio::test]
async fn get_missing_key_returns_none() {
    let (mem, _d) = make();
    let got = mem.get("nope").await.expect("get");
    assert!(got.is_none());
}

#[tokio::test]
async fn store_same_key_upserts_content() {
    let (mem, _d) = make();
    mem.store("k", "v1", MemoryCategory::Core, None)
        .await
        .expect("store v1");
    mem.store("k", "v2", MemoryCategory::Core, None)
        .await
        .expect("store v2");

    let got = mem.get("k").await.expect("get").expect("present");
    assert_eq!(got.content, "v2");
    // count should be 1 (upsert, not insert)
    assert_eq!(mem.count().await.expect("count"), 1);
}

#[tokio::test]
async fn forget_existing_returns_true() {
    let (mem, _d) = make();
    mem.store("k", "v", MemoryCategory::Core, None)
        .await
        .expect("store");
    assert!(mem.forget("k").await.expect("forget"));
    assert!(mem.get("k").await.expect("get").is_none());
}

#[tokio::test]
async fn forget_missing_returns_false() {
    let (mem, _d) = make();
    assert!(!mem.forget("ghost").await.expect("forget"));
}

// ── list / count ───────────────────────────────────────────────

#[tokio::test]
async fn list_all_returns_unsuperseded() {
    let (mem, _d) = make();
    mem.store("a", "alpha", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("b", "beta", MemoryCategory::Daily, None)
        .await
        .expect("s");

    let all = mem.list(None, None).await.expect("list");
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn list_filtered_by_category() {
    let (mem, _d) = make();
    mem.store("a", "alpha", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("b", "beta", MemoryCategory::Daily, None)
        .await
        .expect("s");
    mem.store("c", "gamma", MemoryCategory::Core, None)
        .await
        .expect("s");

    let cores = mem
        .list(Some(&MemoryCategory::Core), None)
        .await
        .expect("list");
    assert_eq!(cores.len(), 2);
    for e in &cores {
        assert_eq!(e.category, MemoryCategory::Core);
    }
}

#[tokio::test]
async fn list_filtered_by_session() {
    let (mem, _d) = make();
    mem.store("a", "alpha", MemoryCategory::Core, Some("s1"))
        .await
        .expect("s");
    mem.store("b", "beta", MemoryCategory::Core, Some("s2"))
        .await
        .expect("s");

    let s1 = mem.list(None, Some("s1")).await.expect("list");
    assert_eq!(s1.len(), 1);
    assert_eq!(s1[0].key, "a");
}

#[tokio::test]
async fn count_tracks_rows() {
    let (mem, _d) = make();
    assert_eq!(mem.count().await.expect("count"), 0);
    mem.store("a", "x", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("b", "y", MemoryCategory::Core, None)
        .await
        .expect("s");
    assert_eq!(mem.count().await.expect("count"), 2);
}

#[tokio::test]
async fn count_in_scope_namespace_category() {
    let (mem, _d) = make();
    mem.store_with_options(
        "a",
        "x",
        MemoryCategory::Core,
        None,
        StoreOptions::default().with_namespace("ns1"),
    )
    .await
    .expect("s");
    mem.store_with_options(
        "b",
        "y",
        MemoryCategory::Daily,
        None,
        StoreOptions::default().with_namespace("ns1"),
    )
    .await
    .expect("s");
    mem.store_with_options(
        "c",
        "z",
        MemoryCategory::Core,
        None,
        StoreOptions::default().with_namespace("ns2"),
    )
    .await
    .expect("s");

    assert_eq!(
        mem.count_in_scope(Some("ns1"), None).await.expect("c"),
        2
    );
    assert_eq!(
        mem.count_in_scope(Some("ns1"), Some(&MemoryCategory::Core))
            .await
            .expect("c"),
        1
    );
    assert_eq!(
        mem.count_in_scope(None, Some(&MemoryCategory::Core))
            .await
            .expect("c"),
        2
    );
}

// ── recall ─────────────────────────────────────────────────────

#[tokio::test]
async fn recall_by_keyword_fts5() {
    let (mem, _d) = make();
    mem.store("rust", "how to open a file in rust", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("python", "how to read a file in python", MemoryCategory::Core, None)
        .await
        .expect("s");

    let hits = mem
        .recall("rust file", 10, None, None, None)
        .await
        .expect("recall");
    assert!(!hits.is_empty(), "fts5 should find the rust entry");
    assert!(hits.iter().any(|e| e.key == "rust"));
}

#[tokio::test]
async fn recall_time_only_with_empty_query() {
    let (mem, _d) = make();
    mem.store("a", "first", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("b", "second", MemoryCategory::Core, None)
        .await
        .expect("s");

    // empty query → recall_by_time_only path
    let hits = mem.recall("", 10, None, None, None).await.expect("recall");
    assert_eq!(hits.len(), 2);
}

#[tokio::test]
async fn recall_star_is_time_only() {
    let (mem, _d) = make();
    mem.store("a", "x", MemoryCategory::Core, None)
        .await
        .expect("s");

    let hits = mem.recall("*", 10, None, None, None).await.expect("recall");
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn recall_respects_limit() {
    let (mem, _d) = make();
    for i in 0..5 {
        mem.store(&format!("k{i}"), "common token", MemoryCategory::Core, None)
            .await
            .expect("s");
    }
    let hits = mem
        .recall("common", 2, None, None, None)
        .await
        .expect("recall");
    assert!(hits.len() <= 2);
}

#[tokio::test]
async fn recall_filtered_by_session() {
    let (mem, _d) = make();
    mem.store("a", "shared keyword", MemoryCategory::Core, Some("s1"))
        .await
        .expect("s");
    mem.store("b", "shared keyword", MemoryCategory::Core, Some("s2"))
        .await
        .expect("s");

    let hits = mem
        .recall("shared", 10, Some("s1"), None, None)
        .await
        .expect("recall");
    assert!(hits.iter().all(|e| e.session_id.as_deref() == Some("s1")));
}

// ── namespace ──────────────────────────────────────────────────

#[tokio::test]
async fn recall_namespaced_filters() {
    let (mem, _d) = make();
    mem.store_with_options(
        "a",
        "shared content",
        MemoryCategory::Core,
        None,
        StoreOptions::default().with_namespace("alpha"),
    )
    .await
    .expect("s");
    mem.store_with_options(
        "b",
        "shared content",
        MemoryCategory::Core,
        None,
        StoreOptions::default().with_namespace("beta"),
    )
    .await
    .expect("s");

    let alpha = mem
        .recall_namespaced("alpha", "shared", 10, None, None, None)
        .await
        .expect("recall");
    assert!(alpha.iter().all(|e| e.namespace == "alpha"));
    assert!(!alpha.is_empty());
}

// ── supersede ──────────────────────────────────────────────────

#[tokio::test]
async fn supersede_marks_old_entries() {
    let (mem, _d) = make();
    mem.store("old", "v1", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("new", "v2", MemoryCategory::Core, None)
        .await
        .expect("s");

    let old = mem.get("old").await.expect("g").expect("present");
    let new = mem.get("new").await.expect("g").expect("present");

    mem.supersede(&[old.id.clone()], &new.id).await.expect("supersede");

    // superseded entry should be filtered from list
    let listed: Vec<_> = mem
        .list(Some(&MemoryCategory::Core), None)
        .await
        .expect("list")
        .into_iter()
        .filter(|e| e.key == "old")
        .collect();
    assert!(listed.is_empty(), "superseded entry should not appear in list");

    // but get() by key still returns it (get does not filter superseded_by)
    let direct = mem.get("old").await.expect("g").expect("present");
    assert_eq!(direct.superseded_by, Some(new.id));
}

// ── stats ──────────────────────────────────────────────────────

#[tokio::test]
async fn stats_reports_counts() {
    let (mem, _d) = make();
    mem.store("a", "alpha", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("b", "beta", MemoryCategory::Daily, None)
        .await
        .expect("s");

    let s = mem.stats().await.expect("stats");
    assert_eq!(s.total_rows, 2);
    assert_eq!(s.superseded_rows, 0);
    assert_eq!(s.pinned_rows, 0);
    assert!(s.bytes > 0);
    // by_category should contain both "core" and "daily"
    let cats: std::collections::HashMap<String, u64> = s.by_category.into_iter().collect();
    assert_eq!(cats.get("core"), Some(&1));
    assert_eq!(cats.get("daily"), Some(&1));
}

#[tokio::test]
async fn stats_counts_pinned_and_superseded() {
    let (mem, _d) = make();
    mem.store_with_options(
        "a",
        "x",
        MemoryCategory::Core,
        None,
        StoreOptions::default().pinned(true),
    )
    .await
    .expect("s");
    mem.store("b", "y", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("c", "z", MemoryCategory::Core, None)
        .await
        .expect("s");

    let b = mem.get("b").await.expect("g").expect("p");
    let c = mem.get("c").await.expect("g").expect("p");
    mem.supersede(&[b.id], &c.id).await.expect("supersede");

    let s = mem.stats().await.expect("stats");
    assert_eq!(s.total_rows, 3);
    assert_eq!(s.pinned_rows, 1);
    assert_eq!(s.superseded_rows, 1);
}

// ── export ─────────────────────────────────────────────────────

#[tokio::test]
async fn export_by_category_filter() {
    let (mem, _d) = make();
    mem.store("a", "alpha", MemoryCategory::Core, None)
        .await
        .expect("s");
    mem.store("b", "beta", MemoryCategory::Daily, None)
        .await
        .expect("s");

    let exported = mem
        .export(&ExportFilter {
            namespace: None,
            session_id: None,
            category: Some(MemoryCategory::Core),
            since: None,
            until: None,
        })
        .await
        .expect("export");
    assert!(exported.iter().all(|e| e.category == MemoryCategory::Core));
    assert_eq!(exported.len(), 1);
}

// ── store_with_options: kind / pinned ──────────────────────────

#[tokio::test]
async fn store_with_kind_and_pinned_roundtrip() {
    let (mem, _d) = make();
    mem.store_with_options(
        "proc",
        "how to open a file",
        MemoryCategory::Core,
        None,
        StoreOptions::default()
            .with_kind(MemoryKind::Procedural)
            .pinned(true),
    )
    .await
    .expect("s");

    let got = mem.get("proc").await.expect("g").expect("present");
    assert_eq!(got.kind, Some(MemoryKind::Procedural));
    assert!(got.pinned);
}

// ── agent scoping ──────────────────────────────────────────────

#[tokio::test]
async fn ensure_agent_uuid_is_idempotent() {
    let (mem, _d) = make();
    let id1 = mem.ensure_agent_uuid("alice").await.expect("uuid");
    let id2 = mem.ensure_agent_uuid("alice").await.expect("uuid");
    assert_eq!(id1, id2, "same alias must resolve to same UUID");

    let bob = mem.ensure_agent_uuid("bob").await.expect("uuid");
    assert_ne!(id1, bob, "different aliases must get different UUIDs");
}

#[tokio::test]
async fn store_with_agent_and_recall_for_agents() {
    let (mem, _d) = make();
    let alice = mem.ensure_agent_uuid("alice").await.expect("uuid");
    let bob = mem.ensure_agent_uuid("bob").await.expect("uuid");

    mem.store_with_agent(
        "prefs",
        "alice likes rust",
        MemoryCategory::Core,
        None,
        None,
        None,
        Some(&alice),
    )
    .await
    .expect("s");
    mem.store_with_agent(
        "prefs",
        "bob likes python",
        MemoryCategory::Core,
        None,
        None,
        None,
        Some(&bob),
    )
    .await
    .expect("s");

    // same key "prefs" exists for both agents (per-agent uniqueness)
    assert_eq!(mem.count().await.expect("count"), 2);

    // alice's recall should only see alice's prefs
    let alice_hits = mem
        .recall_for_agents(&[&alice], "likes", 10, None, None, None)
        .await
        .expect("recall");
    assert!(alice_hits.iter().all(|e| e.content.contains("alice")));

    // cross-agent recall sees both
    let both = mem
        .recall_for_agents(&[&alice, &bob], "likes", 10, None, None, None)
        .await
        .expect("recall");
    assert_eq!(both.len(), 2);
}

#[tokio::test]
async fn get_for_agent_scopes_by_agent_id() {
    let (mem, _d) = make();
    let alice = mem.ensure_agent_uuid("alice").await.expect("uuid");
    let bob = mem.ensure_agent_uuid("bob").await.expect("uuid");

    mem.store_with_agent(
        "shared_key",
        "alice's version",
        MemoryCategory::Core,
        None,
        None,
        None,
        Some(&alice),
    )
    .await
    .expect("s");
    mem.store_with_agent(
        "shared_key",
        "bob's version",
        MemoryCategory::Core,
        None,
        None,
        None,
        Some(&bob),
    )
    .await
    .expect("s");

    let alice_only = mem.get_for_agent("shared_key", &alice).await.expect("g").expect("present");
    assert_eq!(alice_only.content, "alice's version");

    let bob_only = mem.get_for_agent("shared_key", &bob).await.expect("g").expect("present");
    assert_eq!(bob_only.content, "bob's version");
}

