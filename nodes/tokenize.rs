use crate::axiom_context::AxiomContext;
use crate::gen::messages::{Token as TokenMsg, TokenizeRequest, TokenizeResponse};

#[path = "ftsutil.rs"]
mod ftsutil;

/// Run tantivy's tokenizer/analyzer pipeline over raw text and return the
/// resulting token stream — no index is built, no search happens. Useful for
/// inspecting how an analyzer will transform text before indexing it, or for
/// building custom highlighting/analysis outside of Search.
pub fn tokenize(
    ax: &dyn AxiomContext,
    input: TokenizeRequest,
) -> Result<TokenizeResponse, Box<dyn std::error::Error>> {
    let _ = ax;

    if input.text.is_empty() {
        return Ok(TokenizeResponse { tokens: vec![], error: "EMPTY_TEXT".to_string() });
    }
    if input.text.len() > ftsutil::MAX_TEXT_BYTES {
        return Ok(TokenizeResponse { tokens: vec![], error: "TEXT_TOO_LARGE".to_string() });
    }
    let analyzer_name = match ftsutil::normalize_analyzer(&input.analyzer) {
        Ok(n) => n,
        Err(code) => return Ok(TokenizeResponse { tokens: vec![], error: code.to_string() }),
    };

    let tokens = ftsutil::tokenize_text(&input.text, &analyzer_name)
        .into_iter()
        .map(|t| TokenMsg {
            text: t.text,
            position: t.position as i32,
            start: t.offset_from as i32,
            end: t.offset_to as i32,
        })
        .collect();

    Ok(TokenizeResponse { tokens, error: String::new() })
}
