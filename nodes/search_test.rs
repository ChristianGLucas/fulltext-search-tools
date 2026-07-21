// Separate test file: nodes/search_test.rs. The generated service wires
// it into the crate via `#[cfg(test)] #[path="nodes/search_test.rs"] mod
// search_test;`. It reaches the node + SDK through `crate::` paths (this is
// a sibling module of the node, not a child — so `super::*` would not resolve).
#[cfg(test)]
mod tests {
    use crate::axiom_context::*;
    use crate::gen::messages::{Document, SearchRequest};
    use crate::search::search;
    use std::collections::HashMap;

    struct TestLogger;
    impl AxiomLogger for TestLogger {
        fn debug(&self, _m: &str, _a: &HashMap<&str, String>) {}
        fn info(&self, _m: &str, _a: &HashMap<&str, String>) {}
        fn warn(&self, _m: &str, _a: &HashMap<&str, String>) {}
        fn error(&self, _m: &str, _a: &HashMap<&str, String>) {}
    }
    struct TestSecrets;
    impl AxiomSecrets for TestSecrets {
        fn get(&self, _n: &str) -> (String, bool) { (String::new(), false) }
    }
    struct EmptyFlow { pos: FlowPosition }
    impl FlowReflection for EmptyFlow {
        fn nodes(&self) -> &[ReflectionNode] { &[] }
        fn edges(&self) -> &[ReflectionEdge] { &[] }
        fn loop_edges(&self) -> &[ReflectionEdge] { &[] }
        fn position(&self) -> &FlowPosition { &self.pos }
        fn graph_id(&self) -> &str { "" }
    }
    struct TestReflection { flow: EmptyFlow }
    impl Reflection for TestReflection { fn flow(&self) -> &dyn FlowReflection { &self.flow } }
    struct TestFlowMut;
    impl FlowMutation for TestFlowMut {
        fn add_node(&self, _p: &str, _v: &str, _c: Option<CanvasPosition>) -> u32 { 0 }
        fn add_edge(&self, _s: u32, _d: u32, _c: Option<EdgeCondition>) {}
    }
    struct TestMutation { flow: TestFlowMut }
    impl Mutation for TestMutation { fn flow(&self) -> &dyn FlowMutation { &self.flow } }

    struct TestContext {
        log: TestLogger, secrets: TestSecrets,
        reflection: TestReflection, mutation: TestMutation,
    }
    impl AxiomContext for TestContext {
        fn log(&self) -> &dyn AxiomLogger { &self.log }
        fn secrets(&self) -> &dyn AxiomSecrets { &self.secrets }
        fn execution_id(&self) -> &str { "test-execution-id" }
        fn flow_id(&self) -> &str { "test-flow-id" }
        fn tenant_id(&self) -> &str { "test-tenant-id" }
        fn reflection(&self) -> &dyn Reflection { &self.reflection }
        fn mutation(&self) -> &dyn Mutation { &self.mutation }
    }
    fn test_context() -> TestContext {
        TestContext {
            log: TestLogger, secrets: TestSecrets,
            reflection: TestReflection { flow: EmptyFlow { pos: FlowPosition::default() } },
            mutation: TestMutation { flow: TestFlowMut },
        }
    }

    fn doc(id: &str, text: &str) -> Document {
        Document { id: id.to_string(), text: text.to_string() }
    }

    /// The textbook Robertson/Sparck-Jones BM25 formula (as published, e.g. in
    /// the Lucene/Elasticsearch docs and Robertson & Zaragoza 2009), computed
    /// completely independently of tantivy's own scorer/Explanation code path.
    /// k1=1.2, b=0.75 match tantivy's fixed defaults.
    fn bm25_term_score(n_total: f64, n_containing: f64, freq: f64, dl: f64, avgdl: f64) -> f64 {
        let k1 = 1.2_f64;
        let b = 0.75_f64;
        let idf = (1.0 + (n_total - n_containing + 0.5) / (n_containing + 0.5)).ln();
        let denom = freq + k1 * (1.0 - b + b * dl / avgdl);
        idf * (freq * (k1 + 1.0)) / denom
    }

    #[test]
    fn test_search_bm25_ranking_matches_hand_computed_formula() {
        // N=3 documents; "alpha" appears in d0 (freq=1, dl=3) and d1 (freq=2, dl=3);
        // avgdl = (3+3+4)/3 = 10/3. d2 does not contain "alpha" at all.
        let ax = test_context();
        let input = SearchRequest {
            documents: vec![
                doc("d0", "alpha beta gamma"),
                doc("d1", "alpha alpha delta"),
                doc("d2", "epsilon zeta eta theta"),
            ],
            query: "alpha".to_string(),
            analyzer: "default".to_string(),
            limit: 10,
            snippet_max_chars: 0,
        };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.error, "");
        assert_eq!(result.total_matches, 2, "only d0 and d1 contain alpha");
        assert_eq!(result.hits.len(), 2);

        let avgdl = 10.0 / 3.0;
        let expected_d1 = bm25_term_score(3.0, 2.0, 2.0, 3.0, avgdl); // freq=2
        let expected_d0 = bm25_term_score(3.0, 2.0, 1.0, 3.0, avgdl); // freq=1

