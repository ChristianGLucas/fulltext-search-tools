use crate::axiom_context::AxiomContext;
use crate::gen::messages::{Range, SnippetRequest, SnippetResponse};

#[path = "ftsutil.rs"]
mod ftsutil;

use tantivy::query::QueryParser;
use tantivy::tokenizer::TokenizerManager;

/// Generate a highlighted excerpt of `text` for `query`, with byte-offset
/// match ranges into the ORIGINAL text — for re-snippeting text a caller
/// already has, without building a document index or running a search.
pub fn snippet(
    ax: &dyn AxiomContext,
    input: SnippetRequest,
) -> Result<SnippetResponse, Box<dyn std::error::Error>> {
    let _ = ax;

    if input.text.is_empty() {
        return Ok(SnippetResponse { error: "EMPTY_TEXT".to_string(), ..Default::default() });
    }
    if input.text.len() > ftsutil::MAX_TEXT_BYTES {
        return Ok(SnippetResponse { error: "TEXT_TOO_LARGE".to_string(), ..Default::default() });
    }
    if let Err(code) = ftsutil::validate_query(&input.query) {
        return Ok(SnippetResponse { error: code.to_string(), ..Default::default() });
    }
    let analyzer_name = match ftsutil::normalize_analyzer(&input.analyzer) {
        Ok(n) => n,
        Err(code) => return Ok(SnippetResponse { error: code.to_string(), ..Default::default() }),
    };

    let (schema, text_field) = ftsutil::build_text_only_schema(&analyzer_name);
    let query_parser = QueryParser::new(schema, vec![text_field], TokenizerManager::default());
    let query = match query_parser.parse_query(&input.query) {
        Ok(q) => q,
        Err(e) => {
            return Ok(SnippetResponse { error: format!("QUERY_PARSE_ERROR: {e}"), ..Default::default() })
        }
    };

    let terms = ftsutil::collect_all_terms(query.as_ref());
    let highlights = ftsutil::compute_highlights(&input.text, &terms, &analyzer_name);
    let max_chars =
        if input.max_chars <= 0 { ftsutil::DEFAULT_SNIPPET_CHARS } else { input.max_chars as usize };
    let snippet_text = ftsutil::build_snippet(&input.text, &highlights, max_chars);
    let highlight_ranges = highlights.into_iter().map(|(start, end)| Range { start, end }).collect();

    Ok(SnippetResponse { snippet: snippet_text, highlights: highlight_ranges, error: String::new() })
}
