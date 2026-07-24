// Shared helpers for christiangeorgelucas/fulltext-search-tools nodes: schema
// construction, ephemeral index build, analyzer validation/tokenization, and
// term/highlight extraction over tantivy's query tree. Every node builds its
// own index (or none, for query-only nodes) and discards it at the end of the
// call — nothing here persists across invocations.

use std::collections::HashSet;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, STORED, STRING,
};
use tantivy::tokenizer::{Token, TokenizerManager};
use tantivy::{doc, Index, IndexWriter, TantivyDocument, Term};

use crate::gen::messages::Document;

// tantivy's query grammar is NOT simply "recurse per nesting level and blow
// the stack" — empirically (see the probe that motivated this constant) a
// query of adjacent/implicitly-grouped nested clauses with no explicit
// AND/OR between them (e.g. bare "((((term))))" or nested quoted phrases)
// costs EXPONENTIAL parse time in the nesting depth: ~300us at depth 4,
// ~3ms at 8, ~45ms at 12, ~720ms at 16, and it did not return within 3s at
// depth 20 on ordinary dev hardware. The SAME depth with explicit AND/OR
// operators between clauses is linear and cheap even past depth 24 — the
// blowup is specifically the parser's ambiguity-resolution for adjacent
// clauses, not recursion depth per se. 12 keeps the known-bad shapes under
// ~50ms with a comfortable margin while still allowing real, human-written
// nested boolean queries (which essentially never need more than a handful
// of nesting levels). This is a heuristic fast-reject, not a proof of
// safety for every possible query shape — see `parse_with_timeout` below
// for the actual backstop.
pub const MAX_QUERY_NESTING_DEPTH: usize = 12;
// Backstop for any pathological query shape the depth heuristic above does
// not catch (tantivy's grammar is a third-party dependency; its exact set of
// expensive constructs is not something this package can fully enumerate).
// A hard wall-clock deadline on the actual parse means a caller NEVER waits
// indefinitely for a response, regardless of what shape of query triggers
// the cost — see `parse_with_timeout`.
pub const QUERY_PARSE_TIMEOUT: Duration = Duration::from_millis(1500);
pub const DEFAULT_LIMIT: i32 = 10;
pub const DEFAULT_SNIPPET_CHARS: usize = 200;
pub const VALID_ANALYZERS: [&str; 4] = ["default", "en_stem", "whitespace", "raw"];

/// Bound-check a query string BEFORE it ever reaches tantivy's query parser.
/// Rejects the empty string and anything nested past
/// `MAX_QUERY_NESTING_DEPTH` levels of `(`/`[`/`{` — a cheap, fast
/// first-line filter for the known-expensive shapes (see
/// `MAX_QUERY_NESTING_DEPTH`'s doc). This alone is a heuristic, not a proof
/// of safety; every actual parse additionally runs under
/// `parse_with_timeout`'s wall-clock deadline as the real backstop.
pub fn validate_query(query: &str) -> Result<(), &'static str> {
    if query.trim().is_empty() {
        return Err("EMPTY_QUERY");
    }
    let mut depth: i64 = 0;
    for c in query.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ => {}
        }
        if depth as usize > MAX_QUERY_NESTING_DEPTH {
            return Err("QUERY_TOO_DEEPLY_NESTED");
        }
    }
    Ok(())
}

/// Run `f` (a query parse) on its own thread with a hard `QUERY_PARSE_TIMEOUT`
/// wall-clock deadline. tantivy's query grammar has an empirically-confirmed
/// exponential-time worst case (see `MAX_QUERY_NESTING_DEPTH`) that a fixed
/// heuristic cannot fully rule out for every possible query shape — this is
/// the actual backstop: if `f` has not finished within the deadline, the
/// caller gets a clean structured error immediately instead of hanging. The
/// spawned thread is deliberately abandoned rather than joined/cancelled (Rust
/// has no safe thread-cancellation primitive) — a still-running pathological
/// parse can keep consuming CPU in the background until it completes or the
/// process recycles, but it can no longer make a CALLER wait indefinitely,
/// which is the failure mode this exists to close.
pub fn parse_with_timeout<T, F>(f: F) -> Result<T, &'static str>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(QUERY_PARSE_TIMEOUT).map_err(|_| "QUERY_TOO_COMPLEX")
}

/// Validate `analyzer`, defaulting empty to "default". Returns the resolved
/// name or a structured error code.
pub fn normalize_analyzer(analyzer: &str) -> Result<String, &'static str> {
    let name = if analyzer.is_empty() { "default" } else { analyzer };
    if VALID_ANALYZERS.contains(&name) {
        Ok(name.to_string())
    } else {
        Err("INVALID_ANALYZER")
    }
}

/// Bound-check a document set before any index is built.
pub fn validate_documents(docs: &[Document]) -> Result<(), &'static str> {
    if docs.is_empty() {
        return Err("EMPTY_DOCUMENTS");
    }
    Ok(())
}

