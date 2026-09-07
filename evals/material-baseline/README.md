# Material Inventory and Import/Search Baseline

P01 / [issue #47](https://github.com/CaiZongyuan/knowmesh/issues/47), pinned product base
`e8ecbb657d95718976039bd4836affbb1dc64085`. This directory owns the shared
Virtual Cell material identities, acquisition references and unreviewed candidates.
It verifies existing Source CLI behavior without model calls. It is not the human
Compiler, Retrieval or Answer evaluation required by
[SPEC 22.4-22.6](../../docs/KnowMesh_v0.1_Technical_SPEC.md#224-compiler-eval).

## Inputs and Rights

[inventory.json](inventory.json) records six original snapshots: STATE v1, scGPT,
Geneformer, Tahoe-100M v1, genome-scale Perturb-seq and the Virtual Cell Challenge
website shell. Each entry owns its title, author/publisher, version, acquisition
date, original/acquisition URLs, byte hash, citation location, license evidence and
storage/distribution constraints. Acquisition was performed on 2026-09-07 using
public endpoints, without an authenticated browser or subscription credentials.
Accessibility does not establish redistribution rights or complete paper access.

Originals are independently acquired and are **not committed**. The publisher
pages for scGPT and Geneformer declare exclusive rights; Tahoe-100M v1 explicitly
reserves all rights. Challenge redistribution rights remain unverified. These
materials have no quoted or translated body fixture here. Consult each entry's
terms before local retention or reuse; do not put restricted originals in a PR.
The adjacent repository's Chinese reading notes were leads, not authoritative
inputs or a source of licensed fixtures.

[fixtures/](fixtures/) contains only selected paper text from STATE v1 and
Perturb-seq, whose acquired license notices explicitly specify CC BY 4.0.
The files provide attribution, original URLs, license links and modification
notices. They exclude page chrome, figures, external images and supplements.
These third-party excerpts remain CC BY 4.0 rather than inheriting the repository's
MIT license. Text coverage is deliberately limited to these two papers; other
subjects have metadata and question candidates, not reviewed excerpt coverage.

Original SHA-256 identifies the acquired bytes, including dynamic HTML. A new
download with different bytes fails validation, even when the DOI or apparent
article text is unchanged. Do not silently update the hash: retain the previous
snapshot identity, review the change, assign a new material ID for a new snapshot,
then regenerate affected candidates and invalidate their previous reviews.
Perturb-seq's JATS XML is saved as `perturb-seq.txt` with identical bytes because
the existing CLI accepts TXT but not XML. This tests byte preservation, not XML
parsing quality. No PDF parsing or Chunk indexing is exercised.

## Reproduce

Prerequisites: Python 3.10+, the repository's Rust toolchain and a built `knowmesh`
binary. From the worktree root:

```bash
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$PWD/target" cargo build -p knowmesh --locked
python3 evals/material-baseline/baseline.py \
  --binary target/debug/knowmesh \
  --inventory evals/material-baseline/fixture-inventory.json \
  --inputs evals/material-baseline/fixtures \
  --report .agent-task/offline-baseline.json
```

This offline run imports **two derived excerpt files**, not six original papers.
[fixture-inventory.json](fixture-inventory.json) is generated from the original
inventory; each derived input has its own hash and points to its original material
ID/hash. Do not use `--acquire` with this inventory: originals cannot substitute
for derived excerpts and their hashes will differ.

For the six-original baseline, place independently acquired files under the
filenames in `inventory.json` in a private directory, or request acquisition of
missing files:

```bash
python3 evals/material-baseline/baseline.py \
  --binary target/debug/knowmesh \
  --inputs .agent-task/materials \
  --acquire \
  --report .agent-task/original-baseline.json
```

Downloads require network access and the material's acquisition prerequisites.
The script validates pinned hashes before storing downloaded bytes. Existing
files are validated too; they are never silently replaced. An unavailable or
changed source makes the report fail and returns exit code 1 while continuing
with other sources. A newer public HTML rendering can therefore prevent an exact
replay; the offline subset remains independently reproducible. An HTTP 200 or
successful import is not proof that gated scientific content was acquired.

Every invocation initializes a clean temporary workspace, runs `version`, `init`,
`source add`, `sync`, `source get`, `source content --raw`, repeats `source add`
with the recorded `--source-id`, and runs English and Chinese searches. It records
the binary version/hash, input inventory hash, input byte hashes, Source and
Revision IDs, search responses and failures. It verifies byte-exact reads and
same-workspace revision deduplication. Fresh runs retain material/content identity
but allocate different Source/Revision ULIDs. Reports retain this mapping after
the temporary workspace is deleted; replay reacquires bytes by material ID/hash.

Reports record argument arrays with `$BINARY`, `$WORKSPACE` and `$INPUTS` placeholders
for the provided executable, new temporary workspace and input directory. CLI JSON
responses are retained, but raw source output is represented only by hash and byte
count. The child CLI receives a temporary home and minimal environment without
inherited model credentials. Missing binary/invalid inventory are preflight errors;
acquisition/import/read/search failures appear in the report.

## Observed Baseline

The committed [original report](reports/originals.json) and
[offline report](reports/offline.json) are actual runs on the pinned product base,
using CLI `0.1.0`, API contract `1.0.0`. The exact binary and inventory hashes are
in each report.

| Check | Six Original Snapshots | Offline Excerpts |
| --- | --- | --- |
| Import, sync, Source get, byte-exact revision content | 6/6 | 2/2 |
| Repeat import reuses revision | 6/6 | 2/2 |
| English metadata query matches the corresponding Source | 6/6 | 2/2 |
| Chinese metadata query matches the corresponding Source | 6/6 | 2/2 |
| Body-only probes present in acquired UTF-8 bytes, but no corresponding hit | 5/5 | 2/2 |

Chinese tags were curated in the inventory and supplied through `source add
--tag`. Their successful retrieval demonstrates metadata tokenization, not
cross-language retrieval of English body text. Source FTS currently indexes
titles, authors and tags. Body probes deliberately use terms absent from that
metadata; `query_in_raw_utf8` records a literal byte-text check, not a parsed
Evidence locator. No MRR, Recall, nDCG, scientific support or answer-quality score
is asserted. Body indexing belongs to P09.

Initial acquisition used `curl -sSfL --max-time 45 <acquisition_url> -o <filename>`
for the five papers. The first `https://virtualcellchallenge.org/` request failed
with curl exit 35 (`SSL_ERROR_SYSCALL`); a subsequent 30-second request to the
recorded URL without the trailing slash succeeded. The alternate
`https://arcinstitute.org/virtual-cell-challenge` also failed with curl exit 35.
The acquired Challenge page has the site title and a JavaScript application shell;
competition rules, split definitions and scoring protocol were not acquired. Its
`differential expression` query is classified as `unavailable-content`, excluded
from the five verified body-probe gaps. q09 remains blocked on protocol acquisition.
Transient TLS failures are acquisition observations, not product CLI failures.

## Candidates and Human Review

[excerpts.json](excerpts.json) contains 20 stable excerpt IDs with original
snapshot hashes, HTML/JATS paragraph IDs, research topics, derived fixture paths,
Unicode character ranges (zero-based, end-exclusive) and normalized-text hashes.
Quotes live once in the attributed fixture Markdown. These locators are candidate
preparation coordinates, not production Evidence IDs or parser character ranges.
HTML entities are decoded, markup removed and whitespace collapsed; future parser
output must be matched to its own Revision/parser identity before creating Evidence.

[questions.json](questions.json) contains 10 stable research-question candidates,
their topics and source locations. q01 comes from the SPEC question; the remaining
questions were authored by the implementation agent from the material topics,
not represented as observed user interviews. Broad section references on restricted
papers require human verification against independently accessed originals.

All candidates are `unreviewed`. Gold annotations/answers, relevance judgments and
factual-support judgments are null. [reviews.json](reviews.json) is the empty review
record entry point and defines the fields for a named human reviewer. Before any
P51/P52/P53 quality evaluation, a human must verify the exact original snapshot and
locator, approve or reject the candidate, record scientific gold/relevance/support
judgments as applicable, and identify missing or contradictory evidence. A passing
hash check, model answer or this agent's preparation cannot sign that approval.
P01 completion does not mean scientific accuracy or release gates have passed.

## Maintenance and Checks

Regenerate candidates only from the two licensed originals with their pinned hashes:

```bash
python3 evals/material-baseline/prepare_candidates.py --inputs .agent-task/materials
python3 -m unittest discover -s evals/material-baseline -v
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 CARGO_TARGET_DIR="$PWD/target" \
  cargo test -p knowmesh --locked --test source_cli --test source_read_cli --test search_cli
bash scripts/agent-checks.sh
```

The Python tests invoke the real CLI in temporary workspaces and a temporary local
HTTP server. They cover two fresh runs, revision deduplication, metadata success,
body misses, successful acquisition, HTTP 404, missing local files, changed local
and downloaded bytes, a rejected PDF import, continuation after failures, fixture
hashes/ranges and the initial unreviewed candidate state. No server, model or key is
required outside the test process. Candidate regeneration was compared byte-for-byte
against the checked-in files. Source acquisition and scientific review remain
separate from these mechanical checks.
