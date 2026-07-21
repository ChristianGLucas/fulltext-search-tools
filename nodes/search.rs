use crate::axiom_context::AxiomContext;
use crate::gen::messages::{Hit, Range, SearchRequest, SearchResponse};

#[path = "ftsutil.rs"]
mod ftsutil;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::TantivyDocument;

/// Build an ephemeral, purely in-memory tantivy index over `documents`, run
/// `query` against it with BM25 scoring, and return the ranked hits (with a
/// highlighted snippet and exact match byte-offsets per hit) before
/// discarding the index. Deterministic for a fixed analyzer: single-threaded
/// indexing, one commit, standard BM25 (k1=1.2, b=0.75). Bare query terms are
/// combined with OR by default; use `+term`/`-term`/`"phrase"` for
/// require/exclude/phrase semantics.
pub fn search(
    ax: &dyn AxiomContext,
    input: SearchRequest,
) -> Result<SearchResponse, Box<dyn std::error::Error>> {
    let _ = ax;

    if let Err(code) = ftsutil::validate_documents(&input.documents) {
        return Ok(SearchResponse { error: code.to_string(), ..Default::default() });
    }
    if let Err(code) = ftsutil::validate_query(&input.query) {
        return Ok(SearchResponse { error: code.to_string(), ..Default::default() });
    }
    let analyzer_name = match ftsutil::normalize_analyzer(&input.analyzer) {
        Ok(n) => n,
        Err(code) => return Ok(SearchResponse { error: code.to_string(), ..Default::default() }),
    };

    let (index, fields) = match ftsutil::build_index(&input.documents, &analyzer_name) {
        Ok(v) => v,
        Err(e) => return Ok(SearchResponse { error: format!("INDEX_ERROR: {e}"), ..Default::default() }),
    };

    let reader = index.reader()?;
    let searcher = reader.searcher();
    let query_parser = QueryParser::for_index(&index, vec![fields.text]);
    let query_string = input.query.clone();
    let query = match ftsutil::parse_with_timeout(move || query_parser.parse_query(&query_string)) {
        Ok(Ok(q)) => q,
        Ok(Err(e)) => {
            return Ok(SearchResponse { error: format!("QUERY_PARSE_ERROR: {e}"), ..Default::default() })
        }
        Err(code) => return Ok(SearchResponse { error: code.to_string(), ..Default::default() }),
    };

    let limit = if input.limit <= 0 {
        ftsutil::DEFAULT_LIMIT
    } else {
        input.limit.min(ftsutil::MAX_LIMIT)
    } as usize;

    let top_docs = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;
    let total_matches = query.count(&searcher)? as i32;
    let all_terms = ftsutil::collect_all_terms(query.as_ref());

    let mut hits = Vec::with_capacity(top_docs.len());
    for (score, doc_address) in top_docs {
        let retrieved: TantivyDocument = searcher.doc(doc_address)?;
        let id = ftsutil::get_stored_str(&retrieved, fields.id);
        let doc_index = ftsutil::get_stored_i64(&retrieved, fields.idx);
        let text = ftsutil::get_stored_str(&retrieved, fields.text);

        let highlights = ftsutil::compute_highlights(&text, &all_terms, &analyzer_name);
        let snippet = if input.snippet_max_chars > 0 {
            ftsutil::build_snippet(&text, &highlights, input.snippet_max_chars as usize)
        } else {
            String::new()
        };
        let highlight_ranges =
            highlights.into_iter().map(|(start, end)| Range { start, end }).collect();

        hits.push(Hit {
            id,
            doc_index,
            score: score as f64,
            snippet,
            highlights: highlight_ranges,
        });
    }

    Ok(SearchResponse { hits, total_matches, error: String::new() })
}