/// The identifier a Hit/Explain target resolves to for a given input
/// document: its own `id` if set, else its 0-based input position.
pub fn resolve_id(doc: &Document, idx: usize) -> String {
    if doc.id.is_empty() {
        idx.to_string()
    } else {
        doc.id.clone()
    }
}

/// Run the named analyzer's tokenizer/filter pipeline over `text`, standalone
/// (no index involved). Panics only if `analyzer_name` was not first checked
/// by `normalize_analyzer` (all four names are always registered by tantivy's
/// default TokenizerManager).
pub fn tokenize_text(text: &str, analyzer_name: &str) -> Vec<Token> {
    let manager = TokenizerManager::default();
    let mut analyzer = manager
        .get(analyzer_name)
        .expect("analyzer_name must be pre-validated by normalize_analyzer");
    let mut stream = analyzer.token_stream(text);
    let mut out = Vec::new();
    while let Some(tok) = stream.next() {
        out.push(tok.clone());
    }
    out
}

/// The three fields shared by every document-set-backed node (Search, Explain).
#[derive(Clone, Copy)]
pub struct SearchFields {
    pub id: Field,
    pub idx: Field,
    pub text: Field,
}

fn build_search_schema(analyzer_name: &str) -> (Schema, SearchFields) {
    let mut schema_builder = Schema::builder();
    let id_field = schema_builder.add_text_field("id", STRING | STORED);
    let idx_field = schema_builder.add_i64_field("doc_index", STORED);
    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer(analyzer_name)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text_opts = TextOptions::default()
        .set_indexing_options(text_indexing)
        .set_stored();
    let text_field = schema_builder.add_text_field("text", text_opts);
    let schema = schema_builder.build();
    (schema, SearchFields { id: id_field, idx: idx_field, text: text_field })
}

/// A schema with just the single "text" field, for query-only nodes
/// (ParseQuery, Snippet) that never build a document index.
pub fn build_text_only_schema(analyzer_name: &str) -> (Schema, Field) {
    let mut schema_builder = Schema::builder();
    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer(analyzer_name)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text_opts = TextOptions::default().set_indexing_options(text_indexing);
    let text_field = schema_builder.add_text_field("text", text_opts);
    (schema_builder.build(), text_field)
}

/// Build a fresh, purely in-memory index over `docs`, single-threaded (small
/// input, determinism over throughput), and commit it. The caller is
/// responsible for dropping `Index`/`IndexWriter` when done — nothing here
/// touches disk or survives the call.
pub fn build_index(docs: &[Document], analyzer_name: &str) -> Result<(Index, SearchFields), String> {
    let (schema, fields) = build_search_schema(analyzer_name);
    let index = Index::create_in_ram(schema);
    let mut writer: IndexWriter = index
        .writer_with_num_threads(1, 15_000_000)
        .map_err(|e| e.to_string())?;
    for (i, d) in docs.iter().enumerate() {
        let id = resolve_id(d, i);
        writer
            .add_document(doc!(
                fields.id => id,
                fields.idx => i as i64,
                fields.text => d.text.clone(),
            ))
            .map_err(|e| e.to_string())?;
    }
    writer.commit().map_err(|e| e.to_string())?;
    Ok((index, fields))
}

/// Look up the `DocAddress` of the document whose stored `id` field exactly
/// equals `target_id`, via an exact TermQuery (robust regardless of internal
/// doc-id numbering). Returns `None` when no document has that id.
pub fn find_by_id(
    searcher: &tantivy::Searcher,
    id_field: Field,
    target_id: &str,
) -> Result<Option<tantivy::DocAddress>, String> {
    let term = Term::from_field_text(id_field, target_id);
    let query = TermQuery::new(term, IndexRecordOption::Basic);
    let found = searcher
        .search(&query, &TopDocs::with_limit(1).order_by_score())
        .map_err(|e| e.to_string())?;
    Ok(found.into_iter().next().map(|(_score, addr)| addr))
}

/// Read a stored string field off a retrieved document; empty string if absent.
pub fn get_stored_str(doc: &TantivyDocument, field: Field) -> String {
    doc.get_first(field).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// Read a stored i64 field off a retrieved document; `-1` if absent.
pub fn get_stored_i64(doc: &TantivyDocument, field: Field) -> i32 {
    doc.get_first(field).and_then(|v| v.as_i64()).unwrap_or(-1) as i32
}

/// Every literal term text appearing anywhere in the query tree (regardless
/// of MUST/SHOULD/MUST_NOT — a MUST_NOT term can never appear in a matched
/// document's own text, so including it here cannot mis-highlight a hit).
pub fn collect_all_terms(query: &dyn Query) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut visitor = |term: &Term, _needs_position: bool| {
        if let Some(s) = term.value().as_str() {
            out.insert(s.to_string());
        }
    };
    query.query_terms(&mut visitor);
    out
}

/// One term extracted from a parsed query, with its resolved MUST/SHOULD/
/// MUST_NOT occurrence.
pub struct TermInfo {
    pub field: String,
    pub term: String,
    pub occur: &'static str,
}

