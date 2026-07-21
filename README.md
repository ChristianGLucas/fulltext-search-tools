# fulltext-search-tools

Composable **full-text search & ranking** nodes for the [Axiom](https://axiomide.com)
marketplace, published as `christiangeorgelucas/fulltext-search-tools`. Given a
handful of documents and a query in a single call, build an ephemeral in-memory
search index, rank the matches with BM25, highlight them, and throw the index
away — entirely offline and deterministically.

Written in **Rust**, wrapping one battle-tested, permissively-licensed library:

| Concern | Library | License |
|---|---|---|
| Indexing, BM25 ranking, query parsing, tokenization, snippets | [`tantivy`](https://github.com/quickwit-oss/tantivy) (the engine behind Quickwit; Lucene-equivalent) | MIT |

Every node is **stateless**, **offline** (no network, no API keys, no signup),
and **deterministic** for a fixed analyzer. There is no persistent index —
each call builds its own single-segment, single-threaded, purely in-memory
tantivy index from the documents given, searches it, and discards it before
returning.

## Nodes

| Node | Input → Output | Purpose |
|---|---|---|
| `Search` | `SearchRequest` → `SearchResponse` | BM25-ranked hits with highlighted snippets and exact match byte-offsets |
| `Tokenize` | `TokenizeRequest` → `TokenizeResponse` | Run the analyzer pipeline over text; token stream with stream position + offsets |
| `ParseQuery` | `ParseQueryRequest` → `ParseQueryResponse` | Validate + decompose a query into terms with MUST/SHOULD/MUST_NOT occurrence |
| `Snippet` | `SnippetRequest` → `SnippetResponse` | Highlighted excerpt of a single piece of text for a query, no index needed |
| `Explain` | `ExplainRequest` → `ExplainResponse` | BM25 score breakdown (idf, term frequency, length norm) for one document |

## The `Document` envelope

`Search` and `Explain` take a list of `Document { id, text }` — `id` is
optional (defaults to the document's 0-based input index as a string); `text`
is the body to index. The `text` field name deliberately matches the
convention already used by `ocr-tools`, `pdf-tools`, `html-tools`, and
`nlp-tools`, so their text output maps straight into a `Document.text` with a
trivial one-field adapter. Offset fields (`start`/`end`, byte offsets into the
source text) follow the same convention `nlp-tools` established for its
`Token`/`Entity`/`Sentence` messages.

## Analyzers

`analyzer` selects the tokenizer/filter pipeline applied to both indexed text
and the query (empty means `"default"`):

- **`default`** — Unicode-aware word splitting + lowercasing.
- **`en_stem`** — `default` plus English Porter stemming (`running` → `run`).
- **`whitespace`** — splits on whitespace only; no lowercasing, no punctuation
  splitting.
- **`raw`** — the entire field value is a single token, exact-match,
  case-sensitive. An unquoted multi-word query is split into several OR'd
  single-word terms **by the query grammar itself**, before any per-field
  tokenizer runs — wrap a multi-word `raw` query in double quotes
  (`"Exact Match"`) to match a whole multi-word field value.

An unrecognized `analyzer` value is a structured `INVALID_ANALYZER` error,
never a silent fallback to `default`.

## Query syntax

`query` uses tantivy's Lucene-like grammar: bare terms are combined with OR by
default (`cat dog` = `cat OR dog`); `+term` requires it, `-term` excludes it,
`"phrase words"` matches an exact phrase, and `(...)` groups. Malformed syntax
returns a structured `QUERY_PARSE_ERROR`, never a crash.

## Bounds

Per `Search`/`Explain` call: at most 1,000 documents, 1 MiB per document, 8
MiB combined. Per `Tokenize`/`Snippet` call: at most 1 MiB of text. Oversized
input returns a structured error (`TOO_MANY_DOCUMENTS`, `DOCUMENT_TOO_LARGE`,
`INPUT_TOO_LARGE`, `TEXT_TOO_LARGE`) instead of building the index.

Every `query` string, on every node, is capped at 10,000 bytes and 12 levels
of `(`/`[`/`{` nesting, checked before the query ever reaches the parser
(`QUERY_TOO_LARGE` / `QUERY_TOO_DEEPLY_NESTED`). This exists because tantivy's
query grammar has an empirically-confirmed **exponential-time** worst case for
certain nested, operator-less clause groupings (e.g. bare `((((term))))` or
nested quoted phrases) — not linear recursion depth, but combinatorial
ambiguity resolution: ~300µs at 4 levels, ~3ms at 8, ~45ms at 12, ~720ms at
16, and it had not returned after 3 seconds at 20 on ordinary hardware. The
same nesting depth WITH explicit `AND`/`OR` between clauses stays linear and
cheap past 24 levels — real, human-written boolean queries are unaffected.
Because a fixed depth heuristic cannot be proven to catch every possible
expensive shape in a third-party grammar, every parse additionally runs under
a 1.5-second wall-clock deadline (`QUERY_TOO_COMPLEX` if it fires) — a caller
never waits indefinitely for a response, regardless of what triggers the cost.

## Correctness

BM25 scoring is checked in the test suite against the textbook
Robertson/Sparck-Jones formula (k1=1.2, b=0.75, matching tantivy's fixed
defaults), computed independently in the tests from raw term-frequency /
document-frequency / length statistics — not by round-tripping through
tantivy's own `Explanation` API. `en_stem` output is checked against the
Porter/Snowball stemmer's well-documented behavior. Highlight offsets are
checked against plain substring search on the original text.

## Caveats (honest edges)

- **Determinism holds for a fixed analyzer and document set**, not across
  tantivy library versions or different indexing thread counts — this
  package always indexes single-threaded with one commit, so within one
  pinned tantivy version results are stable.
- **BM25 statistics (IDF, average field length) are computed over the whole
  document set given in that one call.** The same document scored in a
  different call, alongside a different document set, will get a different
  score — this is inherent to BM25, not a bug.
- **`Explain`'s `target_id` is looked up by exact match on `Document.id`** (or
  the input index string when `id` was empty); if two input documents share
  the same `id`, the first indexed one is used.

## License

MIT. Built for the [Axiom](https://axiomide.com) marketplace.
