// Separate test file: nodes/explain_test.rs. The generated service wires
// it into the crate via `#[cfg(test)] #[path="nodes/explain_test.rs"] mod
// explain_test;`. It reaches the node + SDK through `crate::` paths (this is
// a sibling module of the node, not a child — so `super::*` would not resolve).
#[cfg(test)]
mod tests {
    use crate::axiom_context::*;
    use crate::explain::explain;
    use crate::gen::messages::{Document, ExplainRequest};
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

    /// The textbook BM25 formula (Robertson & Zaragoza 2009 / Lucene's own
    /// published similarity docs), computed from scratch here — independent
    /// of tantivy's Explanation code path, which is exactly what this test
    /// checks against.
    fn bm25_score(n_total: f64, n_containing: f64, freq: f64, dl: f64, avgdl: f64) -> f64 {
        let k1 = 1.2_f64;
        let b = 0.75_f64;
        let idf = (1.0 + (n_total - n_containing + 0.5) / (n_containing + 0.5)).ln();
        let denom = freq + k1 * (1.0 - b + b * dl / avgdl);
        idf * (freq * (k1 + 1.0)) / denom
    }

    #[test]
    fn test_explain_score_matches_hand_computed_bm25_two_term_sum() {
        // Same fixture as Search's ranking test: N=3, "quick" in 2 of 3 docs,
        // "fox" in 1 of 3 docs; d0 has dl=9 tokens, avgdl = (9+6+5)/3 = 20/3.
        let ax = test_context();
        let documents = vec![
            doc("d0", "the quick brown fox jumps over the lazy dog"), // 9 tokens
            doc("d1", "the lazy dog sleeps all day"),                 // 6 tokens
            doc("d2", "quick foxes are clever animals"),              // 5 tokens
        ];
        let input = ExplainRequest {
            documents,
            query: "quick fox".to_string(),
            analyzer: String::new(),
            target_id: "d0".to_string(),
        };
        let result = explain(&ax, input).unwrap();
        assert_eq!(result.error, "");
        assert!(result.matched);

        let avgdl = 20.0 / 3.0;
        let quick = bm25_score(3.0, 2.0, 1.0, 9.0, avgdl); // "quick" in d0,d2 -> n=2
        let fox = bm25_score(3.0, 1.0, 1.0, 9.0, avgdl); // "fox" only in d0 -> n=1
        let expected = quick + fox;

        assert!(
            (result.score - expected).abs() < 1e-3,
            "explain score {} != hand-computed BM25 sum {}",
            result.score,
            expected
        );
        assert!(result.explanation.contains("idf"));
        assert!(result.explanation.contains("freq"));
    }

    #[test]
    fn test_explain_non_matching_document_reports_unmatched_not_error() {
        let ax = test_context();
        let documents = vec![doc("d0", "hello world"), doc("d1", "goodbye")];
        let input = ExplainRequest {
            documents,
            query: "hello".to_string(),
            analyzer: String::new(),
            target_id: "d1".to_string(),
        };
        let result = explain(&ax, input).unwrap();
        assert_eq!(result.error, "");
        assert!(!result.matched);
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn test_explain_unknown_target_id_is_structured_error() {
        let ax = test_context();
        let input = ExplainRequest {
            documents: vec![doc("d0", "hello world")],
            query: "hello".to_string(),
            analyzer: String::new(),
            target_id: "does-not-exist".to_string(),
        };
        let result = explain(&ax, input).unwrap();
        assert_eq!(result.error, "TARGET_NOT_FOUND");
    }

    #[test]
    fn test_explain_deeply_nested_query_is_rejected_not_a_stack_overflow() {
        let ax = test_context();
        let nested = format!("{}term{}", "(".repeat(200), ")".repeat(200));
        let input = ExplainRequest {
            documents: vec![doc("d0", "hello world")],
            query: nested,
            analyzer: String::new(),
            target_id: "d0".to_string(),
        };
        let result = explain(&ax, input).unwrap();
        assert_eq!(result.error, "QUERY_TOO_DEEPLY_NESTED");
    }

    #[test]
    fn test_explain_empty_documents_is_structured_error() {
        let ax = test_context();
        let input = ExplainRequest {
            documents: vec![],
            query: "hello".to_string(),
            analyzer: String::new(),
            target_id: "d0".to_string(),
        };
        let result = explain(&ax, input).unwrap();
        assert_eq!(result.error, "EMPTY_DOCUMENTS");
    }

    #[test]
    fn test_explain_target_by_default_index_id() {
        // Documents with no id resolve to their 0-based index as a string,
        // same rule as Search's Hit.id.
        let ax = test_context();
        let input = ExplainRequest {
            documents: vec![doc("", "alpha"), doc("", "beta")],
            query: "beta".to_string(),
            analyzer: String::new(),
            target_id: "1".to_string(),
        };
        let result = explain(&ax, input).unwrap();
        assert_eq!(result.error, "");
        assert!(result.matched);
    }
}
