# Development

The [SPEC](KnowMesh_v0.1_Technical_SPEC.md) defines the full v0.1 target.
[Tracking issue #1](https://github.com/CaiZongyuan/knowmesh/issues/1) owns the delivery
checklist. A completed foundation or a green subset of tests is not a v0.1 release.

## Prerequisites

- Rust 1.96.0 with rustfmt and clippy, selected by `rust-toolchain.toml`.
- Backend development does not require Node.js, a browser, or a database server.
- Dependency versions are recorded in `Cargo.lock`.

## Checks

Run from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo run -p knowmesh -- version
```

Core owns the domain, use cases, and wire contracts. The SQLite crate implements
storage ports. The executable owns CLI/HTTP adapters and dependency assembly.

The [architecture guard](../crates/knowmesh-core/tests/architecture.rs) runs with
the workspace tests. Its [policy registry](../crates/knowmesh-core/tests/support/architecture-policy.json)
owns approved composition roots, canonical/projection/runtime writers, filesystem/SQL
writers, process users, and narrow exceptions. New write paths require reviewing
that registry and the corresponding transaction tests. Coverage and limitations
are defined in [SPEC section 22.9](KnowMesh_v0.1_Technical_SPEC.md#229-架构门禁与故障恢复).

## Verified Behavior

- Architecture checks inspect Cargo package identities and production Rust module
  ASTs, including `#[path]` modules, while excluding `#[cfg(test)]` modules.
  Fixtures reject Compiler writes, direct adapter database access, unregistered
  writes, and raw connection/writer exposure; registered reconcile/migration
  paths pass. CLI dependency assembly returns Core ports from `runtime.rs`.
- Operation contract tests inspect every CLI operation mapping against the Core
  descriptor registry without executing handlers. Unknown names, dynamic mappings,
  wildcard fallbacks, and missing mappings fail the gate. This supplements Rust's
  exhaustive enum matching; HTTP route coverage must be added when #34 lands.
- Typed IDs reject another object's prefix, malformed payloads, and ULID overflow;
  valid IDs survive JSON round trips without changing identity.
- Content digests are SHA-256; timestamps serialize as RFC 3339 UTC with `Z`.
- `version` works outside a workspace and emits one JSON value on stdout.
- Invalid CLI arguments and unsupported formats produce typed JSON on stderr,
  an empty stdout, and the corresponding nonzero exit code.
- Core maps every error type to CLI exit and HTTP status codes, with an explicit
  fetch-timeout override. JSON snapshots fix complete success, paginated, and error
  envelopes, including omitted optional fields and forward-compatible error reads.
  The mapping is defined in SPEC 11.8; actual HTTP responses remain part of #34.
- `schema list` and `schema command <operation>` expose the Core operation
  registry, including input/output JSON schemas, policy, effect, and support flags.
- `init [path] --name <name> --template research` creates a portable workspace;
  `--dry-run` reports planned paths without creating the destination. Repeating
  identical initialization preserves the workspace ID and existing content.
- Workspace loading validates config versions, confines an optional Purpose to
  the workspace, caps it at 16 KiB, and resolves model secrets only when requested.
  The `general` template has no research Purpose. Initialization preflights
  existing paths, including `.gitignore`, before creating canonical files.
- `schema pack <id-or-id@version>` reads a configured pack through the same
  workspace resolution rules. Base, Research, and Clinical Preview are embedded;
  `init --template clinical` selects strict review defaults without a research
  Purpose. Clinical Preview is not a production clinical capability.
- Schema validation rejects cycles, ambiguous overrides, undefined relation
  endpoints, and invalid properties. Pack order does not change the schema hash.
  Detailed merge and property rules live in SPEC section 7.3.
- Core Source Library plans managed/referenced/snapshot imports and soft removal.
  Historical revisions cannot be rewritten; importing an already recorded hash
  returns that revision without moving the current head. Reading content verifies
  its size and SHA-256, including referenced files. Unchanged manifests round-trip
  byte-for-byte. `source add <path>` commits local managed/referenced imports;
  `source remove <id> --yes` commits soft removal. Both support `--dry-run`
  without creating a database. Explicit idempotency keys and HTTP APIs are still pending.
- `source add <url>` fetches a single HTTP(S) resource through `reqwest` before
  opening the index. Core checks workspace/source policy; the actual DNS resolver
  checks every connection address. Literal, mapped, private, metadata, and special
  addresses are blocked by default; local CLI can explicitly permit private
  networks for one import. Every redirect is revalidated, with a five-hop maximum.
- Fetching ignores environment proxies, enforces declared and actual size limits,
  and uses configurable connection and total fetch timeouts. It requires HTTP 200
  and a supported Content-Type, rejects compressed responses, and validates the
  returned bytes before index creation. Fixtures cover DNS/private targets,
  redirects, truncated/unbounded bodies, header/body timeouts, and invalid PDFs.
  Preview performs the download without writing snapshots/indexes; confirmed
  imports preserve the final URL and remain readable offline. Defaults, ranges,
  and transport limits live in SPEC 13.4. Server policy integration remains #34.
- `source list/get/content` share fast synchronization and report the actual index
  generation/completeness. Lists return bounded summaries with exact kind/tag
  filters and opt-in removed sources. Cursors bind workspace, filters, and index
  generation/hash; page counts and rows use one SQLite read transaction. CLI
  envelopes expose continuation through `meta.next_cursor` as well as the result DTO.
- Source detail preserves revision history after soft removal. Content accepts a
  Source ID for its indexed head or a Revision ID for fixed historical content;
  reads verify indexed size/hash even with `--no-sync`. JSON uses UTF-8 for text
  and Base64 for PDFs. `source content --raw` emits exact bytes and rejects an
  explicit `--format` in any argument order. See SPEC 13.1.1 for the read contract.
- Core parses and renders Node and Synthesis Markdown. Unchanged documents keep
  their exact bytes; edited claims only replace their managed content. CommonMark
  source spans distinguish markers/wiki links from code examples; lossless YAML
  edits retain unknown frontmatter and comments. Citation validation checks
  canonical Evidence IDs and never creates a missing dependency snapshot.
- The SourceParser port now parses UTF-8 Markdown, TXT, and HTML into typed blocks,
  normalized text, stable revision/profile-scoped IDs, section paths, and Unicode
  character spans. Markdown frontmatter is bounded, preserved as metadata, and
  never interpreted as application configuration; YAML aliases are rejected.
- CommonMark and HTML5 DOM fixtures cover tables/captions, code, lists, repaired
  markup, hidden/script content, and raw HTML blocks. Bounded generated inputs
  exercise malformed structures and span validity. Parser descriptors support
  cache keys before parsing; artifact validation checks revision/text integrity.
  Normalization and locator semantics live in SPEC 13.2. Full Compiler integration
  remains under subsequent compiler issues. The parser
  itself performs no network or file I/O.
- `source add --encoding <label>` records the explicit text encoding on each
  immutable revision. Strict decoding is shared by local/URL import validation,
  content JSON, and parser version 2. Raw snapshots and `--raw` retain the exact
  original bytes; UTF-16/BOM and Windows-1252 fixtures verify this distinction.
  Duplicate hashes retain recorded encoding; conflicting reinterpretation and
  historical metadata edits are rejected. `--no-sync` uses indexed encoding.
  Defaults, labels, and error contracts live in SPEC 8.2/13.2.
- The built-in PDF parser uses bounded `lopdf` loading and per-page text extraction.
  Source and descriptor identity, physical page numbers, and normalized character
  spans are retained. Selectable/image-only/encrypted fixtures, missing page maps,
  Unicode-map precedence, garbled text, and decompression/output limits are tested.
- PDF quality gates report ready/needs_ocr/blocked and control usable_for_compile.
  Explicit Unicode maps are validated before extraction; broken mappings do not
  silently become accepted fallback text. Encrypted documents return no extracted
  text. Thresholds and supported limits live in SPEC 13.3; complex layout recovery,
  OCR, and real-paper extraction quality are not established by synthetic fixtures.
- Core chunking uses `text-splitter` within top-heading/page/table boundaries,
  with pluggable token counting and an explicit language-aware estimate fallback.
  Chunks preserve exact normalized source spans and never change Evidence locators.
  Validation rejects missing source text, inconsistent hashes, and boundary violations.
- The filesystem stage cache uses typed dependency keys, content-addressed JSON,
  streaming hash/size checks, synchronized files, and atomic manifests. A bounded
  OS writer lease coordinates publishers across processes; reads remain unlocked. Missing,
  damaged, incompatible, or invalid DTOs are misses; actual I/O failures propagate.
  Tests cover concurrent publishers, failed replacements, checkpoint references,
  symlink/path rejection, and independent stage/configuration invalidation.
- `parse_cached` and `chunk_cached` validate original revision bytes, parser identity,
  extraction quality, and chunk settings around cache use. Valid caches avoid repeated
  parsing; structurally corrupt entries are recomputed. Source/cache contracts live
  in SPEC 13.5/13.6. Model execution, durable Run recovery, and vector mapping remain
  under their owning issues; these helpers do not implement that complete workflow.
- Core model generation validates Schemars input/output contracts without external
  Schema retrieval, limits JSON repair and transient retries, and accounts for a
  shared request/token/deadline budget. Refusal, truncation, filtering, and tool
  responses never become successful structured output. Diagnostics omit raw content.
- The OpenAI-compatible adapter uses the configured model and API root, keeps keys
  in SecretString, bounds HTTP bodies, disables redirects, and handles Retry-After.
  JSON object/native Schema/prompt-only profiles share Core validation; token-limit
  parameter names are configurable. Tests cover profile identity, secret rotation,
  malformed responses, rate limits, deadlines, and unknown-usage estimates. Details
  and remaining Run/cost integration are specified in SPEC 14.4.
- Core Evidence verification checks immutable parse identity and extraction quality,
  matches exact Unicode spans, and repairs only unique quotes within a bounded
  page/section/paragraph scope. Whitespace-only normalization retains original
  source coordinates; overlapping and repeated scoped matches are rejected.
- Fixtures cover invalid metadata, unknown pages, cross-boundary spans, long Unicode
  text, scoped search limits, and chunk-independent locators. Successful results mint
  Evidence IDs and normalized quote hashes. Defaults/errors live in SPEC 14.5.
  Proposal Builder now rechecks actual source bytes without repairing stored offsets;
  Actual Apply now enforces the same assertion gate, including invalid quote/locator
  rejection and source-byte verification when recovering an interrupted transaction.
- Deterministic entity resolution indexes a complete validated Node catalog and
  uses Schema-declared identifier adapters plus normalized names/aliases. Only
  unique compatible identifier/alias matches are eligible for automatic linking;
  canonical names, conflicting identities, and ambiguous matches remain reviewed.
- Candidate truncation follows uniqueness checks and is explicitly reported.
  Tests cover DOI URL decoding, opaque/NCBI identity rules, Schema hash changes,
  inactive nodes, type conflicts, duplicate identities, and catalog ordering.
  Contract details live in SPEC 7.3/14.6.
- Core batch entity resolution fast-syncs and reads the full catalog plus word,
  trigram, and short title/alias candidates in one SQLite transaction. Snapshot,
  workspace, Schema, and candidate metadata must agree. Body-only FTS matches
  are excluded; exact-match ambiguity survives lexical filters and display limits.
- Real fixtures cover external updates, close ranking scores, literal operators,
  short Unicode aliases, missing FTS channels, and mixed or altered snapshots.
  Retrieval scores are review suggestions; context hashes retain the actual
  channels/configuration.
- Entity model advice uses a versioned prompt, closed output Schema, and the
  shared generation budgets. Unknown or incompatible targets fail with usage
  retained; ambiguity/truncation cannot become a silent selection or new Node.
  Advice always requires review, and already automatic matches skip model calls.
  Fake provider tests cover valid/new/ambiguous suggestions, invalid targets,
  input mismatch, bounded repairs, and safe diagnostics. Optional vectors and
  Compiler/Proposal orchestration remain #18/#27/#28.
- Canonical Claims can retain shared conflict groups with typed IDs, members,
  reasons, review state, and timestamps. Validation requires complete identical
  copies within one subject and qualifier scope. Group changes affect assertion
  freshness hashes while leaving exact-dedup identity unchanged.
- Conflict groups and memberships are projected atomically with Claims. Fixtures
  cover updates/removal, failed-insert rollback, a fresh projection, and the actual
  atomic rebuild including its backup. Contract and storage rationale live in
  SPEC 14.7/26.3.
- Exact assertion deduplication produces changes and ID mappings while preserving
  existing statement/identity/metadata and conflict records. Physical Evidence reuse
  keeps canonical payloads; different revisions, locators, and stances remain separate.
  Directed relation orientation, inactive history, ID conflicts, and batch ordering
  are covered by fixtures. These plans do not authorize canonical writes.
- Claim exact keys now preserve case and compatibility characters in scientific
  statements. Migration 0004 forces legacy keys through a complete reconciliation
  before the metadata fast path is reused. Tests preserve canonical bytes and keep
  Co/CO assertions in separate active rows.
- Semantic Claim comparison fixes a validated context and enumerates same-scope,
  non-exact pairs involving focus Claims through bounded cursor pages. Shared Evidence
  IDs must retain identical payloads. Model batches
  must classify every supplied pair exactly once; unknown, missing, duplicate, or
  malformed results fail with usage retained. Reports bind inputs and prompt content.
- Conflict plans preserve statement/Evidence/lifecycle data, retain possible-duplicate
  and undetermined advice, reuse open groups, and preserve closed history. Missing
  Evidence or group limits produce explicit blocked pairs. Overlapping changes retain
  the original comparison hashes and always require review.
- Six golden scenarios use verified source quotes and fake provider responses, then
  round-trip reports/plans and render the resulting canonical Claims. Other fixtures
  cover pagination, stale context, model repair bounds, overlapping groups, and limits.
  These checks establish controller behavior, not real-model judgment quality (#42).
  SPEC 14.7/14.8 define the remaining Proposal/Run integration (#27/#28).
- The Proposal domain now models closed patch names, immutable revision transitions,
  per-item review, bulk/strict policy, human confirmation, and finalized/stale states.
  Review hashes bind both item content and Proposal context, including decision
  metadata; changes cannot silently reuse an older approval. Same-input review is
  a no-op, while rebase or item-order changes reset decisions.
- Proposal state fixtures cover typed targets, blocked acceptance, preserved
  rejections, stale revisions, edited payloads, attestation changes, and JSON
  round trips. The public Proposal workflows below now use these transitions.
  The contract and limits live in SPEC 14.9.
- The read-only Proposal Builder decodes all thirteen operation payloads, checks
  actual source Evidence, binds original file hashes, and previews the resulting
  canonical graph. New objects are available to later dependent edits regardless
  of input order. Invalid items remain blocked and cannot be accepted.
- Builder fixtures cover each operation, cross-item references, source metadata
  restrictions, escaped Node titles, immutable closed conflict history, Schema/ID
  failures, and bounded Evidence/diagnostics. Synthesis snapshots retain supplied
  historical hashes/heads and require valid references; the Builder does not prove
  their Ask-run origin. Full payload contracts and bounds live in SPEC 14.8.
  Ask integration must supply the original run snapshot rather than reconstructing
  its contents. User-supplied idempotency keys and combined accept-all/apply remain #27.
- Accepted-subset previews rerun Builder against current files and actual Schema
  policy. Rejected dependencies, changed generation/content, forged item hashes,
  and reviews missing Builder-derived preconditions cannot yield an accepted preview.
  Five fixtures cover selection, strict/human policy, and stale baselines. The helper
  uses the generation/base hash supplied by its coordinator and never writes.
  Actual Apply now supplies these values under the workspace lock.
- SQLite Proposal storage appends complete, hashed revision snapshots and updates
  current header/items plus audit in one transaction. Reads recover the original
  review metadata; saves compare expected revisions, preserve identity, and reject
  direct applied-state writes or restoration of stale approvals. Same-revision,
  identical saves are no-ops. Baseline checks use the complete indexed snapshot.
- Nine storage fixtures cover concurrent writers, rollback, corrupt history,
  stale approvals, legacy migration, runtime copying, and real atomic rebuild with
  backups. Migration 0006 preserves legacy rows without inventing missing review
  snapshots. User-supplied idempotency keys remain #27.
  The storage contract, bounds, and rationale live in SPEC 14.9.
- Core Apply revalidates persisted approved items and writes their canonical files.
  SQLite holds the revision comparison and file callback in one write transaction,
  then commits the projection, applied revision, audit, and durable receipt together.
  Replays return the original receipt; no-op Apply creates no file journal or new
  generation. Preview and missing confirmation do not change runtime or canonical data.
- Twelve Apply fixtures cover real Compiler assertions, invalid quote/locator and
  missing Synthesis Evidence, stale revisions/content, partial file replacement,
  rollback after file changes, interruption after DB commit, Doctor recovery,
  referenced-source changes, and receipt preservation through real rebuild/backups.
  Ordinary reconcile cannot consume a Proposal-journal snapshot, and the architecture
  guard blocks adapters from directly calling the new transaction port.
- Proposal journals use version 2 and migration 0007 stores Apply receipts. Legacy
  source/initialization journals remain compatible. Contracts, bounds, transaction
  tradeoffs, and remaining idempotency-key work live in SPEC 10.6/14.9.
- Core create/edit/review/revalidate/reject workflows check canonical baselines,
  actual Schema policy, expected revisions, and pending journals before runtime
  writes. Dry-runs preserve both index and history. Stale review records a stale
  revision; explicit revalidation refreshes the index and resets affected approvals.
  Six workflow fixtures include diagnostic repair, partial review retention, invalid
  current Schema, historical reads, and interrupted Apply exclusion.
- Builder diagnostics carry optional `origin: builder`; new validation replaces
  those findings while retaining unmarked upstream/legacy warnings. Omitted origin
  retains the old JSON representation. The runtime-write architecture guard rejects
  direct adapter calls to Proposal create/save ports.
- CLI exposes `proposal create/get/edit/review/revalidate/reject/apply` and
  `schema patch <op>`. Four shell fixtures exercise stdin/file JSON, descriptor
  discovery, repair/revalidation, historical reads, confirmation, and actual Apply.
  Invalid JSON does not open an index; runtime reads and rejection do not load a
  broken Schema Pack. Complete request contracts and CLI examples live in SPEC 14.9.
  Proposal list, explicit idempotency keys and combined accept-all/apply remain pending.
- Canonical document previews overlay Node/Synthesis Markdown and existing source
  metadata in memory. They reuse projection and link resolution, revalidate the
  complete reference graph, and check that the original file inventory/content
  remains unchanged. Preview hashes match a full scan after the same bytes are written.
- Previews have a distinct read-only type. Canonical snapshots retain a private
  integrity seal, so copying proposed data and its public hash into a scanned
  snapshot cannot produce an indexable snapshot. Fixtures cover identity/source
  restrictions, new link resolution, Schema/reference failures, and external changes.
  Builder and actual Apply now supply patch payload/quote validation.
- Node summary editing shares CommonMark section recognition with indexed summary
  extraction. It preserves frontmatter, other sections, managed assertions, line
  endings, indented code, and existing reference definitions. Structural injection
  and ambiguous summary sections are rejected. Missing sections can be inserted.
- Summary fixtures cover quoted/code headings, external reference definitions,
  nodes named Summary, clearing/no-op edits, and protected Markdown structure.
  Migration 0005 refreshes old derived summaries without changing canonical files.
  Windows preview paths follow the scanner's component-by-component path layout;
  the first preview CI exposed an otherwise different hash for new files.
- SQLite bootstrap applies checksum-verified migrations, binds one workspace ID,
  and configures WAL/foreign keys/busy timeouts on each connection. Existing
  stores remain readable during another connection's write transaction. Both
  FTS indexes follow search-unit insert/update/delete through triggers.
- The Core lexical port queries both FTS indexes and short title/alias fallback
  through one SQLite read transaction. Literal input escapes FTS operators, quotes,
  and LIKE wildcards. Mixed English/Chinese fixtures verify word, substring, and
  one/two-character recall, including long/short mixed terms.
- Candidate queries apply all Search filters before per-channel limits,
  retain empty successful channels, and return stable ranks/public identities.
  Shared ownership/dependency CTEs are materialized once per query to limit repeated
  expansion inside correlated filters; large-corpus latency still requires #42.
  Advanced syntax is opt-in; query bounds and SQLite execution interruption apply
  to both modes. Error paths release the progress callback for later queries.
  Contract details live in SPEC 9.3/15.6.
- Core's pure RRF implementation uses configurable weights and the theoretical
  channel bound, preserves successful empty channels, excludes unavailable ones,
  deduplicates per channel, and sorts exact IDs in a separate tier. Explanations
  include contributions, bounded boosts, and degradation reasons. Tests preserve
  a no-boost baseline and scores across candidate-tail truncation. Workspace config
  validates weights/budgets and supplies defaults for older files. Retrieval
  quality and performance evaluation remain part of #42.
- Ranked snapshot pagination binds workspace/query, generation/snapshot hash,
  ranking settings/candidate limits, actual channels, and candidate results.
  The opaque cursor stores an exact-tier/score/ID position. Tests cover changing
  final page sizes, stale snapshots/config/channels/candidates, and malformed
  positions. CLI returns the continuation in both data and envelope metadata.
- `search` calls the registered `knowledge.search` Core operation. Ordinary and
  exact-ID candidates share filter semantics, including recorded assertion/source
  links, Synthesis dependency snapshots, and Chunk owners (SPEC 15.4). A real
  workspace fixture verifies filters before a one-candidate channel limit.
- Candidate identities, canonical aliases, bounded Unicode previews, freshness
  dependencies, and index version are read in one SQLite transaction. Search
  fast-syncs by default; `--no-sync` reports unknown freshness. Removed sources
  retain historical evidence and mark affected knowledge as needing review.
- Failed lexical I/O channels expose their error code, drop partial candidates,
  and trigger renormalization with a degradation warning. An actual missing
  trigram index fixture invalidates the previous cursor. Validation, syntax,
  lock, and execution-budget failures remain typed errors. Optional vectors and
  Graph paths remain unavailable until #18/#19 and are explicitly disclosed.
- Core scans canonical files into a validated snapshot, resolving wiki links and
  checking cross-object references, schema constraints, and managed revision
  hashes. Ambiguous or unresolved links produce warnings. A stale Workspace or
  a modified projection payload is rejected before indexing.
- SQLite reconciles Source, Node, Claim, Relation, Evidence, Synthesis, link,
  search-unit, and file-manifest projections atomically. Identical content keeps
  its generation; file swaps and Claim replacements preserve surviving IDs and
  runtime references. Deleted files remove their projections. Historical Source
  revisions cannot be rewritten. Failed validation or a database error leaves
  the previous complete projection intact.
- `sync` performs a full canonical scan and projection update; `sync --dry-run`
  only validates and reports files. `status` uses a file-list/size/mtime fast path
  and falls back to a full scan when those hints change. Timestamp-only changes
  refresh the hints without advancing generation. Link warnings survive the
  fast path and database upgrades. Explicit `sync` always rechecks content hashes;
  it also detects edits whose size and mtime were intentionally preserved.
  The server watcher remains under KM-022. Application Source writes first
  validate/reconcile the existing projection, then commit their file journal and final projection under one
  workspace lock. Core recovery completes interrupted writes before permitting
  new syncs; repeating recovery after a database commit keeps its generation.
- `status --no-sync` keeps the previous index generation while retaining Schema
  validation. During transaction recovery or another active writer, status can
  report the last complete index and the reason synchronization was skipped.
- `doctor` inspects database integrity/version, canonical references, pending
  transactions, index freshness, and Git ignore/tracking state. Missing, outdated,
  and corrupt databases are reported without creation or migration. Git is
  optional; unavailable Git produces a diagnostic warning. Doctor locator checks
  remain structural; the Compiler verifier is not yet connected to diagnostics.
- `doctor --repair --dry-run` reports the current diagnostics and pending paths.
  `doctor --repair --yes` rolls forward pending transactions and synchronizes
  the index. Corrupt databases and invalid journals are preserved for explicit
  recovery; this command does not discard runtime state or modify Git.
- Doctor also resolves workspaces by their transaction directory when configuration
  is missing or invalid. Diagnostics retain the configuration error and use a null
  workspace ID until loading succeeds. Repair preview checks every target/staged
  hash. Confirmed repair checks the intended configuration against an existing
  readable database, rolls forward under the workspace lock, and completes the
  journal only after canonical validation and index commit. Unjournaled external
  damage is preserved; database read/version errors block pending file writes.
- The SQLite rebuild helper copies all seven runtime tables from one read
  transaction into a separate candidate. It preserves self-referencing Runs,
  Proposal revision links, idempotent responses, and the audit sequence even
  after event deletion. Invalid references roll back the candidate's runtime
  copy. Final runtime copying repeats while the exclusive replacement guard
  excludes writers, preserving changes committed during candidate preparation.
- Writable SQLite connections retain a shared `*.sqlite3.lease` file lock until
  the connection closes. An exclusive replacement guard rejects active writers;
  controlled maintenance connections retain that guard until they close too.
  Read-only diagnostics do not acquire a writable connection lease. Checkpoint
  or platform file-sharing conflicts stop replacement and retain both databases.
- `rebuild --dry-run` validates a candidate in memory, including runtime references,
  and reports logical counts/hash and prospective backup paths. `rebuild --yes`
  creates a separate candidate, rescans canonical content under the workspace
  lock, checks the prior generation/hash, verifies integrity, and backs up the old
  database before atomic replacement. Existing candidates are retained for inspection.
  Unchanged canonical content preserves the generation. `--keep-backups <1..20>`
  defaults to three; retention preserves the current backup and unrecognized files.
- Corrupt runtime state stops rebuild by default. `--discard-runtime --yes`
  explicitly discards all seven runtime tables while keeping a verified backup.
  The flag cannot override workspace identity, migration checksum, or database
  version errors. Older databases currently require migration before rebuild;
  `sync` applies recognized migrations. Server connection draining is still pending.
- Core freshness evaluation compares Source heads, removal state, and recorded
  assertion hashes. It preserves all Evidence IDs, identifies evidence from
  current sources separately, and derives deterministic reasons. Missing snapshots,
  missing dependencies, or incomplete synchronization produce `unknown`, even if
  another dependency has changed. Incomplete indexes cannot mark individual
  Evidence as current.
- `source impact <id>` traverses Evidence, Claim, Relation, and Synthesis references,
  including snapshot-only assertion and Source head dependencies. `--revision`,
  `--kind`, `--limit`, and opaque cursors select stable `(kind,id)` pages. Counts
  reflect all matches under the filters. One SQLite read transaction covers counts,
  rows, and the dependencies needed for that page. Cursors bind workspace/query
  and generation/snapshot hash; stale or changed queries return typed errors.
  `--no-sync`, pending recovery, and skipped synchronization produce unknown
  freshness. Source updates/removal preserve independent Evidence, and rebuild
  preserves impact results. Search shares these freshness rules; HTTP integration
  is still pending.
- `source remove --dry-run` includes an impact preview built from canonical files
  in an in-memory index. It uses the same query and freshness rules, preserves the
  disk index, and returns the first 20 dependencies before removal. The preview
  flag distinguishes its prospective generation from a persisted index generation;
  its cursor can continue through `source impact` after normal synchronization.
  Existing incompatible/unreadable indexes stop preview without changes. Core's
  lower-level Source planning helpers remain usable without an impact backend.

Initialization now uses a durable file journal under `.knowmesh/transactions/`
and verified staging under `.knowmesh/staging/`. The Core coordinator can roll
forward after any file replacement; external edits or staged corruption stop
recovery and preserve its materials. Pending transactions block new writes.
The coordinator and reconciler are connected through Application Core Source
writes and CLI doctor repair, including interrupted initialization before
configuration exists. Server integration and freshness verification remain under
KM-023 and their owning implementation issues.

## TDD Evidence

| Issues | Red Evidence | Green Verification |
| --- | --- | --- |
| [KM-001 / #2](https://github.com/CaiZongyuan/knowmesh/issues/2), [KM-002 / #3](https://github.com/CaiZongyuan/knowmesh/issues/3) | Commit `3da473e`: core tests fail on missing domain/error modules; all three CLI tests fail on empty output and missing validation errors | `cargo +stable test --workspace`: 8 tests pass using installed Rust 1.96.0 |
| [KM-003 / #4](https://github.com/CaiZongyuan/knowmesh/issues/4) | Commit `d8c80d7`: registry tests fail on missing Application Core module | `cargo +stable test --workspace`: 11 tests pass, including schema discovery and unknown-operation errors |
| [KM-010 / #6](https://github.com/CaiZongyuan/knowmesh/issues/6) | Commits `658b109`, `6465a39`: missing workspace module and CLI init; invalid ignore paths cause partial writes | `cargo +stable test --workspace --locked`: 22 tests pass, including CLI dry-run/repeatability and workspace confinement |
| [KM-011 / #7](https://github.com/CaiZongyuan/knowmesh/issues/7) | Commits `fb1ee40`, `cfe3307`: missing schema module, pack CLI, and clinical template | `cargo +stable test --workspace --locked`: 33 tests pass, including DAG/override errors, relation/property constraints, deterministic hash, and strict Clinical Preview |
| [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), file transaction layer | Commits `9d021cd`, `7a151f5`: missing coordinator, changed staging installed after preflight, reserved path aliases accepted | `cargo +stable test --workspace --locked`: 40 tests pass, including interruption after each replacement, recovery conflict preservation, staging revalidation, and writer exclusion |
| [KM-012 / #8](https://github.com/CaiZongyuan/knowmesh/issues/8), Source Library | Commits `794e5a8`, `e477c9c`: missing source model/store, unrelated files break enumeration, interrupted cleanup and case aliases fail | `cargo +stable test --workspace --locked`: 51 tests pass, including all storage modes, immutable revisions, soft removal, content integrity, portable transaction paths, and repeated cleanup |
| [KM-013 / #9](https://github.com/CaiZongyuan/knowmesh/issues/9), [KM-014 / #10](https://github.com/CaiZongyuan/knowmesh/issues/10), canonical parsers | Commits `e82d34b`, `26209a8`: missing parsers and shared Evidence rejected | `cargo +stable test --workspace --locked`: 63 tests pass, including Unicode property tests, CRLF, managed-span preservation, YAML comments, shared evidence consistency, and synthesis citations |
| [KM-020 / #11](https://github.com/CaiZongyuan/knowmesh/issues/11), [KM-030 / #16](https://github.com/CaiZongyuan/knowmesh/issues/16), database infrastructure | Commits `6d209df`, `4caac1d`: missing store/migrations; current-store reads block behind a writer | `cargo +stable test --workspace --locked`: 69 tests pass, including migration preservation/checksums, workspace binding, WAL concurrency, and dual FTS triggers |
| [KM-021 / #12](https://github.com/CaiZongyuan/knowmesh/issues/12), canonical projection | Commits `59ceaa5`, `faf5d06`: missing snapshot/reconcile ports, unique-key conflicts during replacements, and stale or modified snapshots accepted | `cargo +stable test --workspace --locked`: 80 tests pass, including rebuild equivalence, deletion propagation, revision history checks, runtime reference preservation, and transaction rollback after an injected database failure |
| [KM-012 / #8](https://github.com/CaiZongyuan/knowmesh/issues/8), [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), [KM-051 / #31](https://github.com/CaiZongyuan/knowmesh/issues/31), Source write workflow | Commits `7360d3a`, `2075236`, `7565f76`: missing application/CLI operations and incompatible stores detected after file writes | `cargo +stable test --workspace --locked`: 86 tests pass, including preview, confirmation, duplicate import, soft removal, store preflight, and recovery before/after the database commit |
| [KM-022 / #13](https://github.com/CaiZongyuan/knowmesh/issues/13), [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), status/doctor | Commits `c4c3b1e`, `9c63316`, `a58df05`: missing fast sync, doctor, and status commands | `cargo +stable test --workspace --locked`: 93 tests pass, including change detection, stable generations, warning migration, read-only inspection, explicit repair, last-complete status during recovery, and Schema checks with `--no-sync` |
| [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), runtime preservation | Commit `be10667`: missing runtime-copy operation | `cargo +stable test --workspace --locked`: 95 tests pass, including all five runtime tables, parent/child Runs, foreign-key rollback, candidate refresh, and audit sequence preservation |
| [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), connection coordination | Commits `3865c8c`, `a4ac6b8`: missing replacement guards and controlled maintenance connections | `cargo +stable test --workspace --locked`: 98 tests pass, including multiple writers, equivalent paths, read-only inspection, and guard lifetime |
| [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), atomic rebuild | Commits `8f13e4e`, `80eca4d`, `6f71b8d`: missing rebuild, CLI/preview validation gaps, and backup ordering deletes the latest backup | `cargo +stable test --workspace --locked`: 109 tests pass, including runtime recopy, corrupt-database preservation/discard, generation conflicts, failed-backup retry, retention, and simulated interruption at four replacement boundaries |
| [KM-023 / #14](https://github.com/CaiZongyuan/knowmesh/issues/14), initialization recovery | Commit `42db30e`: doctor cannot reach recovery without a loadable configuration | `cargo +stable test --workspace --locked`: 113 tests pass, including every initialization file boundary, invalid configuration, environment/ancestor resolution, staging corruption, external conflicts, identity checks, and repeated repair |
| [KM-024 / #15](https://github.com/CaiZongyuan/knowmesh/issues/15), freshness rules | Commits `5b4190e`, `4e35f48`: missing freshness evaluation and incomplete indexes mark evidence as current | `cargo +stable test --workspace --locked`: 118 tests pass, including independent evidence preservation, snapshot/hash comparison, missing dependencies, deterministic reasons, and incomplete-index precedence |
| [KM-024 / #15](https://github.com/CaiZongyuan/knowmesh/issues/15), source impact | Commit `ba7f868`: missing impact operation | `cargo +stable test --workspace --locked`: 123 tests pass, including bounded pages, query/generation cursor checks, revision ownership, multiple sources, missing snapshots, snapshot-only dependencies, and impact/freshness equivalence after rebuild |
| [KM-024 / #15](https://github.com/CaiZongyuan/knowmesh/issues/15), removal preview | Commit `2a911f6`: source removal preview has no impact query | `cargo +stable test --workspace --locked`: 124 tests pass, including preview with missing/stale indexes, unchanged index bytes and canonical removal state, and cursor continuation after synchronization |
| [KM-004 / #5](https://github.com/CaiZongyuan/knowmesh/issues/5), architecture guard | Commits `1286326`, `7367010`, `f28795d`: missing checker, missed glob/qualified mutation/public connection cases, and adapters can invoke reconcile through Core ports | `cargo +stable test --workspace --locked`: 132 tests pass, including eight architecture tests covering dependency identities, production module discovery, registered writers, forbidden capabilities, and repository boundaries |
| [KM-012 / #8](https://github.com/CaiZongyuan/knowmesh/issues/8), [KM-050 / #30](https://github.com/CaiZongyuan/knowmesh/issues/30), [KM-051 / #31](https://github.com/CaiZongyuan/knowmesh/issues/31), Source reads | Commits `951de18`, `106794d`: missing Core/CLI reads and absent envelope continuation metadata | `cargo +stable test --workspace --locked`: 138 tests pass, including filtered pagination, query/workspace/generation mismatch, external metadata sync, historical reads after removal, content integrity, binary encoding, raw output, and format conflicts |
| [KM-012 / #8](https://github.com/CaiZongyuan/knowmesh/issues/8), URL fetching | Commits `2983eb9`, `1d0f963`: missing fetch policy/transport/CLI override, and invalid downloaded bytes create an index before rejection | `cargo +stable test --workspace --locked`: 143 tests pass, including public/private address policy, actual DNS checks, bounded redirects/downloads/timeouts, preview without writes, MIME validation before index creation, repeat-hash imports, and offline snapshot reads |
| [KM-003 / #4](https://github.com/CaiZongyuan/knowmesh/issues/4), public handler registration | Commit `237a7d9`: no automated coverage of the full CLI operation mapping | `cargo +stable test --workspace --locked`: 146 tests pass, including unregistered-handler fixtures and rejection of dynamic/wildcard/missing mappings |
| [KM-044 / #24](https://github.com/CaiZongyuan/knowmesh/issues/24), Evidence verifier component | Commit `bf76b48`: missing Evidence verifier module | `cargo +stable test -p knowmesh-core --test evidence_verify --locked`: 14 component tests; actual Apply gate coverage is recorded below |
| [KM-045 / #25](https://github.com/CaiZongyuan/knowmesh/issues/25), deterministic entity resolution | Commit `9f26ba8`: missing entity resolution module | `cargo +stable test -p knowmesh-core --test entity_resolution --locked`: 10 tests pass |
| [KM-045 / #25](https://github.com/CaiZongyuan/knowmesh/issues/25), entity retrieval | Commit `9e5fd6b`: missing batch entity resolution operation | `cargo +stable test -p knowmesh-sqlite --test entity_resolution --locked`: 7 tests cover real FTS retrieval, snapshot consistency, ambiguity, and degradation |
| [KM-045 / #25](https://github.com/CaiZongyuan/knowmesh/issues/25), bounded entity model advice | Commit `4dcec77`: missing advice function | `cargo +stable test -p knowmesh-core --test entity_advice --locked`: 6 tests cover constrained decisions, review requirements, usage, and invalid output |
| [KM-046 / #26](https://github.com/CaiZongyuan/knowmesh/issues/26), canonical conflict groups | Commit `3b75c6d`: missing conflict types and Claim metadata | `cargo +stable test -p knowmesh-core --test claim_conflicts --locked`: 7 tests pass |
| [KM-046 / #26](https://github.com/CaiZongyuan/knowmesh/issues/26), conflict projection | Commit `f8856dd`: missing group rows and projection behavior; adding groups also exposed the old rebuild hash inventory | `cargo +stable test -p knowmesh-sqlite --test conflict_groups --locked`: 3 tests pass, including actual rebuild and transaction rollback |
| [KM-046 / #26](https://github.com/CaiZongyuan/knowmesh/issues/26), exact deduplication | Commits `ddc74a8`, `f31a49e`: missing deduplication module; scientific symbols collapse and legacy keys skip reindexing | `assertion_dedup`: 11 tests pass; `fast_sync`: 5 tests pass, including two normalization/migration regressions |
| [KM-046 / #26](https://github.com/CaiZongyuan/knowmesh/issues/26), semantic comparison/planning | Commit `84f5bba`: missing Claim comparison context; an additional regression exposed conflicting shared Evidence payloads | `cargo +stable test -p knowmesh-core --test assertion_compare --locked`: 11 tests cover six golden scenarios, canonical rendering, and shared Evidence integrity |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), Proposal state/review | Commit `3fb7339`: missing Proposal domain module; further regressions expose unbound context/attestation metadata | `proposal_state`: 10 state/review tests |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), read-only Builder | Commits `8c8a5df`, `0a1f129`, `4d1fefe`: missing Builder, invalid snapshot acceptance, Evidence/diagnostic overflow, and mutable closed conflicts | `proposal_builder` and `proposal_builder_operations`: 17 tests cover all operations and actual source verification |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), accepted subset | Commit `2417e1c`: missing `prepare_accepted` helper | `proposal_selection`: 5 tests cover selected dependencies, stale content/revisions/generation, and actual workspace review policy |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), revision storage | Commits `2bbb27f`, `5544a42`: missing Proposal store, then unsafe restoration of stale approvals | `proposal_store`: 9 tests cover atomic history, concurrency, rollback, migration, copying, and rebuild |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), [KM-044 / #24](https://github.com/CaiZongyuan/knowmesh/issues/24), coordinated Apply | Commits `4093cea`, `ab764a6`, `8533304`: missing Apply, uncoordinated projection writes, and stale referenced bytes accepted during recovery | `proposal_apply`: 12 tests cover actual canonical/SQLite Apply and recovery; the architecture suite also covers the transaction port |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), [KM-051 / #31](https://github.com/CaiZongyuan/knowmesh/issues/31), authoring workflows/CLI | Commits `06db97c`, `4224d89`, `edab1cf`, `1b1e2cc`, `204b0ca`: retained stale diagnostics, missing workflows/commands, broken-Schema rejection, and unguarded runtime ports | Six Core/SQLite workflow tests, four CLI tests, diagnostic repair and runtime boundary fixtures pass; keys, combined accept-all/apply and Proposal list remain pending |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), canonical preview | Commit `9d8b7b4`: missing document preview | `cargo +stable test -p knowmesh-core --test canonical_preview --locked`: 6 tests pass, including preview/scan equivalence and rejection of proposed data as a canonical snapshot |
| [KM-047 / #27](https://github.com/CaiZongyuan/knowmesh/issues/27), controlled summary editing | Commits `1fa51a2`, `f803758`: missing summary editor, accepted reference injection, and stale summary projections | `node_summary`: 8 focused cases; `fast_sync`: 6 cases including v3 Claim-key and v4 summary refresh |

The [foundation CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33976876366)
passed formatting, clippy, tests, and CLI version smoke checks on Linux, macOS,
and Windows for commit `77fc2ec`.
The [workspace CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33978958983)
also passed on all three operating systems for commit `2c2ca76`.
The [projection CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33984019917)
passed on all three operating systems for commit `d352ac9`.
The [Source workflow CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33984687743)
passed on all three operating systems for commit `fcc9ca9`.
The [status/doctor CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33985656293)
passed on all three operating systems for commit `1326bc4`.
The [runtime-copy CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33986239519)
passed on all three operating systems for commit `70b228b`.
The [connection-guard CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33998288373)
passed on all three operating systems for commit `6ef5f2f`.
The [atomic-rebuild CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33999413554)
passed on all three operating systems for commit `18d6232`.
The [initialization-recovery CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/33999792619)
passed on all three operating systems for commit `c97d2da`.
The [freshness-rule CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/34000090333)
passed on all three operating systems for commit `904bb5c`.
The [source-impact CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/34000711119)
passed on all three operating systems for commit `afc84f6`.
The [architecture-gate CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/34002031716)
passed on all three operating systems for commit `037cac0`.
The [Source-read CI run](https://github.com/CaiZongyuan/knowmesh/actions/runs/34002667660)
passed on all three operating systems for commit `b9531de`.

The foundation evidence does not validate the remaining SPEC workflows, supported
platform matrix, model quality, or release packages. Those gates remain tracked
by their implementation issues.
