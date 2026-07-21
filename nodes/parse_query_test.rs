// Separate test file: nodes/parse_query_test.rs. The generated service wires
// it into the crate via `#[cfg(test)] #[path="nodes/parse_query_test.rs"] mod
// parse_query_test;`. It reaches the node + SDK through `crate::` paths (this is
// a sibling module of the node, not a child — so `super::*` would not resolve).
#[cfg(test)]
mod tests {
    use crate::axiom_context::*;
    use crate::gen::messages::ParseQueryRequest;
    use crate::parse_query::parse_query;
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

    #[test]
    fn test_parse_query_decomposes_must_should_mustnot() {
        // Independent oracle: tantivy's documented query grammar — a bare term
        // defaults to SHOULD, `+term` to MUST, `-term` to MUST_NOT — is a
        // published spec (mirrored by Lucene's own query syntax), checkable
        // by hand against the literal query string below.
        let ax = test_context();
        let input =
            ParseQueryRequest { query: "+quick -lazy cat".to_string(), analyzer: String::new() };
        let result = parse_query(&ax, input).unwrap();
        assert!(result.valid);
        assert_eq!(result.error, "");
        assert_eq!(result.terms.len(), 3);

        let quick = result.terms.iter().find(|t| t.term == "quick").expect("quick present");
        assert_eq!(quick.occur, "MUST");
        assert_eq!(quick.field, "text");

        let lazy = result.terms.iter().find(|t| t.term == "lazy").expect("lazy present");
        assert_eq!(lazy.occur, "MUST_NOT");

        let cat = result.terms.iter().find(|t| t.term == "cat").expect("cat present");
        assert_eq!(cat.occur, "SHOULD");
    }

    #[test]
    fn test_parse_query_phrase_terms_share_the_clause_occur() {
        let ax = test_context();
        let input = ParseQueryRequest { query: "\"brown fox\"".to_string(), analyzer: String::new() };
        let result = parse_query(&ax, input).unwrap();
        assert!(result.valid);
        assert_eq!(result.terms.len(), 2);
        assert!(result.terms.iter().all(|t| t.occur == "SHOULD"));
        let words: Vec<&str> = result.terms.iter().map(|t| t.term.as_str()).collect();
        assert_eq!(words, vec!["brown", "fox"]);
    }

    #[test]
    fn test_parse_query_analyzer_transforms_term_text() {
        // The query's own terms are analyzed too: en_stem should stem "running".
        let ax = test_context();
        let input = ParseQueryRequest { query: "running".to_string(), analyzer: "en_stem".to_string() };
        let result = parse_query(&ax, input).unwrap();
        assert!(result.valid);
        assert_eq!(result.terms.len(), 1);
        assert_eq!(result.terms[0].term, "run");
    }

    #[test]
    fn test_parse_query_debug_representation_is_nonempty_when_valid() {
        let ax = test_context();
        let input = ParseQueryRequest { query: "cat".to_string(), analyzer: String::new() };
        let result = parse_query(&ax, input).unwrap();
        assert!(result.valid);
        assert!(result.debug_representation.contains("cat"));
    }

    #[test]
    fn test_parse_query_deeply_nested_query_is_rejected_not_a_stack_overflow() {
        let ax = test_context();
        let nested = format!("{}term{}", "(".repeat(200), ")".repeat(200));
        let input = ParseQueryRequest { query: nested, analyzer: String::new() };
        let result = parse_query(&ax, input).unwrap();
        assert!(!result.valid);
        assert_eq!(result.error, "QUERY_TOO_DEEPLY_NESTED");
    }

    #[test]
    fn test_parse_query_malformed_syntax_is_invalid_not_a_crash() {
        let ax = test_context();
        let input = ParseQueryRequest { query: "field:[1 TO".to_string(), analyzer: String::new() };
        let result = parse_query(&ax, input).unwrap();
        assert!(!result.valid);
        assert!(result.terms.is_empty());
        assert!(!result.error.is_empty());
    }

    #[test]
    fn test_parse_query_invalid_analyzer_is_structured_error() {
        let ax = test_context();
        let input = ParseQueryRequest { query: "cat".to_string(), analyzer: "bogus".to_string() };
        let result = parse_query(&ax, input).unwrap();
        assert!(!result.valid);
        assert_eq!(result.error, "INVALID_ANALYZER");
    }
}
