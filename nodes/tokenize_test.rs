// Separate test file: nodes/tokenize_test.rs. The generated service wires
// it into the crate via `#[cfg(test)] #[path="nodes/tokenize_test.rs"] mod
// tokenize_test;`. It reaches the node + SDK through `crate::` paths (this is
// a sibling module of the node, not a child — so `super::*` would not resolve).
#[cfg(test)]
mod tests {
    use crate::axiom_context::*;
    use crate::gen::messages::TokenizeRequest;
    use crate::tokenize::tokenize;
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
    fn test_tokenize_default_splits_lowercases_and_tracks_offsets() {
        // Independent oracle: "default" is documented as whitespace/punctuation
        // splitting + lowercasing — a hand-countable transformation of the
        // literal input string, not tied to tantivy internals.
        let ax = test_context();
        let input = TokenizeRequest { text: "The Quick-Brown Fox!".to_string(), analyzer: String::new() };
        let result = tokenize(&ax, input).unwrap();
        assert_eq!(result.error, "");
        let texts: Vec<&str> = result.tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["the", "quick", "brown", "fox"]);
        assert_eq!(result.tokens[0].position, 0);
        assert_eq!(result.tokens[1].position, 1);
        // "The " = 0..3 ("The"), "Quick" starts at byte 4.
        assert_eq!((result.tokens[0].start, result.tokens[0].end), (0, 3));
        assert_eq!((result.tokens[1].start, result.tokens[1].end), (4, 9));
    }

    #[test]
    fn test_tokenize_en_stem_applies_porter_stemming() {
        // Independent oracle: the English Porter/Snowball stemmer is a
        // published, deterministic algorithm — "running"->"run" and
        // "runners"->"runner" are its well-known documented outputs,
        // independent of tantivy's implementation of it.
        let ax = test_context();
        let input =
            TokenizeRequest { text: "running runners".to_string(), analyzer: "en_stem".to_string() };
        let result = tokenize(&ax, input).unwrap();
        let texts: Vec<&str> = result.tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["run", "runner"]);
    }

    #[test]
    fn test_tokenize_whitespace_does_not_lowercase_or_split_punctuation() {
        let ax = test_context();
        let input =
            TokenizeRequest { text: "Hello,World  Foo".to_string(), analyzer: "whitespace".to_string() };
        let result = tokenize(&ax, input).unwrap();
        let texts: Vec<&str> = result.tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["Hello,World", "Foo"], "whitespace tokenizer keeps case and punctuation");
    }

    #[test]
    fn test_tokenize_raw_yields_a_single_unmodified_token() {
        let ax = test_context();
        let input = TokenizeRequest { text: "Mixed CASE text!".to_string(), analyzer: "raw".to_string() };
        let result = tokenize(&ax, input).unwrap();
        assert_eq!(result.tokens.len(), 1);
        assert_eq!(result.tokens[0].text, "Mixed CASE text!");
        assert_eq!((result.tokens[0].start, result.tokens[0].end), (0, 16));
    }

    #[test]
    fn test_tokenize_empty_text_is_structured_error() {
        let ax = test_context();
        let result = tokenize(&ax, TokenizeRequest::default()).unwrap();
        assert_eq!(result.error, "EMPTY_TEXT");
        assert!(result.tokens.is_empty());
    }

    #[test]
    fn test_tokenize_invalid_analyzer_is_structured_error() {
        let ax = test_context();
        let input = TokenizeRequest { text: "hi".to_string(), analyzer: "bogus".to_string() };
        let result = tokenize(&ax, input).unwrap();
        assert_eq!(result.error, "INVALID_ANALYZER");
    }
}
