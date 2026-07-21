use crate::axiom_context::AxiomContext;
use crate::gen::messages::{ExplainRequest, ExplainResponse};

#[path = "ftsutil.rs"]
mod ftsutil;

use tantivy::query::QueryParser;
use tantivy::TantivyDocument;

/// Build an ephemeral index over `documents` and return the BM25 score
/// breakdown for one target document against a query — why it would rank
/// where it does in a Search call over the same document set. The BM25
/// statistics (IDF, average field length) the explanation is computed
/// against depend on the whole document set, exactly as in Search.
pub fn explain(
    ax: &dyn AxiomContext,
    input: ExplainRequest,
) -> Result<ExplainResponse, Box<dyn std::error::Error>> {
    let _ = ax;

    if let Err(code) = ftsutil::validate_documents(&input.documents) {
        return Ok(ExplainResponse { error: code.to_string(), ..Default::default() });
    }
    if let Err(code) = ftsutil::validate_query(&input.query) {
        return Ok(ExplainResponse { error: code.to_string(), ..Default::default() });
    }
    if input.target_id.is_empty() {
        return Ok(ExplainResponse { error: "TARGET_NOT_FOUND".to_string(), ..Default::default() });
    }
    let analyzer_name = match ftsutil::normalize_analyzer(&input.analyzer) {
        Ok(n) => n,
        Err(code) => return Ok(ExplainResponse { error: code.to_string(), ..Default::default() }),
    };

    let (index, fields) = match ftsutil::build_index(&input.documents, &analyzer_name) {
        Ok(v) => v,
        Err(e) => return Ok(ExplainResponse { error: format!("INDEX_ERROR: {e}"), ..Default::default() }),
    };

    let reader = index.reader()?;
    let searcher = reader.searcher();
    let query_parser = QueryParser::for_index(&index, vec![fields.text]);
    let query = match query_parser.parse_query(&input.query) {
        Ok(q) => q,
        Err(e) => {
            return Ok(ExplainResponse { error: format!("QUERY_PARSE_ERROR: {e}"), ..Default::default() })
        }
    };

    let doc_address = match ftsutil::find_by_id(&searcher, fields.id, &input.target_id) {
        Ok(Some(addr)) => addr,
        Ok(None) => return Ok(ExplainResponse { error: "TARGET_NOT_FOUND".to_string(), ..Default::default() }),
        Err(e) => return Ok(ExplainResponse { error: format!("INDEX_ERROR: {e}"), ..Default::default() }),
    };
    let _ensure_stored: TantivyDocument = searcher.doc(doc_address)?;

    match query.explain(&searcher, doc_address) {
        Ok(explanation) => Ok(ExplainResponse {
            score: explanation.value() as f64,
            matched: true,
            explanation: explanation.to_pretty_json(),
            error: String::new(),
        }),
        Err(_) => Ok(ExplainResponse {
            score: 0.0,
            matched: false,
            explanation: String::new(),
            error: String::new(),
        }),
    }
}
