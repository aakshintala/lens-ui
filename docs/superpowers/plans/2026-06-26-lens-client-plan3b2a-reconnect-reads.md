# lens-client Plan 3b-2a — typed reconnect read surfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the two typed read surfaces the §7 reconnect/wake protocol consumes — `Sessions::items()` (the durable transcript, merged by item `id`) and a `SessionSnapshot` grown to carry the bucket-B chrome — modeled from the golden captures.

**Architecture:** Both are static-shape, byte-grounded REST reads (composer-2.5's strength), the same "typed wrapper, no `Value` to consumers" pattern as the Plan 2a–2e surface. `Sessions::items()` returns a typed `ItemList` envelope over the existing `stream::event::Item` union (completed here so every persisted item is reconcilable by `id`). `SessionSnapshot` grows the bucket-B scalars/collections (`GET /v1/sessions/{id}`). The §7 reconnect **state machine** that drives these reads (backoff, lifecycle events, items-replay, seq-dedup, normalizer reset) is **Plan 3b-2b** — out of scope here.

**Tech Stack:** Rust (edition 2024), `serde`/`serde_json`, `reqwest::blocking` (already deps). Reuses `Client::get_json` and the `stream::event::Item` union. No new dependencies.

## Global Constraints

- **MANDATORY** No `serde_json::Value` exposed to consumers. `Item::from_value` consumes a `Value` *internally* only (never returned); `ItemList`/`SessionSnapshot` expose typed fields via getters. (AGENTS.md typed-end-to-end.)
- **MANDATORY** The UI never panics — `Item::from_value` stays total (unmodeled item types → `Item::Other`, never a parse failure); reads return `Result`, never `unwrap` on wire data. (AGENTS.md.)
- **MANDATORY** Reconcile-by-`id` is load-bearing (typed-client §7 step 5: persisted items have **no `sequence_number`**, merged by item `id`). **Every** `Item` variant — including `Other` — must expose its `id`. (app-arch state-model §6.3.)
- **MANDATORY** `generated.rs` stays untouched — hand-model these (the item/snapshot schemas are under-typed in openapi). Run `cargo clippy --all-targets -- -D warnings` + `cargo fmt` clean before every commit.
- **MANDATORY** Ground-truth discipline — every field shape comes from the golden captures (`docs/spikes/captures/2026-06-26-sse/happy_path.items.json`, `…snapshot.json`), not memory or the §10 sketch. Collections that are **empty/null in the capture** (`todos`, `pending_elicitations`, `model_options`, `sandbox_status`) are **deferred** (left out of the struct, flagged ⚠ — not guessed), so the read can't silently break live on a shape we never saw. Scalars that are null in the capture but have an obvious type (`context_window`, `llm_model`) are modeled as `Option<…>`.
- Pin: omnigent `0.3.0.dev0` (`36b2a11c`). Live tests are `#[cfg(feature = "live-tests")]`.

**Scope of 3b-2a.** The typed reads: (1) complete the `Item` union (`ResourceEvent` variant, `id` on `Other`, an `id()` accessor) so `items()` is reconcilable; (2) `Sessions::items()` + `ItemList` envelope; (3–4) grow `SessionSnapshot` with the byte-grounded bucket-B chrome. **Out of scope (Plan 3b-2b):** the reconnect state machine, the `Reconnecting`/`Reconnected`/`Disconnected` lifecycle events, items-replay as `OutputItemDone`, seq-dedup, and the normalizer `seen_items` reset (all per typed-client §7 / the seams recorded there).

---

### Task 1: Complete the `Item` union (`ResourceEvent`, `id` on `Other`, `id()` accessor)

The persisted `/items` corpus contains a `resource_event` item type the stream `Item` union maps to `Other`, and `Item::Other` currently drops everything but `item_type` — so a resource event (or any unmodeled type) loses its `id` and can't be reconciled. Model `resource_event` typed, give `Other` its `id`, and add a total `id()` accessor.

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs` (the `Item` enum, `Item::from_value`, accessors)

**Interfaces:**
- Consumes: the existing `Item` union + `Item::from_value` (Plan 3a).
- Produces (public):
  - New variant `Item::ResourceEvent { id: String, resource_id: String, resource_type: String, event_type: String }`.
  - Changed variant `Item::Other { item_type: String, id: String }` (gains `id`).
  - `impl Item { pub fn id(&self) -> &str }` — total over every variant.
  - `Item::from_value` becomes `pub(crate)` (so `sessions::items()` in Task 2 can reuse it).

- [ ] **Step 1: Copy the items fixture**

```bash
cp docs/spikes/captures/2026-06-26-sse/happy_path.items.json crates/lens-client/tests/fixtures/sse/happy_path.items.json
```

- [ ] **Step 2: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `event.rs`:

```rust
    #[test]
    fn resource_event_item_is_typed_with_id() {
        let item = Item::from_value(serde_json::json!({
            "id": "rse_1", "type": "resource_event", "status": "completed",
            "event_type": "session.resource.created",
            "resource_id": "terminal_tui_main", "resource_type": "terminal",
            "resource": {"id": "terminal_tui_main", "object": "resource"}
        }));
        assert_eq!(
            item,
            Item::ResourceEvent {
                id: "rse_1".into(),
                resource_id: "terminal_tui_main".into(),
                resource_type: "terminal".into(),
                event_type: "session.resource.created".into(),
            }
        );
        assert_eq!(item.id(), "rse_1");
    }

    #[test]
    fn other_item_retains_its_id_for_reconcile() {
        let item = Item::from_value(serde_json::json!({
            "id": "x_9", "type": "native_tool", "kind": "web_search_call"
        }));
        assert_eq!(item, Item::Other { item_type: "native_tool".into(), id: "x_9".into() });
        assert_eq!(item.id(), "x_9"); // reconcile-by-id works even for unmodeled types
    }

    #[test]
    fn id_accessor_is_total_over_all_variants() {
        let msg = Item::from_value(serde_json::json!({"id":"m1","type":"message","role":"assistant","content":[]}));
        let fc = Item::from_value(serde_json::json!({"id":"fc1","type":"function_call","call_id":"c","name":"n","arguments":"{}","status":"completed"}));
        let fco = Item::from_value(serde_json::json!({"id":"fco1","type":"function_call_output","call_id":"c","output":"o"}));
        assert_eq!(msg.id(), "m1");
        assert_eq!(fc.id(), "fc1");
        assert_eq!(fco.id(), "fco1");
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p lens-client --lib stream::event 2>&1 | head -20`
Expected: FAIL — `no variant ResourceEvent`, `Other` has no field `id`, `no method id`.

- [ ] **Step 4: Implement**

In `event.rs`, add the `ResourceEvent` variant and `id` to `Other` (in the `pub enum Item` block, before `Other`):

```rust
    /// A persisted resource lifecycle item (`/items` only; the live stream carries
    /// these as `session.resource.*` SessionEvents instead). `resource_type` is
    /// e.g. `terminal`; `event_type` is the wire `session.resource.created` form.
    ResourceEvent {
        id: String,
        resource_id: String,
        resource_type: String,
        event_type: String,
    },
```

Change `Other`:

```rust
    /// Forward-compat for item types not yet modeled. Retains `id` so the state
    /// model can still reconcile it by `id` (typed-client §7 step 5).
    Other { item_type: String, id: String },
```

Add the accessor (in the existing `impl Item` block, or a new one):

```rust
impl Item {
    /// The item's stable `id` — the reconcile key for `GET /items` merge
    /// (persisted items carry no `sequence_number`; typed-client §7 step 5).
    pub fn id(&self) -> &str {
        match self {
            Item::Message { id, .. }
            | Item::FunctionCall { id, .. }
            | Item::FunctionCallOutput { id, .. }
            | Item::Error { id, .. }
            | Item::ResourceEvent { id, .. }
            | Item::Other { id, .. } => id,
        }
    }
}
```

In `Item::from_value`, make it `pub(crate)` and add the `resource_event` arm + carry `id` into `Other`. Change the signature line `fn from_value` → `pub(crate) fn from_value`, add before the `other =>` arm:

```rust
            "resource_event" => Item::ResourceEvent {
                id,
                resource_id: s("resource_id"),
                resource_type: s("resource_type"),
                event_type: s("event_type"),
            },
```

and change the catch-all to keep `id`:

```rust
            other => Item::Other { item_type: other.to_string(), id },
```

> `id`, `s`, `so` helpers already exist in `from_value` (Plan 3a). `id` is the `let id = …` bound at the top of the function — confirm it is still in scope at the `Other` arm (it is; it is bound before the `match`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::event`
Expected: PASS (new tests + all Plan 3a/3b-1 event tests; the existing `unmodeled_item_type_becomes_other_not_panic` test now also needs `id` — update its expected value to `Item::Other { item_type: "native_tool".into(), id: "x".into() }` if it asserts the full struct).

- [ ] **Step 6: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/event.rs crates/lens-client/tests/fixtures/sse/happy_path.items.json
git commit -m "feat(lens-client): complete Item union (ResourceEvent, id on Other, id() accessor)"
```

---

### Task 2: `Sessions::items()` + `ItemList` envelope

The durable transcript read. `GET /v1/sessions/{id}/items` returns a paginated envelope `{object, data, first_id, last_id, has_more}`; model it typed over the completed `Item` union, reusing `Item::from_value` so unmodeled types degrade to `Other` (with `id`) rather than failing the whole page.

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (add `ItemList`, `ItemsPage`, `Sessions::items`)
- Create: `crates/lens-client/tests/fixtures/sse/` already holds `happy_path.items.json` (Task 1)

**Interfaces:**
- Consumes: `Item`, `Item::from_value` (Task 1), `Client::get_json` (`pub(crate) fn get_json<T: DeserializeOwned>(&self, path: &str, query: &[(&'static str, String)]) -> Result<T>`).
- Produces (public):
  - `pub struct ItemsPage { pub limit: Option<u32>, pub after: Option<String>, pub before: Option<String>, pub order: Option<String> }` (`Default`) with a private `to_query`.
  - `pub struct ItemList` with getters `items(&self) -> &[Item]`, `has_more(&self) -> bool`, `first_id(&self) -> Option<&str>`, `last_id(&self) -> Option<&str>`.
  - `Sessions::items(&self, id: &SessionId, page: &ItemsPage) -> Result<ItemList>`.

- [ ] **Step 1: Write the failing test**

Add a test module/section in `sessions.rs` (match the file's existing `#[cfg(test)]` layout):

```rust
    #[test]
    fn item_list_parses_the_golden_items_envelope() {
        let raw = include_str!("../tests/fixtures/sse/happy_path.items.json");
        let list: super::ItemList = serde_json::from_str(raw).expect("parse items envelope");
        assert_eq!(list.items().len(), 11);
        assert!(!list.has_more());
        // The corpus opens with a resource_event and contains the function_call pair.
        assert!(matches!(list.items()[0], crate::stream::Item::ResourceEvent { .. }));
        assert!(list.items().iter().any(|i| matches!(i, crate::stream::Item::FunctionCall { .. })));
        assert!(list.items().iter().any(|i| matches!(i, crate::stream::Item::FunctionCallOutput { .. })));
        // Every item is reconcilable by a non-empty id.
        assert!(list.items().iter().all(|i| !i.id().is_empty()));
    }

    #[test]
    fn items_page_to_query_skips_none() {
        let q = super::ItemsPage { limit: Some(50), order: Some("asc".into()), ..Default::default() }.to_query();
        assert!(q.contains(&("limit", "50".to_string())));
        assert!(q.contains(&("order", "asc".to_string())));
        assert!(!q.iter().any(|(k, _)| *k == "after" || *k == "before"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client --lib sessions 2>&1 | head -20`
Expected: FAIL — `cannot find type ItemList` / `ItemsPage`.

- [ ] **Step 3: Implement `ItemList`, `ItemsPage`, `items()`**

Add to `sessions.rs` (near the other typed read wrappers). `ItemList` deserializes `data` as raw `Value`s and maps each through `Item::from_value` (internal `Value`, never exposed):

```rust
/// `GET /v1/sessions/{id}/items` — the durable, paginated transcript. Persisted
/// items carry no `sequence_number`; reconcile by `Item::id()` (typed-client §7).
#[derive(Debug)]
pub struct ItemList {
    items: Vec<crate::stream::Item>,
    has_more: bool,
    first_id: Option<String>,
    last_id: Option<String>,
}

impl ItemList {
    pub fn items(&self) -> &[crate::stream::Item] { &self.items }
    pub fn has_more(&self) -> bool { self.has_more }
    pub fn first_id(&self) -> Option<&str> { self.first_id.as_deref() }
    pub fn last_id(&self) -> Option<&str> { self.last_id.as_deref() }
}

// Internal wire envelope (private; `data` is Value only to feed Item::from_value).
#[derive(serde::Deserialize)]
struct RawItemList {
    #[serde(default)]
    data: Vec<serde_json::Value>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    first_id: Option<String>,
    #[serde(default)]
    last_id: Option<String>,
}

impl<'de> serde::Deserialize<'de> for ItemList {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let raw = RawItemList::deserialize(d)?;
        Ok(ItemList {
            items: raw.data.into_iter().map(crate::stream::Item::from_value).collect(),
            has_more: raw.has_more,
            first_id: raw.first_id,
            last_id: raw.last_id,
        })
    }
}

/// Pagination for `Sessions::items` (openapi `/v1/sessions/{id}/items`: limit,
/// after, before, order). All optional; `None` fields are omitted from the query.
#[derive(Debug, Default, Clone)]
pub struct ItemsPage {
    pub limit: Option<u32>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub order: Option<String>,
}

impl ItemsPage {
    fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(n) = self.limit { q.push(("limit", n.to_string())); }
        if let Some(a) = &self.after { q.push(("after", a.clone())); }
        if let Some(b) = &self.before { q.push(("before", b.clone())); }
        if let Some(o) = &self.order { q.push(("order", o.clone())); }
        q
    }
}
```

Add the method in the `impl Sessions` block (match the `self.client` field name used by `get`/`list`):

```rust
    /// `GET /v1/sessions/{id}/items` — the durable transcript. Blocking.
    pub fn items(&self, id: &SessionId, page: &ItemsPage) -> Result<ItemList> {
        self.client
            .get_json(&format!("/v1/sessions/{id}/items"), &page.to_query())
    }
```

> `get_json`'s query param is `&[(&'static str, String)]`; `to_query()` returns `Vec<(&'static str, String)>` which derefs to that slice. Confirm `crate::stream::Item` is the re-export path (`stream/mod.rs` re-exports `Item`); if `Item::from_value` is not visible, confirm Task 1 made it `pub(crate)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib sessions`
Expected: PASS (both new tests + existing sessions tests).

- [ ] **Step 5: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/sessions.rs
git commit -m "feat(lens-client): Sessions::items() + typed ItemList envelope"
```

---

### Task 3: `SessionSnapshot` bucket-B scalars + `ModelUsage`/`SkillRef`

Grow `SessionSnapshot` with the byte-grounded scalar chrome the §7 reconnect/wake restores (bucket B). This task adds the scalars + the two helper structs; Task 4 adds the collections + embedded items.

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (`SessionSnapshot` struct + getters; new `ModelUsage`, `SkillRef`)

**Interfaces:**
- Produces (public): new fields + getters on `SessionSnapshot` — `harness() -> &str`, `title() -> Option<&str>`, `runner_id() -> Option<&str>`, `host_id() -> Option<&str>`, `llm_model() -> Option<&str>`, `model_override() -> Option<&str>`, `reasoning_effort() -> Option<&str>`, `context_window() -> Option<i64>`, `last_total_tokens() -> Option<i64>`, `total_cost_usd() -> Option<f64>`, `permission_level() -> Option<i64>`, `workspace() -> Option<&str>`, `git_branch() -> Option<&str>`, `root_conversation_id() -> Option<&str>`, `parent_session_id() -> Option<&str>`, `sub_agent_name() -> Option<&str>`, `last_task_error() -> Option<&str>`. New `pub struct ModelUsage` (tokens + cost), `pub struct SkillRef { name, description }` with getters (used in Task 4).

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)]` in `sessions.rs`:

```rust
    #[test]
    fn snapshot_parses_bucket_b_scalars_from_golden() {
        let raw = include_str!("../tests/fixtures/sse/happy_path.snapshot.json");
        let s: super::SessionSnapshot = serde_json::from_str(raw).expect("parse snapshot");
        assert_eq!(s.harness(), "claude-sdk");
        assert_eq!(s.workspace(), Some("/Users/aakshintala/work/lens"));
        assert_eq!(s.permission_level(), Some(4));
        assert_eq!(s.root_conversation_id(), Some("conv_91d8bde71cae41e7b32e01a648e00f72"));
        // Byte fact: total_cost_usd present, llm_model/context_window null on this turn.
        assert!(s.total_cost_usd().unwrap() > 0.0);
        assert_eq!(s.llm_model(), None);
        assert_eq!(s.context_window(), None);
    }
```

- [ ] **Step 2: Copy the snapshot fixture + run to verify it fails**

```bash
cp docs/spikes/captures/2026-06-26-sse/happy_path.snapshot.json crates/lens-client/tests/fixtures/sse/happy_path.snapshot.json
```

Run: `cargo test -p lens-client --lib sessions::tests::snapshot_parses_bucket_b_scalars_from_golden 2>&1 | head -20`
Expected: FAIL — `no method harness` etc.

- [ ] **Step 3: Implement the scalars + helper structs**

Add the new fields to the `SessionSnapshot` struct (all `#[serde(default)]`, after the existing fields):

```rust
    #[serde(default)]
    harness: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    runner_id: Option<String>,
    #[serde(default)]
    host_id: Option<String>,
    #[serde(default)]
    llm_model: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(default)]
    context_window: Option<i64>,
    #[serde(default)]
    last_total_tokens: Option<i64>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    permission_level: Option<i64>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    git_branch: Option<String>,
    #[serde(default)]
    root_conversation_id: Option<String>,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(default)]
    sub_agent_name: Option<String>,
    #[serde(default)]
    last_task_error: Option<String>,
```

Add the getters to `impl SessionSnapshot`:

```rust
    pub fn harness(&self) -> &str { &self.harness }
    pub fn title(&self) -> Option<&str> { self.title.as_deref() }
    pub fn runner_id(&self) -> Option<&str> { self.runner_id.as_deref() }
    pub fn host_id(&self) -> Option<&str> { self.host_id.as_deref() }
    pub fn llm_model(&self) -> Option<&str> { self.llm_model.as_deref() }
    pub fn model_override(&self) -> Option<&str> { self.model_override.as_deref() }
    pub fn reasoning_effort(&self) -> Option<&str> { self.reasoning_effort.as_deref() }
    pub fn context_window(&self) -> Option<i64> { self.context_window }
    pub fn last_total_tokens(&self) -> Option<i64> { self.last_total_tokens }
    pub fn total_cost_usd(&self) -> Option<f64> { self.total_cost_usd }
    pub fn permission_level(&self) -> Option<i64> { self.permission_level }
    pub fn workspace(&self) -> Option<&str> { self.workspace.as_deref() }
    pub fn git_branch(&self) -> Option<&str> { self.git_branch.as_deref() }
    pub fn root_conversation_id(&self) -> Option<&str> { self.root_conversation_id.as_deref() }
    pub fn parent_session_id(&self) -> Option<&str> { self.parent_session_id.as_deref() }
    pub fn sub_agent_name(&self) -> Option<&str> { self.sub_agent_name.as_deref() }
    pub fn last_task_error(&self) -> Option<&str> { self.last_task_error.as_deref() }
```

Add the helper structs (near `SessionSnapshot`), used by Task 4:

```rust
/// Per-model token+cost usage from `usage_by_model` on the session snapshot.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ModelUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
    #[serde(default)]
    total_cost_usd: f64,
}

impl ModelUsage {
    pub fn input_tokens(&self) -> i64 { self.input_tokens }
    pub fn output_tokens(&self) -> i64 { self.output_tokens }
    pub fn total_tokens(&self) -> i64 { self.total_tokens }
    pub fn cache_read_input_tokens(&self) -> i64 { self.cache_read_input_tokens }
    pub fn cache_creation_input_tokens(&self) -> i64 { self.cache_creation_input_tokens }
    pub fn total_cost_usd(&self) -> f64 { self.total_cost_usd }
}

/// An attached skill summary from `skills` on the session snapshot.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SkillRef {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

impl SkillRef {
    pub fn name(&self) -> &str { &self.name }
    pub fn description(&self) -> &str { &self.description }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib sessions`
Expected: PASS. (`ModelUsage`/`SkillRef` are unused until Task 4 — add `#[allow(dead_code)]` on them only if clippy blocks the commit; removed in Task 4.)

- [ ] **Step 5: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/sessions.rs crates/lens-client/tests/fixtures/sse/happy_path.snapshot.json
git commit -m "feat(lens-client): SessionSnapshot bucket-B scalars + ModelUsage/SkillRef"
```

---

### Task 4: `SessionSnapshot` bucket-B collections + embedded items

Add the **byte-grounded, non-empty** collection chrome: `usage_by_model` (typed map), `skills`, and the embedded `items` (`Vec<Item>`, present when fetched with `include_items`). The collections that are **empty/null in the only capture** — `todos`, `pending_elicitations`, `model_options`, `sandbox_status` — are **deferred** (not modeled here): their non-empty shape is unverified, and modeling them now risks a live-deser break (e.g. `TodoItem` is not `Deserialize` and its wire key is `activeForm`; `pending_elicitations` are likely objects, not id strings). They are left out of the struct entirely — serde ignores unknown wire fields, so the snapshot still parses when they are present-but-empty.

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (`SessionSnapshot` collection fields + getters)

**Interfaces:**
- Consumes: `ModelUsage`, `SkillRef` (Task 3), `crate::stream::Item` (Task 1).
- Produces (public): getters `usage_by_model(&self) -> &BTreeMap<String, ModelUsage>`, `skills(&self) -> &[SkillRef]`, `items(&self) -> &[Item]` (empty unless fetched with `include_items`).

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)]` in `sessions.rs`:

```rust
    #[test]
    fn snapshot_parses_bucket_b_collections_from_golden() {
        let raw = include_str!("../tests/fixtures/sse/happy_path.snapshot.json");
        let s: super::SessionSnapshot = serde_json::from_str(raw).expect("parse snapshot");
        // usage_by_model: one model with token+cost detail.
        let usage = s.usage_by_model();
        assert!(usage.contains_key("claude-opus-4-8"));
        assert!(usage["claude-opus-4-8"].total_tokens() > 0);
        // skills: 20 attached, each with a name.
        assert_eq!(s.skills().len(), 20);
        assert!(s.skills().iter().all(|sk| !sk.name().is_empty()));
        // embedded items: 11 (snapshot was captured with include_items).
        assert_eq!(s.items().len(), 11);
        assert_eq!(s.items()[0].id().is_empty(), false);
        // (todos/pending_elicitations/model_options are empty in this capture and
        //  deferred — the snapshot still parses with them present-but-empty.)
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client --lib sessions::tests::snapshot_parses_bucket_b_collections_from_golden 2>&1 | head -20`
Expected: FAIL — `no method usage_by_model` etc.

- [ ] **Step 3: Implement the collections**

Add fields to `SessionSnapshot`. The embedded `items` deserialize from raw `Value` via `Item::from_value` (a custom field path), so add a `#[serde(default, deserialize_with = …)]` or deserialize as `Vec<Value>` into a private field and map in a constructor. Simplest: a private `items_raw: Vec<serde_json::Value>` field plus a lazily-mapped public getter is awkward; instead map at deserialize via a helper. Use a `deserialize_with` function:

```rust
    #[serde(default)]
    usage_by_model: std::collections::BTreeMap<String, ModelUsage>,
    #[serde(default)]
    skills: Vec<SkillRef>,
    #[serde(default, deserialize_with = "de_items")]
    items: Vec<crate::stream::Item>,
    // ⚠ DEFERRED (empty/null in the only capture — model when non-empty, Plan 3b-2b/
    //   config-time): todos (TodoItem is not Deserialize; wire key `activeForm`),
    //   pending_elicitations (likely objects, not id strings), model_options,
    //   sandbox_status. Left out of the struct: serde skips unknown wire fields,
    //   so the snapshot still parses with them present-but-empty.
```

Add the `deserialize_with` helper near `SessionSnapshot` (maps wire `Value`s through the total `Item::from_value`):

```rust
fn de_items<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<crate::stream::Item>, D::Error> {
    let raw: Vec<serde_json::Value> = Vec::deserialize(d)?;
    Ok(raw.into_iter().map(crate::stream::Item::from_value).collect())
}
```

(Add `use serde::Deserialize;` to the imports if `Vec::deserialize` is not already in scope.)

Add getters to `impl SessionSnapshot`:

```rust
    pub fn usage_by_model(&self) -> &std::collections::BTreeMap<String, ModelUsage> { &self.usage_by_model }
    pub fn skills(&self) -> &[SkillRef] { &self.skills }
    /// The transcript items embedded in the snapshot — non-empty only when fetched
    /// with `GetOpts { include_items: true }`. The standalone paginated read is
    /// `Sessions::items()`.
    pub fn items(&self) -> &[crate::stream::Item] { &self.items }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib sessions`
Expected: PASS (Task 3 + Task 4 snapshot tests + existing). (Remove any `#[allow(dead_code)]` added to `ModelUsage`/`SkillRef` in Task 3 — now used.)

- [ ] **Step 5: Full crate sweep + live read (optional)**

Run: `cargo test -p lens-client --lib 2>&1 | tail -3`
Expected: PASS — Plan 3a/3b-1 suite + the new reads, no regressions.

If a live server is available, sanity-check the reads against it (no assertion harness needed — eyeball the typed output):

```bash
LENS_OMNIGENT_URL=http://127.0.0.1:6767 LENS_OMNIGENT_SESSION_ID=<conv_…> \
  cargo test -p lens-client --features live-tests --test live_stream -- --nocapture
# (extend live_stream or add a one-off live read of items()/get(include_items) if desired)
```

(If no server, record that the live read was not run — the golden-fixture tests are the byte-grounded gate.)

- [ ] **Step 6: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/sessions.rs
git commit -m "feat(lens-client): SessionSnapshot bucket-B collections + embedded items"
```

---

## Out of scope for 3b-2a (Plan 3b-2b)

- **The §7 reconnect state machine:** disconnect detection at the reader's `Err(_) => return` seam, exponential backoff (~7s), and the synthetic `ServerStreamEvent::{Reconnecting { attempt }, Reconnected { gap }, Disconnected}` lifecycle (designed 2026-06-26, typed-client §7/§10/§11).
- **Items-replay:** converting the `ItemList` from this plan into replayed `ResponseEvent::OutputItemDone` events on reconnect; **`Reconnected` precedes** all replayed history (§7a ordering).
- **Bucket-B chrome restore on reconnect:** how the snapshot grown here is applied — the open design question (crate emits synthetic chrome `SessionEvent`s vs. consumer applies the snapshot) to resolve before 3b-2b.
- **Seq-dedup of the live overlap** + **normalizer `seen_items` reset on `Reconnected { gap != Some(0) }`** (the two seams recorded in typed-client §7).
- **Reader architecture change:** the reader thread gaining a re-open capability (`Client` + `SessionId` or a reopen closure) so it can drive snapshot/items/re-open internally.

## Self-Review notes

- **Spec coverage:** `GET /items` typed + reconcile-by-id (§7 step 5) → Tasks 1–2 (`Item::id()` total, incl. `Other`); bucket-B snapshot chrome (§7 step 4 / app-arch §6.3 wake) → Tasks 3–4 (scalars + collections + embedded items). The reconnect *protocol* that consumes them → explicitly deferred to 3b-2b.
- **No-Value rule:** `serde_json::Value` appears only internally (`RawItemList.data`, `de_items`, `Item::from_value`); `ItemList`/`SessionSnapshot` expose typed getters. ✓
- **Never-panic / total:** `Item::from_value` stays total (unmodeled → `Other` with `id`); `Item::id()` is an exhaustive match; reads return `Result`. ✓
- **Ground-truth:** every field/test value is from `happy_path.items.json` / `happy_path.snapshot.json`; empty/null-in-capture collections (`todos`, `model_options`, `sandbox_status`, `pending_elicitations`) are **deferred** (not guessed), flagged ⚠. ✓
- **Type consistency:** `Item::id()`, `Item::ResourceEvent`, `Item::Other { item_type, id }`, `ItemList::items()`, `ItemsPage::to_query`, `ModelUsage`, `SkillRef`, and the `SessionSnapshot` getters are named identically across tasks and match the Plan 3a `Item` union. ✓
- **Process:** static-shape, byte-grounded REST modeling = composer-2.5's strength; per-task cross-family review still applies at the seams (no-Value + reconcile-by-id are the load-bearing checks), but the temporal-logic intensity of Plan 3 lives in 3b-2b, not here.
