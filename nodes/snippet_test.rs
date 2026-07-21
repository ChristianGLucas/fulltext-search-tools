// Separate test file: nodes/snippet_test.rs. The generated service wires
// it into the crate via `#[cfg(test)] #[path="nodes/snippet_test.rs"] mod
// snippet_test;`. It reaches the node + SDK through `crate::` paths (this is
// a sibling module of the node, not a child — so `super::*` would not resolve).
#[cfg(test)]
mod tests {
    use crate::axiom_context::*;
    use crate::gen::messages::SnippetRequest;
    use crate::snippet::snippet;
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

    const TEXT: &str = "a quick brown fox jumps over the lazy dog near the riverbank at dawn";

    #[test]
    fn test_snippet_highlight_offset_matches_literal_substring_search() {
        // Independent oracle: `TEXT.find("riverbank")` is a plain Rust stdlib
        // substring search, entirely independent of tantivy's tokenizer.
        let ax = test_context();
        let input = SnippetRequest {
            text: TEXT.to_string(),
            query: "riverbank".to_string(),
            analyzer: String::new(),
            max_chars: 20,
        };
        let result = snippet(&ax, input).unwrap();
        assert_eq!(result.error, "");
        assert_eq!(result.highlights.len(), 1);
        let expected_start = TEXT.find("riverbank").unwrap() as i32;
        assert_eq!(result.highlights[0].start, expected_start);
        assert_eq!(result.highlights[0].end, expected_start + "riverbank".len() as i32);
        // The highlighted range, read back out of the ORIGINAL text, is the word itself.
        let (s, e) = (result.highlights[0].start as usize, result.highlights[0].end as usize);
        assert_eq!(&TEXT[s..e], "riverbank");
    }

    #[test]
    fn test_snippet_is_bounded_by_max_chars_and_contains_match() {
        let ax = test_context();
        let input = SnippetRequest {
            text: TEXT.to_string(),
            query: "riverbank".to_string(),
            analyzer: String::new(),
            max_chars: 20,
        };
        let result = snippet(&ax, input).unwrap();
        assert!(result.snippet.chars().count() <= 20);
        assert!(result.snippet.contains("riverbank"));
    }

    #[test]
    fn test_snippet_max_chars_default_when_unset() {
        let ax = test_context();
        let input = SnippetRequest {
            text: TEXT.to_string(),
            query: "quick".to_string(),
            analyzer: String::new(),
            max_chars: 0,
        };
        let result = snippet(&ax, input).unwrap();
        // Default is 200 chars, comfortably larger than the whole fixture text.
        assert_eq!(result.snippet, TEXT);
    }

    #[test]
    fn test_snippet_no_match_yields_empty_highlights_but_still_a_snippet() {
        let ax = test_context();
        let input = SnippetRequest {
            text: TEXT.to_string(),
            query: "nonexistentterm".to_string(),
            analyzer: String::new(),
            max_chars: 20,
        };
        let result = snippet(&ax, input).unwrap();
        assert!(result.highlights.is_empty());
        assert_eq!(result.snippet.chars().count(), 20);
    }

    #[test]
    fn test_snippet_empty_text_is_structured_error() {
        let ax = test_context();
        let input =
            SnippetRequest { text: String::new(), query: "x".to_string(), ..Default::default() };
        let result = snippet(&ax, input).unwrap();
        assert_eq!(result.error, "EMPTY_TEXT");
    }

    #[test]
    fn test_snippet_empty_query_is_structured_error() {
        let ax = test_context();
        let input =
            SnippetRequest { text: "hi there".to_string(), query: String::new(), ..Default::default() };
        let result = snippet(&ax, input).unwrap();
        assert_eq!(result.error, "EMPTY_QUERY");
    }

    #[test]
    fn test_snippet_malformed_query_returns_structured_error_not_panic() {
        let ax = test_context();
        let input = SnippetRequest {
            text: "hi there".to_string(),
            query: "field:[1 TO".to_string(),
            ..Default::default()
        };
        let result = snippet(&ax, input).unwrap();
        assert!(result.error.starts_with("QUERY_PARSE_ERROR"));
    }
}