fn occur_str(o: Occur) -> &'static str {
    match o {
        Occur::Must => "MUST",
        Occur::Should => "SHOULD",
        Occur::MustNot => "MUST_NOT",
    }
}

/// Recursively decompose a parsed query into its individual terms with
/// occurrence, honoring tantivy's own `Occur::compose` semantics for nested
/// boolean queries (e.g. a MustNot wrapping a nested Boolean).
pub fn walk_query_terms(query: &dyn Query, occur: Occur, schema: &Schema, out: &mut Vec<TermInfo>) {
    if let Some(bq) = query.downcast_ref::<BooleanQuery>() {
        for (child_occur, sub) in bq.clauses() {
            walk_query_terms(sub.as_ref(), Occur::compose(occur, *child_occur), schema, out);
        }
        return;
    }
    let mut visitor = |term: &Term, _needs_position: bool| {
        if let Some(text) = term.value().as_str() {
            let field_name = schema.get_field_name(term.field()).to_string();
            out.push(TermInfo { field: field_name, term: text.to_string(), occur: occur_str(occur) });
        }
    };
    query.query_terms(&mut visitor);
}

#[cfg(test)]
mod ftsutil_tests {
    use super::*;

    #[test]
    fn test_parse_with_timeout_bounds_a_runaway_closure() {
        // Proves the timeout mechanism itself works, independent of any real
        // tantivy query shape: a closure that sleeps far longer than
        // QUERY_PARSE_TIMEOUT must still return promptly with a structured
        // error, not block the caller.
        let start = std::time::Instant::now();
        let result: Result<i32, &'static str> =
            parse_with_timeout(|| { thread::sleep(Duration::from_secs(10)); 42 });
        let elapsed = start.elapsed();
        assert_eq!(result, Err("QUERY_TOO_COMPLEX"));
        assert!(
            elapsed < Duration::from_secs(3),
            "parse_with_timeout must return near its deadline, not wait for the closure; took {elapsed:?}"
        );
    }

    #[test]
    fn test_parse_with_timeout_passes_through_a_fast_result() {
        let result = parse_with_timeout(|| 7);
        assert_eq!(result, Ok(7));
    }

    #[test]
    fn test_query_nesting_depth_cap_rejects_the_known_exponential_shape_fast() {
        // Regression for the finding this module exists to close: bare nested
        // parens with no explicit operator between clauses is the shape that
        // was empirically confirmed exponential in tantivy 0.26.1's parser.
        // At MAX_QUERY_NESTING_DEPTH + 1 it must be rejected by the cheap
        // depth check, never reaching the parser at all.
        let deep = "(".repeat(MAX_QUERY_NESTING_DEPTH + 1) + "term" + &")".repeat(MAX_QUERY_NESTING_DEPTH + 1);
        let start = std::time::Instant::now();
        assert_eq!(validate_query(&deep), Err("QUERY_TOO_DEEPLY_NESTED"));
        assert!(start.elapsed() < Duration::from_millis(50), "the depth check itself must be O(n), not exponential");
    }
}

/// Byte ranges (into `text`, using the same analyzer that indexed it) of
/// every token whose analyzed text is in `terms`. Ascending order (tokens are
/// visited left to right).
pub fn compute_highlights(text: &str, terms: &HashSet<String>, analyzer_name: &str) -> Vec<(i32, i32)> {
    if terms.is_empty() {
        return Vec::new();
    }
    tokenize_text(text, analyzer_name)
        .into_iter()
        .filter(|t| terms.contains(&t.text))
        .map(|t| (t.offset_from as i32, t.offset_to as i32))
        .collect()
}

/// A char-boundary-safe excerpt of `text` of at most `max_chars` characters,
/// windowed around the first highlight (or the start of the text when there
/// is none). Plain substring — no inline markup; `highlights` carries the
/// match positions relative to the ORIGINAL text, not this excerpt.
pub fn build_snippet(text: &str, highlights: &[(i32, i32)], max_chars: usize) -> String {
    if max_chars == 0 || text.is_empty() {
        return String::new();
    }
    let char_offsets: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    let total_chars = char_offsets.len();
    if total_chars <= max_chars {
        return text.to_string();
    }
    let anchor_char_idx = match highlights.first() {
        Some((start_byte, _)) => char_offsets
            .iter()
            .position(|b| *b as i32 >= *start_byte)
            .unwrap_or(0),
        None => 0,
    };
    let lead = max_chars / 4;
    let mut start_idx = anchor_char_idx.saturating_sub(lead);
    let mut end_idx = (start_idx + max_chars).min(total_chars);
    if end_idx - start_idx < max_chars {
        start_idx = end_idx.saturating_sub(max_chars);
    }
    if end_idx == start_idx {
        end_idx = (start_idx + max_chars).min(total_chars);
    }
    let start_byte = char_offsets[start_idx];
    let end_byte = if end_idx < total_chars { char_offsets[end_idx] } else { text.len() };
    text[start_byte..end_byte].to_string()
}
