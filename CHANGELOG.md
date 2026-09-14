# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

> **On the version numbers.** `1.0.0` to `1.2.1` are the Python stack,
> preserved on the [`python-legacy`](../../tree/python-legacy) branch and no
> longer developed. `0.1.x` is the Rust rewrite that replaced it: a new codebase,
> which restarts the numbering at `0.1` because its storage layout and HTTP
> surface are not settled yet. So the newer line carries the *lower* number —
> see [Upgrading from 1.x](README.md#upgrading-from-1x).

---

## [Unreleased]

### Fixed

- **`IngestionGuard::start` returns an error instead of panicking on a
  closed semaphore.** The `acquire_owned().expect(...)` sat at the top of
  every upload request: unreachable in practice, but a panic there would
  take down the request task rather than surfacing a normal 500 — against
  this file's own rule that request-path failures are `Result`s. The
  upload handler maps it to a bare 500 (detail in the log, per the 5xx
  rule), and a new test pins the no-panic behaviour.

---

## [0.1.43] - 2026-09-14

### Performance

- **The upload and backup handlers no longer block the async executor on
  disk I/O.** Saving the original file copied it with `std::fs::copy`
  straight inside the handler, which stalls every other request sharing
  that worker thread for as long as the copy takes — up to the whole
  configured upload limit. Listing backups did the same with a
  synchronous `read_dir`. Both use `tokio::fs` now, as do the cleanup
  paths around them. The one exception is the staging file's `Drop`
  guard, which cannot await: an unlink is a syscall rather than a
  transfer, which is the part that actually mattered.
- **Uploads stream to disk instead of being buffered in RAM.**
  `process_upload` used to load the entire body into a `Vec<u8>` (plus
  a transient copy) before hashing and writing it, so a few concurrent
  large uploads could OOM the process — on ARM boards fatally. The
  multipart field is now written to the temp file chunk by chunk, the
  sha256 for the provenance chain is computed incrementally (identical
  digest to a one-shot hash), and the request is rejected with `413` as
  soon as the configured `STORAGE__MAX_UPLOAD_MB` cap is exceeded rather
  than after the whole body arrived. The `DefaultBodyLimit` stays as a
  backstop; the parser interface is unchanged, and a partial temp file is
  removed on every error path.

### Fixed

- **Restore refuses non-regular tar entries, not just escaping paths.**
  `unpack_tar_gz` checked each entry's path against traversal but never
  its type: a symlink or hard link with an innocent path still
  materialised an arbitrary target on unpack, and a restored symlink
  could redirect a later write outside the destination. Only regular
  files and directories are now unpacked — everything this project's
  own archives ever contain.

### Security

- **Uploads are staged owner-only, under the data directory, and only
  after the format is accepted.** Three things about the file an upload
  is written to while it is parsed. It was created with the default
  `0644`, so on a shared host every other local user could read whatever
  was being ingested for as long as the parse took; it is now `0600` on
  Unix. It lived in the system temp directory, which on most Linux
  installs is a tmpfs — that is RAM, so a large upload went straight back
  into the memory that streaming to disk exists to avoid, and on a small
  ARM board could fill it; staging now lives in `{DATA__DIR}/tmp`, which
  is also on the same filesystem as the originals store. And the
  extension was checked only by the parser, at step 2 — a `.pptx` was
  written out in full, up to the configured limit, and only then
  refused; an unsupported format is now a `415` before a single byte is
  written.

  Because staging left the system temp directory it also left the
  operating system's cleanup of it, so the startup sequence now sweeps
  `{DATA__DIR}/tmp` — safe there and only there, since no upload can be
  in flight before the server accepts requests, and every file present is
  by definition left over from a process that died mid-upload.

---

## [0.1.42] - 2026-09-14

### Changed

- **A search result whose payload cannot be read is now logged instead of
  silently dropped.** Points written by an older schema, or by another tool
  against the same collection, were discarded by `.ok()?` with no trace —
  which is what made "the answer ignores a document I know is in there"
  impossible to explain from the outside. Each dropped point is logged with
  its id and the parse error, plus a summary line when a search loses any.

### Performance

- **Two uploads at once no longer sabotage each other.** The ingestion
  window — unload eullm, move bge-m3 onto the GPU, parse/chunk/embed, move
  it back, reload eullm — was only counted, never serialised: the first
  upload to finish reloaded the chat model into VRAM while the second was
  still embedding in it, and the second hit CUDA OOM, fell back to the CPU
  and finished an order of magnitude slower without reporting anything to
  anyone. One permit now guards the window, so the second upload waits its
  turn; the pair takes about as long as before and neither degrades.

- **Qdrant now carries a payload index on `document_id`.** Without one,
  every `document_id` filter — which is every document deletion — was
  answered by reading the payload of every point in the collection: at ten
  thousand documents, removing one meant scanning millions of points to
  find its few hundred. The index is created on startup for existing
  collections too, not only for newly created ones, and a Qdrant that
  refuses it logs a warning rather than blocking startup.

- **The chunker no longer shifts its whole overlap window on every
  removal.** `Vec::remove(0)` moved every remaining element down a slot
  each time; with the `" "` separator the window holds 150–200 words and
  most of them are dropped at each flush, so a 10 MB document spent a few
  hundred million element moves achieving nothing. It is a `VecDeque` now,
  and each piece carries the character count taken when it went in instead
  of being counted a second time on the way out.

### Fixed

- **Startup now refuses a misconfigured `STORAGE__MAX_UPLOAD_MB`.** `0`
  used to turn into `DefaultBodyLimit::max(0)` and reject every upload
  with a misleading `413`; values above the new provisional ceiling of
  `1024`, and values whose byte count overflows `u64`, are rejected too —
  all fail fast in `Settings::load`, following the existing
  `validate_auth` pattern. The ceiling is deliberately provisional:
  while upload bodies are buffered in RAM it doubles as memory
  protection, and it can rise once uploads stream to disk.

- **A long answer is no longer truncated and then saved as if it were
  complete.** The HTTP client applied a single 180-second timeout to every
  request, and in `reqwest` that timeout covers the whole request including
  the response body — so on the streaming endpoint it was not a liveness
  check but a cap on how long the model was allowed to talk. With
  `EULLM__NUM_PREDICT=4096` a 14B model passes three minutes routinely.
  Past that the connection was dropped mid-sentence, and because a severed
  stream reached the SSE layer as the same closed channel a finished one
  does, the half-written text was stored as the assistant's reply, shown
  with a normal completion event, and — with `use_history` on — replayed to
  the model on every later turn in the same conversation. The streaming
  path now bounds *silence* rather than total duration: 30 s to open the
  connection and 180 s without a single byte from eullm, which still covers
  the legitimately long gaps (a cold model load, or the swap back into VRAM
  after `EULLM__UNLOAD_DURING_INGESTION` or an eullm-mode embedding call)
  while letting a healthy generation run to its end. An interrupted
  generation is now reported as such: the stream emits an `error` event
  instead of `done`, the partial text is deliberately not persisted, and
  the UI marks what did arrive as incomplete. Unary requests (`invoke`,
  `unload`, `embed`) keep the 180-second total timeout, which is the right
  shape for them.

### Security

- **A `conversation_id` from the request body is now checked against the
  caller before it is used.** Every read in `db::conversations` filters by
  `user_id` as well, so a conversation id belonging to someone else could
  never disclose anything — but the write paths took the id at face value,
  which let a user file their own messages under another user's
  conversation and, through the `updated_at` touch that follows every
  insert, move that conversation to the top of its owner's list. Both
  `/api/query` and `/api/query/stream` now answer `404 conversation not
  found` for an id the caller does not own (404 and not 403: whether
  someone else's conversation exists is not theirs to learn), and
  `touch_conversation` carries a `user_id` filter of its own.

- **Stale-instance cleanup at startup no longer matches on command lines.**
  Before spawning eullm the supervisor ran `pkill -f <path to eullm>`,
  which tests that path as an extended regex against the full command line
  of every process the user owns — an editor with the file open, a
  `tail -f` on it, a script that merely names it, all matched and all
  killed — and, since the path was never escaped, a data directory
  containing `+` or `(` silently changed what it matched. On Linux it now
  reads `/proc/<pid>/exe`, the kernel's own answer to what a process is
  running, and signals only the processes that are genuinely this binary
  (including one left over from an in-place upgrade, which the kernel
  reports with a " (deleted)" suffix).

---

## [0.1.41] - 2026-09-14

### Changed

- **The server now listens on `127.0.0.1` by default instead of
  `0.0.0.0`.** This binary serves its admin UI over plain HTTP and
  terminates no TLS anywhere, so the previous default put the login
  password and every session token on the wire in clear text, readable by
  anyone on the same network — while the quick start told the user the app
  lives at `localhost`. It is now reachable only from the machine it runs
  on unless `SERVER__HOST` says otherwise, and when it does say otherwise
  the server logs a warning at startup explaining what that costs. See
  "Reaching it from another machine" in the README for the reverse-proxy
  setup this replaces it with.

  **Upgrading:** an installation that was being reached from other
  machines will stop answering them until `SERVER__HOST=0.0.0.0` is set
  explicitly — preferably behind a proxy that terminates TLS.

### Removed

- **Five declared dependencies that no code referenced** — `quick-xml`,
  `validator`, `pulldown-cmark`, `walkdir` and `zstd`. Beyond build time,
  two of them were pulling known-vulnerable crates into the tree for
  nothing: dropping `validator` removes `idna 0.5.0` (RUSTSEC-2024-0421)
  entirely, and dropping the direct `quick-xml` removes one of the four
  copies flagged by RUSTSEC-2026-0194/0195.

### Fixed

- **Downloaded filenames are sanitised for the `Content-Disposition`
  header.** The stored filename comes verbatim from the multipart body
  while `storage::path_for` only neutralises path traversal, so a name
  containing `"`, `\` or CR/LF reached the header uninterpolated —
  breaking out of the quoted string at best, response splitting at
  worst. Quotes, backslashes and ASCII controls now become `_`;
  legitimate names (including non-ASCII ones) are untouched.

- **The user-management stubs no longer answer non-admin callers.**
  `GET /api/auth/users` returned `200` to any authenticated role, and the
  other stubs had no role check at all — whatever gets built on them later
  would have inherited the hole. All four now require the admin role
  (`403` otherwise), mirroring `api::admin::require_admin`, via the
  previously-dead `Role::can_manage_users` helper.


- **A single malformed SSE line no longer discards a whole streaming
  answer.** `JSON.parse` on a `data:` line ran unguarded inside the
  stream loop, so one truncated chunk (flaky network) threw into the
  outer `catch` and replaced the partial answer with a generic error.
  Malformed lines are now skipped, keeping the tokens received so far.
  The upload progress bar is also guarded against a missing
  `evt.total` (chunked encoding), which previously rendered as `NaN%`.

- **`POST /api/auth/change-password` now enforces a password policy
  instead of accepting anything.** An empty (or up to 5-character) new
  password was hashed and stored without complaint via a direct API call —
  the 6-character floor only existed in the frontend form — and there was
  no upper bound at all, so a megabyte-long password fed straight into
  Argon2. The endpoint now rejects passwords shorter than 6 or longer
  than 128 characters with a 400, enforced by a tested
  `auth::password::validate_new_password`.

### Security

- **A published placeholder is no longer accepted as `AUTH__JWT_SECRET`.**
  The `.env.example` shipped in every release tarball carried
  `change-this-to-a-long-random-string` uncommented — 35 characters, so it
  cleared the 32-character rule added in 0.1.40 and the server started
  normally. An operator who copied the template, which is what the template
  tells them to do, was signing session tokens with a key published on
  GitHub: anyone could forge a token for user 1, the seeded administrator,
  and the database re-check added in this same release would then look that
  row up and hand back its real admin role.

  Startup now refuses every value this project has ever printed in a
  template or a document, whatever its length, and the shipped template
  ships empty.

  **Upgrading:** an installation still running on the template value will
  refuse to start until a real secret is set (`openssl rand -hex 32`). That
  is deliberate — it was forgeable — but check `.env` before rolling this
  out. Replacing the secret logs everyone out; if it was the placeholder,
  that is the point.

- **A question now has a maximum length, and the embedder truncates
  regardless.** The only limit was axum's 2 MB default body: a question
  that size was tokenised whole, embedded on the CPU, and pasted into the
  prompt, so any authenticated account — including a plain `user`, who
  cannot upload anything — could spend minutes of CPU and gigabytes of
  memory per request, repeatedly. Questions over 4000 characters are now
  refused with a 400 before any of that happens, and the tokenizer is
  configured with bge-m3's own 8192-position limit at load time, so the
  bound holds for every caller rather than only this one endpoint.

- **DOCX and XLSX are refused when they declare more than 512 MiB
  uncompressed.** Both formats are zip archives that the parsers inflate
  into memory in one go, and the upload limit only ever bounded the
  *compressed* file — a thousand-to-one ratio is trivial, so a few
  kilobytes on the wire could become gigabytes of resident memory, fatal on
  the ARM64 boards this project ships builds for. The check reads the
  archive's central directory only, so nothing is decompressed to run it.
  It stops the ordinary bomb, which declares its real size; an archive that
  lies about its sizes needs the decompression itself to run through a
  capped reader, and that is left for its own change.

- **A token's claims are now re-checked against the database on every
  request.** The role and identity were taken from the JWT and believed as
  written, so deactivating a user, demoting an administrator, or changing a
  leaked password had *no effect* until the token expired on its own —
  eight hours, by default. The role is now read from the row, and the
  lookup already filters `is_active = 1`, so a deactivated account stops
  working on its next request. Costs one SQLite primary-key lookup per
  authenticated request. Tokens issued before a password change still
  survive until expiry; closing that needs a token-version column and is
  left for its own change.

- **`AUTH__ADMIN_DEFAULT_PASSWORD` no longer overwrites an existing admin
  password on every start.** It rewrote the stored hash at *every* startup,
  which meant an installation carrying that variable could never really
  change its admin password — the value in `.env` silently won again at the
  next restart, even after the admin had set a new one from the UI. And
  because the only check was "not empty", `=x` produced a one-character
  administrator, reinstated at every boot. It now seeds a fresh install
  only, is validated against the ordinary password policy, and is ignored
  with a warning once the account exists.

  **Upgrading:** if you relied on that variable to reset the password, use
  the new `AUTH__ADMIN_RESET_PASSWORD` — it does the same thing on purpose,
  says so loudly in the log, and should be unset again afterwards.

- **The API no longer answers cross-origin requests from anywhere.** It
  attached `CorsLayer::permissive()` — `Access-Control-Allow-Origin: *`
  with every method and header — to a binary that serves its own frontend
  from its own origin, so the layer permitted everything and protected
  nothing: any page on the internet could call this API from a visitor's
  browser and read the answers. There is now no CORS layer at all unless
  `SERVER__CORS_ORIGINS` names the origins to allow, which is only needed
  for a dev server on another port.

- **Every response now carries security headers.** A Content Security
  Policy (`script-src 'self'`, `frame-ancestors 'none'`), `nosniff`, and
  `Referrer-Policy: same-origin` — none of which were set, on a binary
  whose entire surface is an administrative UI. The policy admits
  `'unsafe-inline'` for styles alone, because the upload progress bar sets
  its width through a style attribute; script-src is not relaxed to buy
  that.

- **Server errors no longer hand their internals to the caller.** A 5xx
  returned the `anyhow` chain verbatim — filesystem paths, the Qdrant URL,
  SQL text, the body of an eullm reply — which is free reconnaissance for
  anyone who can provoke one. The detail now goes to the log, where it is
  useful, and the response says only that something broke. Client errors
  are unchanged: a 4xx still explains what the caller got wrong.

- **CI now runs on pull requests, and checks that the committed frontend
  bundle matches its source.** The workflow only triggered on pushes to
  `main`, so an external contributor's code was validated *after* it had
  already landed — the wrong order, and one that only worked this month
  because every such pull request was compiled and tested by hand. The
  bundle check exists for the same reason: `frontend/dist` is committed,
  so a pull request carries a minified file built on someone else's
  machine that nobody can review by reading it. CI rebuilds it and fails
  on any difference. First-time contributors from a fork still need a
  maintainer to approve the run, which is the correct default and is left
  alone.

- **Repeated failed logins against one account now earn a growing delay,
  and only a few password verifications run at once.**
  `POST /api/auth/login` accepted unlimited attempts and paid an Argon2id
  verification for each one, which made two things cheap that should not
  be: brute-forcing `admin` — an account that exists on every install,
  under a name everyone knows — and saturating the CPU that also has to
  serve the language model, without any credentials at all.

  After three free failures a username's next attempt waits one second,
  then two, four, and so on up to thirty. It is a delay and not a lockout
  on purpose: a lockout would let anyone deny an account to its owner just
  by failing against it. A successful login clears the count, and one
  account's failures never delay another's.

  Independently, at most four verifications run concurrently; anything
  past that is refused with `429` *before* hashing, so the cost of
  rejecting an attack does not scale with the attack.

  Neither mechanism reads the client's IP, deliberately — behind the
  reverse proxy this README now recommends, every request shares one
  address, and a per-IP limit would let one person's typo lock out an
  entire organisation. See the module documentation in
  `src/auth/throttle.rs` for the full reasoning.

- **`cargo audit` is clean: twelve advisories down to zero.** The remaining
  `quick-xml` copies went with `docx-rs` 0.4.22, `pdf_oxide` 0.3.77 and
  `calamine` 0.36.1, all of which now use the patched 0.41 — closing
  RUSTSEC-2026-0194 and RUSTSEC-2026-0195, the quadratic-parse and
  unbounded-allocation flaws reachable through any DOCX or XLSX a user
  uploads. `crossbeam-epoch` went to 0.9.21 (RUSTSEC-2026-0204).

  What is left is `rsa` (RUSTSEC-2023-0071, no fix published), which
  reaches the lockfile through sqlx's MySQL driver — a backend this project
  does not build, so the crate is never linked into the binary.
  `.cargo/audit.toml` records that with the reasoning, rather than leaving
  a permanent red line nobody reads. Seven "unmaintained" advisories remain
  on transitive crates; none has an action.

- **`h2` updated to 0.4.19** (RUSTSEC-2026-0258: unbounded empty DATA
  frames). It reaches this server through hyper under axum, and axum 0.7
  accepts cleartext HTTP/2 with prior knowledge, so the denial of service
  was reachable before any authentication ran. A patch bump; no code
  changed.

---

## [0.1.40] - 2026-09-11

### Changed

- **Upgrading to this release can stop an existing deployment from
  starting, and that is deliberate.** The `AUTH__JWT_SECRET` validation
  below is fail-closed: any installation currently running with a secret
  shorter than 32 characters will refuse to start until that secret is
  replaced (`openssl rand -hex 32`). Such a deployment was already
  forgeable, so refusing to start is the correct outcome — but it is a
  breaking change on upgrade, so check the secret before rolling this out
  rather than discovering it at the next restart. Note that replacing the
  secret invalidates every token in circulation: all users have to log in
  again.

### Fixed

- **Startup now rejects a weak `AUTH__JWT_SECRET` instead of signing tokens
  with it.** Any secret shorter than 32 characters (and a zero or
  overflowing `AUTH__JWT_EXPIRY_MINUTES`) fails fast in `Settings::load`
  with instructions (`openssl rand -hex 32`); previously an empty or
  single-character secret started normally, producing HS256 tokens anyone
  could brute-force and forge. `jwt::create_token` also returns an error
  instead of panicking/wrapping on an out-of-range expiry.

---

## [0.1.38] - 2026-08-22

### Fixed

- **Scrolling up to read earlier messages while a reply was still
  streaming was effectively impossible.** The auto-scroll effect ran
  unconditionally on every `messages` update — and streaming appends a
  token to `messages` many times a second — so any manual scroll up got
  yanked back to the bottom on the very next token. Now tracks whether
  the user is within 100px of the bottom (via an `onScroll` handler) and
  only auto-scrolls when they already were; sending your own message
  always scrolls, regardless of prior position.

### Changed

- **`bench::run()`'s chunk enrichment is now pluggable, via the same
  `ChunkEnricher` a real ingestion uses.** `--bench <file>` used to
  always call `chunker::inject_heading_context` directly, regardless of
  what a caller's `ExtensionRegistry` actually registered — meaning a
  Pro launcher's `--bench` could never measure its own real enricher
  (e.g. Contextual Retrieval), only Community's default. `run()` and the
  internal `run_ingestion()` now take a `&dyn ChunkEnricher` parameter;
  `run_with_extensions` passes `extensions.chunk_enricher.as_ref()` —
  already in scope exactly where `--bench` dispatches. This binary's own
  behavior is unchanged (`ExtensionRegistry::default()`'s enricher wraps
  the same heading-injection as before); a Pro build now gets a
  `--bench` that genuinely exercises whatever it registered. Needed for
  `rag-enterprise-pro`'s Phase 7 (`I3K_RAG_Pro_Open_Core_Architecture.md`,
  private repo, section 24): comparing Community baseline vs. Pro
  baseline vs. Pro+Contextual Retrieval needs the same tool measuring
  the real thing in each case, not three different measurements of the
  same hardcoded default. 121 tests pass, clippy stays clean.

- **`BRANDING.clientName`/`.version` are now build-time configurable**
  (`VITE_BRANDING_NAME`/`VITE_BRANDING_VERSION`, defaulting to this
  repo's own `'i3k RAG Engine'`/`'Community'` — building without them
  set is unchanged). The version badge — previously shown only in small
  print in the app's footer — now also appears next to the product name
  at login and in the main header. Added so a downstream build (e.g.
  `rag-enterprise-pro`'s release CI, which builds this exact frontend
  from the pinned Community rev) can show its own edition without
  forking this file — not proprietary logic, just a configurable label,
  same principle as `extensions::`'s generic hooks.

---

## [0.1.37] - 2026-08-22

### Changed

- **This crate is now also a library, not just a binary.** New
  `src/lib.rs` holds every module (now `pub mod`) and a new
  `pub async fn run()` containing the full startup sequence that used
  to live in `main()`. `src/main.rs` is now a thin launcher:
  `i3k_rag_engine::run().await`. Zero behavior change — same 113 tests
  pass, clippy stays clean, `--version` verified end-to-end through the
  new wiring. This is Phase 1 of the open-core migration: it makes this
  crate a reusable dependency for `I3K-IT/rag-enterprise-pro`, pinned to
  an exact commit, per that repository's
  `I3K_RAG_Pro_Open_Core_Architecture.md` (private — not this repo).

- **New `extensions::` module: generic extension points for a Pro
  binary to register against.** Six traits (`ChunkEnricher`,
  `StructuredKnowledgeProvider`, `QueryPlanner`, `RetrievalStrategy`,
  `Reranker`, `EvidenceLayer`), bundled into `ExtensionRegistry` and
  threaded through `AppState`. `ExtensionRegistry::default()` — what
  this binary always uses — wraps only Community's own existing
  behavior (e.g. the chunk enricher default is the already-shipped
  heading-injection logic, proven byte-for-byte identical to calling it
  directly via a new test; the query planner default always routes
  `Semantic`, Community's one real retrieval path today) or is a
  genuine no-op where Community has no such feature yet (reranking,
  evidence, structured knowledge). `api::router` also now takes an
  optional second `pro_router` parameter, merged in when present, so a
  Pro launcher can add its own routes against the same `AppState`
  without this crate depending on Pro's code. No proprietary logic
  lives here — see this repo's own rule on that. 115 tests pass (two
  new), clippy stays clean. This is Phase 2 of the open-core migration;
  see Phase 1's changelog entry above for context.

- **New `build_info() -> (&str, &str)`, exposing the version/commit
  `--version` already printed.** No behavior change for this binary —
  it's the same two values, now also reachable as a function. Added so
  `rag-enterprise-pro`'s Pro launcher, which statically links this crate
  via a pinned Git dependency (Phase 4 of the open-core migration), can
  read Community's own version/commit at runtime for its own
  `--version` output, without re-deriving it. 116 tests pass (one new),
  clippy stays clean.

- **New `ChunkPayload.retrieval_text`, and a real (if small) citation
  behavior change: stored/cited chunk text is now always the original,
  un-enriched text, never whatever a `ChunkEnricher` produced.** Until
  now, `text` (used both for citations shown to the user and for the
  context fed to the answering LLM) was whatever the enricher returned —
  for Community's own heading-injection, a real but easy-to-miss
  side-effect: a citation could show an injected `[Article 42 — Title]`
  prefix that isn't verbatim what's at that exact position in the
  source. `text` is now always the chunk's real content; the enricher's
  output, when it differs, is stored separately as `retrieval_text` and
  used only to build the LLM's context (`api/query.rs`) — never shown to
  a user as if it were the source. `chunk_size` now matches `text`'s own
  length accordingly. `retrieval_text` is optional and
  `skip_serializing_if`, so points written before this field existed
  keep deserializing unchanged. Required core change for
  `rag-enterprise-pro`'s upcoming Contextual Retrieval (Phase 6): an
  LLM-generated context blurb is not "real document content" the way an
  injected heading is, so this distinction matters far more there — see
  that repository's `I3K_RAG_Pro_Open_Core_Architecture.md` (private)
  section 14, and section 18 on why this core change comes from
  Community first. 121 tests pass (five new), clippy stays clean.

- **New `EullmClient::model_id() -> &str`.** The configured model name
  was write-only (set in the constructor, never read back). Needed so
  `rag-enterprise-pro`'s Contextual Retrieval cache key — which must
  include `model_id` per the architecture document's section 15 — can
  read it from the same client instance doing the generation, instead
  of duplicating the configured model name as a second source of truth.

---

## [0.1.36] - 2026-08-21

### Changed

- **Chat column is now centered and wider.** The messages area and input
  bar had no max-width/centering wrapper of their own — only individual
  message bubbles capped at `max-w-3xl` — so on a wide screen bubbles sat
  flush against the far left/right edges of the whole `<main>` panel
  instead of reading as one centered column. Answers on documents like
  the EU AI Act run long, and the narrow 3xl cap meant a lot of line
  wrapping — i.e. more vertical scrolling than the actual text length
  warranted. Both the messages list and the input form are now wrapped in
  a centered `max-w-5xl` column, and bubbles widened from `max-w-3xl` to
  `max-w-[85%]` of that column.

---

## [0.1.35] - 2026-08-21

### Fixed

- **Ingestion crashed on real-world PDFs containing curly quotes, accents
  or em dashes.** `chunker::is_heading_line` (new in the heading-context
  feature above) sliced `line[..prefix.len()]` — a raw BYTE offset —
  which panics the instant a multi-byte character straddles that offset,
  taking the whole ingestion request down with it
  (`thread 'tokio-rt-worker' panicked ... is not a char boundary`).
  Reproduced on the real EU AI Act PDF within hours of shipping the
  heading-context feature. Rewritten to compare characters one at a time
  instead of byte-slicing — panic-free by construction regardless of
  content. New exhaustive test sweeps every prefix length × lead-in
  length combination with a multi-byte character landing at each
  possible offset, not just the one that happened to crash first.

- **Deleting a conversation with any messages in it always failed** with
  `FOREIGN KEY constraint failed (code 787)`. `chat_messages.conversation_id`
  has a (non-cascading) FK on `conversations(id)`
  (migrations/0002_conversations.sql) and sqlx enables
  `PRAGMA foreign_keys = ON` by default; `delete_conversation` deleted the
  parent row before its messages, which SQLite has always correctly
  refused. Fixed by deleting children before parent, within the same
  transaction. Both statements already filtered by `user_id` independently
  (not just `conv_id`), so reordering doesn't weaken the existing IDOR
  protection — see the updated doc comment on `delete_conversation`.

- **README quick-start couldn't actually be followed as written.** The
  release tarball is built as `tar czf ... -C stage .` — flat, no
  wrapping version-named folder inside it — but the quick-start snippet
  told users to `cd i3k-rag-engine-vX.Y.Z-linux-x86_64` right after
  `tar -xzf`, a directory that plain extraction never creates. Fixed to
  `mkdir` first. Also documented (new "Upgrading" section) that
  extracting a new release's tarball *over* an existing install reuses
  every already-downloaded component — `bootstrap::ensure_component`
  only fetches what's missing or sha256-mismatched, and `DATA__DIR`
  defaults to the binary's own directory — so this isn't a special
  workflow, just how the existing idempotent provisioning already
  behaves when pointed at a non-empty directory.

---

## [0.1.34] - 2026-08-21

### Added

- **Sources card now shows the page number(s) a chunk came from.** The
  API response has carried `page_start`/`page_end` since the Source
  Provenance Foundation work, but the frontend never rendered them —
  the sources list under an answer only showed filename + similarity
  %. `frontend/src/App.jsx` now adds a "pag. N" (or "pag. N–M" when a
  chunk spans multiple pages) badge next to each source, when the
  document has page info (non-PDF formats and documents ingested
  before this feature existed still have none, so the badge is
  omitted for those, not shown as blank/zero).

- **Chunks now carry the nearest preceding structural heading** ("Article
  99", "Chapter XII", "Section 4", ...) when they don't already contain
  it themselves. Root cause: chunk boundaries are byte-count-driven and
  know nothing about document structure, so a chunk can land entirely
  inside e.g. Article 100's body without the "Article 100" heading —
  which landed a chunk or two earlier. Retrieval then hands the LLM a
  fragment like "...administrative fines of up to EUR 1 500 000" with no
  indication of which article it belongs to. This is confirmed as the
  actual cause of two independently-built RAG stacks (this one and the
  old Python one) both misattributing the AI Act's Article 99/100/101
  penalty clauses to the wrong article on the same questions — verified
  against the real Official Journal PDF text, not assumed. New
  `chunker::detect_headings`/`inject_heading_context`: only touches the
  text that gets embedded/stored, not `Chunk.start_byte`/`end_byte` —
  page numbers and citation spans are unaffected. `CHUNKING_CONFIG_VERSION`
  bumped 1 → 2 (same chunk_index now stores different text than before,
  so re-ingesting must not collide with old provenance_ids).

---

## [0.1.33] - 2026-08-20

### Fixed (yet again)

- **`EULLM__MODEL_OVERRIDE` didn't stop the manifest-pinned qwen3-14b
  (8.4GB) from downloading anyway.** Same shape of bug as the bge-m3
  gguf one above: `select_components()` always selects "qwen3-14b" —
  a manifest model with no target, universal like every other model
  component — regardless of Settings, but `start_eullm` only ever
  falls back to it when `model_override` is unset. Once an override
  is set — a local path already on the machine, or a URL eullm
  fetches itself on `eullm run` — that download was never going to be
  read by anything. New `bootstrap::drop_unused_chat_model`, applied
  the same way as `drop_unused_embedding_model`, skips it whenever
  `EULLM__MODEL_OVERRIDE` is set, either form.

### Fixed (again)

- **bge-m3's GGUF was never going to resolve for eullm.** The manifest
  used to pin it as a component this binary downloaded directly via
  HTTP into eullm's model-store directory — but eullm's own model
  resolution (`resolve_model`) never downloads anything at request
  time; it only reads its store, mount points, or an explicit path
  (opt-in only). Removed `bge-m3-gguf` from manifest.toml entirely.
  It is now provisioned the correct way: `bootstrap::start_eullm`
  calls `eullm pull <url>` itself, once, the first time
  `ingestion_embedding=eullm` runs and the file isn't already
  present — idempotent and offline-safe like every other component,
  sha256-verified independently since a URL pull doesn't verify
  itself. Also fixes `config::EULLM_EMBEDDING_MODEL`, which was
  `"bge-m3"`: running a real pull against a real eullm 0.6.90 showed
  it actually registers as `"bge-m3-f16"` (derived from the pulled
  file's own name) — every `/api/embed` call under the old constant
  would have 404'd.

### Security

- **Qdrant no longer listens on all network interfaces.** The bundled
  qdrant binary defaults to `0.0.0.0` when its host isn't set —
  verified by running the actual pinned binary and reading its own
  startup log — and this project has no API key concept for it at
  all, so on any host where ports 6333/6334 weren't independently
  firewalled, the full REST+gRPC API (read, write, and delete every
  ingested document, completely unauthenticated) was reachable by
  anyone who could reach those ports, bypassing this binary's own JWT
  auth entirely. Now bound to `127.0.0.1` explicitly — this binary
  only ever talks to qdrant over localhost anyway, so nothing
  functional changes. Deployments with `DATA__MANAGE_SUBPROCESSES=false`
  (qdrant run externally) are unaffected either way — that qdrant's
  bind address was never this project's to control.
  eullm was checked too and needs no equivalent fix: it also listens
  on `0.0.0.0` at the socket level, but enforces its own IP allowlist
  at the application layer, loopback-only by default (`Allowed source
  IPs/subnets: 127.0.0.1/32, ::1/128`, per its own startup log) — a
  deliberate default, not an oversight.

### Fixed

- **Release tarballs did not include `.env.example`.** It existed in
  the repository since 0.1.32 but the packaging step never copied it
  into `stage/` alongside the binary, so anyone who downloaded a
  release instead of cloning the repo had no configuration template
  at all. Added to all three platform tarballs.

### Changed

- **`EMBEDDINGS__INGESTION_EMBEDDING=eullm` now also covers query-time
  embedding, not just ingestion.** Previously this mode only routed
  document-ingestion embedding through eullm's `POST /api/embed`;
  every question was still embedded through the in-process Candle
  instance regardless. Now both go through eullm, and Candle is not
  loaded at startup at all in this mode — not even on CPU — so
  bootstrap no longer downloads its ~2.1GB of bge-m3 weights either
  (only the ~1.1GB GGUF eullm itself uses). `Off` and `CandleGpu` are
  unaffected: query embedding still always uses Candle for those two,
  same as before. Worth knowing before enabling: on a card where
  bge-m3 and the chat model do not both fit in VRAM, every query now
  pays a potential model-swap round trip (evict chat to embed the
  question, evict bge-m3 back out to answer it) — see the doc comment
  on `config::IngestionEmbedding::Eullm`.

## [0.1.32] - 2026-08-20

### Fixed

- **`Settings::load()` rejected the minimal `.env` README.md has
  documented since 0.1.28.** Every field of `EullmSettings` already
  had its own default, but the `Settings.eullm` field itself had no
  `#[serde(default)]` and `EullmSettings` had no `Default` impl — with
  zero `EULLM__*` variables set, config loading failed outright with
  "missing field `eullm`" before ever reaching the individual fields'
  defaults. Verified against the actual compiled binary, not just the
  struct definitions: running it with only `AUTH__JWT_SECRET` set
  reproduced the failure before the fix, and reached the real
  bootstrap/download flow after it. Every other documented `.env`
  example (`BUILD.md`) happened to already set `EULLM__URL`/
  `EULLM__MODEL` explicitly, which is why this went unnoticed.

### Added

- `EULLM__REPEAT_LAST_N` — how many recent tokens `repeat_penalty`
  looks back over, previously hardcoded to 256 with no way to change
  it short of a rebuild. Default unchanged (256).
- `.env.example` at the repo root, documenting every `SECTION__FIELD`
  variable `Settings::load()` reads.

## [0.1.31] - 2026-08-20

### Added

- **Source Provenance Foundation.** Every retrieved chunk now carries a
  byte-offset span (`source_start`/`source_end`) into its source
  document, a PDF page span (`page_start`/`page_end`, for both the
  native `pdf_oxide` path and the OCR fallback), and a deterministic
  `provenance_id` anchored to the uploaded file's own sha256 — stable
  across re-ingestion of an unchanged file, and versioned so a later
  chunking/extraction config change produces visibly distinct ids
  instead of silent collisions. Infrastructural only: this is not
  claim-level (sentence) attribution, source highlighting, or
  NLI-based verification — those stay roadmap items. All new fields
  are optional; points written before this change deserialize them as
  absent, no re-ingestion required. See `rag::chunker::provenance_id`
  and `documents::parser::PageSpan`.

### Fixed

- **eullm never started on a GPU-less Linux x86_64 host.** The
  manifest's only `linux-x86_64` eullm pin required CUDA; a machine
  without an NVIDIA GPU found no usable target and bootstrap silently
  fell back to no LLM. Added the missing CPU-only entry (mirrors the
  ARM64 fix from 0.1.28).
- `--embedding-model` is now gated behind a separate
  `EullmSettings::reserve_embedding_model` flag (default off) instead
  of firing automatically whenever ingestion embedding runs through
  eullm — on a tight-VRAM card it was reserving space for bge-m3
  before `--fit` sized the chat model, starving it. Deployments with
  headroom for both can opt in explicitly.

### Changed

- CI no longer builds or publishes the Candle-CUDA Linux release
  variants (`release-linux-{x86_64,arm64}-cuda`): eullm's own bundled
  CUDA already covers GPU-accelerated embedding
  (`/api/embed` since 0.6.82, `--embedding-model` since 0.6.90), making
  a second CUDA toolchain compiled into this binary redundant. The
  source (`--features cuda`) stays buildable by hand; this only stops
  CI from shipping it. Every platform now ships one CPU-only tarball,
  matching how Windows was already described — a GPU is still used
  automatically wherever eullm detects one at runtime, regardless of
  how this binary itself was compiled.
- `manifest.toml` also pins the `eullm-linux-x64-vulkan` asset
  (sha256/size verified against the real downloaded binary). Not yet
  selectable — no Vulkan-GPU detection exists in `current_targets()`
  — pinned in advance so the entry is ready once that detection is
  written.

## [0.1.30] - 2026-08-18

### Added

- **Document-ingestion embedding through eullm.** `EMBEDDINGS__INGESTION_EMBEDDING=eullm`
  routes the embedding step of document ingestion through eullm's own
  `POST /api/embed` (eullm ≥ 0.6.82) instead of the in-process Candle path —
  an alternative for GPU-accelerated embedding that does not need this
  binary itself compiled with `--features cuda`, since eullm manages its own
  device placement. Query-time embedding is unaffected either way: it always
  runs through the resident Candle instance, a single short text per
  request. Off by default; existing deployments see no change unless this is
  set. See `config::IngestionEmbedding` for the two other explicit modes
  (`candle_gpu`, the pre-existing CPU↔GPU swap; `off`, unchanged).

  With eullm ≥ 0.6.90, `bootstrap::spawn_eullm` also passes
  `--embedding-model` at startup, so bge-m3 loads as a reserved companion
  next to the chat model when there is room for both — decided once, inside
  eullm, instead of gambled on which of two independently-started processes
  happened to claim VRAM first.

### Changed

- `manifest.toml`'s eullm fleet moves to 0.6.90 (all six pinned targets).

## [0.1.28] - 2026-08-17

### Added

- **Native Windows x86_64 support**, CPU and CUDA. `i3k-rag-engine.exe`
  cross-compiles from a Linux host (no Windows runner needed) — see
  BUILD.md's "Windows (cross-compiled from Linux, CPU only)". OCR
  (Tesseract 5.5.0 + Leptonica 1.85.0) is built statically for the release
  tarball, no runtime mingw dependency; the engine binary itself still needs
  `libstdc++-6.dll` bundled alongside it, a dependency Tesseract's own build
  does not have. Embedding is CPU-only on this platform (Candle's CUDA
  support is not cross-compiled for Windows); eullm — a separate process —
  still uses the GPU when one is present, so chat inference is accelerated
  regardless.
- OCR no longer needs a build-time `--features ocr` flag: `libtesseract` and
  `libleptonica` load at runtime through `libloading`, from an explicit path
  next to the executable, the same pattern already used for `libpdfium`. It
  is always compiled in now.

---

## [0.1.27] - 2026-08-14

### Added

- **Restore.** `POST /api/admin/backup/restore` puts an archive back, as the
  exact inverse of the backup that produced it: the Qdrant snapshot is uploaded
  with `priority=snapshot` so the archived vectors win, and every table the
  archive shares with the live schema is replaced inside one transaction. Until
  now a backup could be taken and listed but never used, which made the whole
  feature ornamental.

  Details worth knowing before running it:

  - It **replaces**, it does not merge. Rows created after the backup are gone.
  - Qdrant is restored first. If that fails nothing else is touched, so a failed
    restore leaves the installation as it was rather than stranding fresh
    metadata against old vectors.
  - An archive older than the current schema still restores: tables that no
    longer exist are skipped, and columns added by a later migration keep their
    default.
  - `_sqlx_migrations` is never copied back — the schema belongs to the binary
    that is running, not to the archive.
  - The response reports what was actually restored, because an archive taken
    while Qdrant was unreachable contains no snapshot.

  Verified end to end against a running Qdrant 1.18.2, not only in unit tests:
  seed a collection, back it up, add a point and a row, restore, and check that
  both additions are gone and the archived state is back. That test is
  `#[ignore]`d by default and runs with
  `QDRANT_URL_FOR_TEST=… cargo test qdrant_round_trip -- --ignored`.

- **Backups are verified, twice.** A backup nobody has checked is a guess, and
  the guess is only tested on the day it has to work.

  When the archive is written: the SQLite copy is reopened and put through
  `PRAGMA integrity_check`, and the Qdrant snapshot is checked against the size
  and sha256 Qdrant itself reports for it — a truncated download used to be
  written out silently. Each member's digest goes into a `backup.json` inside
  the archive.

  When it is restored: every member is checked against that manifest **before
  anything is written**. A damaged archive stops there, with the installation
  untouched, instead of being discovered halfway through.

  Two failures are now told apart. If Qdrant answers with a snapshot that does
  not match what it says it made, the backup fails outright — a verifiably
  broken archive must never reach the backup directory. If Qdrant does not
  answer at all, the archive is still written, without vectors, and says so:
  losing today's copy of the users and the document metadata as well would be
  worse, and the gap is recorded rather than hidden.

  Archives written by 0.1.25 and 0.1.26 carry no manifest. They still restore,
  and the response reports `verified: false` so it is clear they could not be
  checked.

### Fixed

- The Qdrant snapshot was downloaded and written with nothing verified at all.
  A truncated HTTP body is not an error — it just leaves a shorter file — so a
  half-downloaded snapshot was archived as though it were sound.
- `backup/mod.rs` described the archive as including an "optional rclone
  upload". There is no rclone anywhere in this codebase and never was; backups
  are local files and nothing is uploaded anywhere.

---

## [0.1.26] - 2026-08-13

### Changed

- **The first run downloads 5 GB less** — `qwen3-8b` was pinned in
  `manifest.toml` but referenced nowhere in the code, left over from a pipeline
  this engine does not have. The first run goes from ~17.3 GB to ~12.3 GB. Every
  component is still verified against a pinned sha256 before use. An existing
  installation can delete `models/qwen3-8b/` to reclaim the space.
- `EULLM__MODEL` now defaults to `qwen3-14b` and is no longer required. Starting
  without it used to fail, for a setting the default configuration then ignores:
  when the engine launches eullm itself it passes the GGUF path from the
  manifest. It still applies if you run eullm separately and point the engine at
  it. `AUTH__JWT_SECRET` is now the only setting with no default.
- Remaining Italian text translated to English: the 503 returned while a document
  is being ingested, the errors reported when the first run cannot download or
  verify a component, and the stage labels of the `--bench` report.
- README: documents what actually changed between the `1.x` Python stack and
  this one, and states that the two cannot run side by side — they contend for
  ports 8000, 6333 and 11434, so the Compose stack must be stopped first. The
  `1.x` volumes are left in place, so going back remains possible.

### Fixed

- `Cargo.toml` declared `MIT OR Apache-2.0`; this project is Apache-2.0, as
  `LICENSE` has always said.
- The README still advertised a 17 GB first-run download after the manifest had
  changed, and still listed `EULLM__MODEL` as required after it had been given a
  default.

### Removed

- The `license/` module — dead code (`check_page_limit` was never called) that
  described a commercial gating model not implemented here, and pulled in
  `ed25519-dalek` for nothing.
- Source comments referencing files and internal documents absent from this
  repository, some of which described unreleased plans. Three stale `TODO`
  markers on code that has been complete and working for a long time.

### Notes

No changes to retrieval, ingestion, chunking, embeddings or the HTTP API.
Upgrading is a matter of replacing the binary; data and Qdrant collections are
untouched.

---

## [0.1.25] - 2026-08-13

First public release of the Rust rewrite. The system is now a **single binary**
with no Docker, no Compose and no Java: on first run it downloads and
sha256-verifies everything it needs — vector database, inference engine, models,
OCR data — and supervises those processes itself. After that it needs no network
at all.

### Added

- Single self-contained binary for Linux x86\_64 and arm64, with and without
  CUDA, each published as a release tarball.
- Component bootstrap driven by `manifest.toml`: every download is pinned by
  sha256 and size, and a mismatch aborts the run. Platform variants are selected
  at runtime, including a CIX P1 build of the inference engine gated on actual
  CPU feature detection rather than on the SoC name.
- Embeddings in-process via [Candle](https://github.com/huggingface/candle)
  (BAAI/bge-m3, 1024 dimensions), on GPU or CPU, with
  `EMBEDDINGS__REQUIRE_GPU` to refuse to start rather than silently degrade to a
  much slower CPU path.
- LLM inference through [eullm](https://github.com/eullm/eullm) as a supervised
  subprocess, with VRAM handed back and forth around ingestion.
- Document ingestion for PDF (including scanned pages via OCR), DOCX, XLSX, HTML,
  TXT, Markdown and CSV. Scanned pages are detected automatically and passed
  through Tesseract in Italian and English.
- JWT authentication with three roles — user, super user, admin.
- Streaming answers over SSE, with conversation history carried into follow-ups.
- Scheduled backups of the SQLite database and the Qdrant collection together, so
  a restore brings back a consistent pair.
- `--bench` writes a Markdown report timing each stage of ingestion and inference
  on your own hardware and document; `--bench-live` records a whole session
  instead and reports on shutdown.

### Changed

- Apache-2.0, with a full third-party inventory in
  [THIRD\_PARTY\_LICENSES.md](THIRD_PARTY_LICENSES.md).
- The Python stack is preserved unchanged on
  [`python-legacy`](../../tree/python-legacy) and in the `1.x` tags. There is no
  automatic migration path: stop the Compose stack, re-upload the documents, then
  decommission it once satisfied.

---

## [1.2.1] - 2026-05-27

### Added

- `.zenodo.json` with project metadata, keywords, ORCID-linked author and
  licence, enabling automatic DOI generation via [Zenodo](https://zenodo.org/)
  so the project is citable as a research output. ORCID iD:
  [0009-0003-8613-3065](https://orcid.org/0009-0003-8613-3065).

No functional changes from 1.2.0.

---

## [1.2.0] - 2026-03-02

### Added

- **Multi-GPU support** — NVIDIA (CUDA), AMD (ROCm) and CPU-only modes
  ([#9](https://github.com/I3K-IT/RAG-Enterprise/issues/9))
  - New `GPU_TYPE` setting in `.env` (`nvidia`, `amd`, `cpu`)
  - Docker Compose override files: `docker-compose.nvidia.yml` (NVIDIA CUDA),
    `docker-compose.amd.yml` (AMD ROCm)
  - Setup wizard now asks GPU type and auto-configures the correct Docker images
    and device mappings
  - AMD uses the `ollama/ollama:rocm` image with `/dev/kfd` and `/dev/dri`
    device passthrough
  - CPU-only mode works out of the box with no GPU drivers required

---

## [1.1.5] - 2026-03-01

### Added

- **Automatic model download at startup** — if the configured LLM model is not
  present in Ollama, the backend downloads it showing real-time progress:
  percentage, downloaded/total size, speed and estimated time remaining
- **Ollama readiness check** — the backend waits for Ollama to be reachable
  before proceeding, preventing 404/connection errors on fresh installations

### Fixed

- **Ollama URL now configurable** via `OLLAMA_HOST` and `OLLAMA_PORT` environment
  variables — previously hardcoded to `http://ollama:11434`, which only worked
  inside Docker networking
- Replaced stale `MILVIUS_HOST`/`MILVIUS_PORT` env vars in the Dockerfile with
  correct `OLLAMA_HOST`/`OLLAMA_PORT` defaults

---

## [1.1.0] - 2026-02-27

### Added

- **Backup & Restore system** with full admin panel UI
  - One-click local backup of database, documents and vector store
  - Cloud backup via rclone (70+ providers: Mega, S3, Google Drive, OneDrive,
    Dropbox, WebDAV, FTP, SFTP, B2, pCloud)
  - Automatic scheduled backups with cron expressions and configurable retention
    policies
  - Selective restore (choose which components to restore individually)
  - Cloud provider management with connection testing
  - Backup history tracking (last 100 operations)
  - Download backups from cloud to local storage
- Complete backup documentation (`docs/BACKUP.md`) with setup guides for all
  providers
- rclone pre-installed in the Docker image for cloud storage integration

### Security

- All backup endpoints require admin role authentication
- Cloud provider passwords encrypted via rclone obscure mechanism
- Path traversal protection on archive extraction during restore
- Safe online SQLite backup (no downtime, no data corruption)

---

## [1.0.0] - 2026-02-21

First public release of RAG Enterprise — a 100% local Retrieval-Augmented
Generation system for organisations that need complete data privacy.

### Added

- One-command setup with Docker Compose (`setup.sh`)
- Multi-format document processing (PDF, DOCX, PPTX, XLSX, TXT, MD, ODT, RTF,
  HTML, XML)
- Local LLM inference via Ollama (Qwen3 14B Q4, Mistral 7B Q4)
- Vector search with Qdrant and BAAI/bge-m3 multilingual embeddings
- OCR pipeline for scanned documents (Tesseract + Apache Tika)
- JWT authentication with role-based access control (user / super user / admin)
- Conversational memory per user with session isolation
- GPU acceleration support (NVIDIA CUDA)
- 29-language support for document processing and retrieval
- React + Vite frontend with Tailwind CSS
- Auto-configuration of network and security during setup
- Smart PDF detection and routing (digital vs scanned)
- Benchmark script for performance testing
- Community files: contributing guide, issue templates, PR template, roadmap
- Qdrant API key support for secured deployments

### Security

- Production-ready JWT + CORS configuration
- Conversation isolation between users
- Removal of all hardcoded credentials
- Automatic security configuration during setup

### Performance

- Direct Ollama API client replacing the LangChain wrapper
- Optimised RAG search parameters for large documents
- GPU memory management with automatic CPU fallback
- Tika heap tuning (4 GB) with auto-restart on failure
- Robust timeout and auto-recovery for document processing
- Thread pool execution for document processing, to avoid blocking the event loop
- OOM crash prevention on sequential document uploads

### Fixed

- PyTorch and CUDA compatibility across GPU generations
- Embedding batch size tuning with CUDA fallback
- HTTP enforcement for local Qdrant connections
- Benchmark script authentication and output paths
- PaddleOCR / PyMuPDF dependency conflict resolution

### Changed

- LLM switched from Qwen 2.5 to Qwen3 14B Q4\_K\_M for improved quality
- All Italian text translated to English for international accessibility

---

[Unreleased]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.27...HEAD
[0.1.27]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.26...v0.1.27
[0.1.26]: https://github.com/I3K-IT/RAG-Enterprise/compare/v0.1.25...v0.1.26
[0.1.25]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v0.1.25
[1.2.1]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/v1.2.1
[1.2.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.2.0
[1.1.5]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.1.5
[1.1.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.1.0
[1.0.0]: https://github.com/I3K-IT/RAG-Enterprise/releases/tag/1.0.0
