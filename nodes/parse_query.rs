use crate::axiom_context::AxiomContext;
use crate::gen::messages::{ParseQueryRequest, ParseQueryResponse, QueryTerm};

#[path = "ftsutil.rs"]
mod ftsutil;

use tantivy::query::{Occur, QueryParser};
use tantivy::tokenizer::TokenizerManager;

/// Validate a tantivy query string and decompose it into its individual
/// terms (field, analyzed term text, and MUST/SHOULD/MUST_NOT occurrence),
/// plus a debug rendering of the parsed query tree — without building an
/// index or running a search. Useful for validating a user-typed query
/// before spending an index build on it.
pub fn parse_query(
    ax: &dyn AxiomContext,
    input: ParseQueryRequest,
) -> Result<ParseQueryResponse, Box<dyn std::error::Error>> {
    let _ = ax;

    if let Err(code) = ftsutil::validate_query(&input.query) {
        return Ok(ParseQueryResponse { valid: false, error: code.to_string(), ..Default::default() });
    }
    let analyzer_name = match ftsutil::normalize_analyzer(&input.analyzer) {
        Ok(n) => n,
        Err(code) => {
            return Ok(ParseQueryResponse { valid: false, error: code.to_string(), ..Default::default() })
        }
    };

    let (schema, text_field) = ftsutil::build_text_only_schema(&analyzer_name);
    let query_parser = QueryParser::new(schema.clone(), vec![text_field], TokenizerManager::default());
    let query_string = input.query.clone();
    let parsed = match ftsutil::parse_with_timeout(move || query_parser.parse_query(&query_string)) {
        Ok(inner) => inner,
        Err(code) => {
            return Ok(ParseQueryResponse { valid: false, error: code.to_string(), ..Default::default() })
        }
    };

    match parsed {
        Ok(query) => {
            let debug_representation = format!("{query:?}");
            let mut terms = Vec::new();
            ftsutil::walk_query_terms(query.as_ref(), Occur::Should, &schema, &mut terms);
            let terms = terms
                .into_iter()
                .map(|t| QueryTerm { field: t.field, term: t.term, occur: t.occur.to_string() })
                .collect();
            Ok(ParseQueryResponse { valid: true, terms, debug_representation, error: String::new() })
        }
        Err(e) => Ok(ParseQueryResponse {
            valid: false,
            terms: vec![],
            debug_representation: String::new(),
            error: e.to_string(),
        }),
    }
}