        // d1 (freq=2) must outrank d0 (freq=1): higher term frequency, same length.
        assert_eq!(result.hits[0].id, "d1");
        assert_eq!(result.hits[1].id, "d0");
        assert!(
            (result.hits[0].score - expected_d1).abs() < 1e-3,
            "d1 score {} != hand-computed BM25 {}",
            result.hits[0].score,
            expected_d1
        );
        assert!(
            (result.hits[1].score - expected_d0).abs() < 1e-3,
            "d0 score {} != hand-computed BM25 {}",
            result.hits[1].score,
            expected_d0
        );
    }

    #[test]
    fn test_search_highlights_are_exact_byte_offsets() {
        let ax = test_context();
        let input = SearchRequest {
            documents: vec![doc("d0", "the quick brown fox jumps over the lazy dog")],
            query: "quick fox".to_string(),
            analyzer: "default".to_string(),
            limit: 10,
            snippet_max_chars: 0,
        };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.hits.len(), 1);
        let hit = &result.hits[0];
        // "the "=0..4, "quick"=4..9, "brown "=9..15, "fox"=16..19 (space at 15).
        assert_eq!(hit.highlights.len(), 2);
        assert_eq!((hit.highlights[0].start, hit.highlights[0].end), (4, 9));
        assert_eq!((hit.highlights[1].start, hit.highlights[1].end), (16, 19));
        assert_eq!(&input_text()[4..9], "quick");
        assert_eq!(&input_text()[16..19], "fox");
    }

    fn input_text() -> &'static str {
        "the quick brown fox jumps over the lazy dog"
    }

    #[test]
    fn test_search_snippet_window_is_char_boundary_safe_and_bounded() {
        let ax = test_context();
        let input = SearchRequest {
            documents: vec![doc(
                "d0",
                "a quick brown fox jumps over the lazy dog near the riverbank at dawn",
            )],
            query: "riverbank".to_string(),
            analyzer: "default".to_string(),
            limit: 10,
            snippet_max_chars: 20,
        };
        let result = search(&ax, input).unwrap();
        let hit = &result.hits[0];
        assert!(hit.snippet.chars().count() <= 20);
        assert!(hit.snippet.contains("riverbank"));
    }

    #[test]
    fn test_search_is_deterministic() {
        let ax = test_context();
        let make_input = || SearchRequest {
            documents: vec![doc("d0", "alpha beta"), doc("d1", "beta gamma")],
            query: "beta".to_string(),
            analyzer: "default".to_string(),
            limit: 10,
            snippet_max_chars: 20,
        };
        let r1 = search(&ax, make_input()).unwrap();
        let r2 = search(&ax, make_input()).unwrap();
        assert_eq!(r1.hits.len(), r2.hits.len());
        for (a, b) in r1.hits.iter().zip(r2.hits.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.score, b.score);
            assert_eq!(a.snippet, b.snippet);
        }
    }

    #[test]
    fn test_search_id_defaults_to_input_index_when_empty() {
        let ax = test_context();
        let input = SearchRequest {
            documents: vec![doc("", "alpha"), doc("", "beta")],
            query: "beta".to_string(),
            ..Default::default()
        };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].id, "1");
        assert_eq!(result.hits[0].doc_index, 1);
    }

    #[test]
    fn test_search_stemming_changes_match_set() {
        let ax = test_context();
        let base = |analyzer: &str| SearchRequest {
            documents: vec![doc("a", "I love to run every morning")],
            query: "running".to_string(),
            analyzer: analyzer.to_string(),
            limit: 10,
            snippet_max_chars: 0,
        };
        let stemmed = search(&ax, base("en_stem")).unwrap();
        assert_eq!(stemmed.hits.len(), 1, "en_stem should fold running/run to the same stem");

        let unstemmed = search(&ax, base("default")).unwrap();
        assert_eq!(unstemmed.hits.len(), 0, "default analyzer must not stem, so no match");
    }

    #[test]
    fn test_search_empty_documents_is_structured_error() {
        let ax = test_context();
        let input = SearchRequest { documents: vec![], query: "x".to_string(), ..Default::default() };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.error, "EMPTY_DOCUMENTS");
        assert!(result.hits.is_empty());
    }

    #[test]
    fn test_search_empty_query_is_structured_error() {
        let ax = test_context();
        let input =
            SearchRequest { documents: vec![doc("a", "hi")], query: "".to_string(), ..Default::default() };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.error, "EMPTY_QUERY");
    }

    #[test]
    fn test_search_too_many_documents_is_structured_error() {
        let ax = test_context();
        let documents: Vec<Document> = (0..1001).map(|i| doc(&i.to_string(), "x")).collect();
        let input = SearchRequest { documents, query: "x".to_string(), ..Default::default() };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.error, "TOO_MANY_DOCUMENTS");
    }

    #[test]
    fn test_search_document_too_large_is_structured_error() {
        let ax = test_context();
        let huge = "x".repeat(1_048_577);
        let input =
            SearchRequest { documents: vec![doc("a", &huge)], query: "x".to_string(), ..Default::default() };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.error, "DOCUMENT_TOO_LARGE");
    }

    #[test]
    fn test_search_invalid_analyzer_is_structured_error_not_silent_fallback() {
        let ax = test_context();
        let input = SearchRequest {
            documents: vec![doc("a", "hi")],
            query: "hi".to_string(),
            analyzer: "bogus".to_string(),
            ..Default::default()
        };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.error, "INVALID_ANALYZER");
    }

    #[test]
    fn test_search_malformed_query_returns_structured_error_not_panic() {
        let ax = test_context();
        let input = SearchRequest {
            documents: vec![doc("a", "hi")],
            query: "field:[1 TO".to_string(),
            ..Default::default()
        };
        let result = search(&ax, input).unwrap();
        assert!(result.error.starts_with("QUERY_PARSE_ERROR"));
    }

    #[test]
    fn test_search_limit_defaults_and_caps() {
        let ax = test_context();
        let documents: Vec<Document> = (0..5).map(|i| doc(&i.to_string(), "match term")).collect();
        let input =
            SearchRequest { documents, query: "match".to_string(), limit: -5, ..Default::default() };
        let result = search(&ax, input).unwrap();
        assert_eq!(result.hits.len(), 5, "negative limit should default to 10, not error");
    }
}
