# TECH.md — `{conversationId}` in provider header values

Product spec: `specs/byop-header-vars/PRODUCT.md`
Researched against `1978f6ec9` (branch `warp-local`, the worktree's base).

## Context

BYOP requests are built in `app/src/ai/agent_providers/`. A provider is resolved from settings,
its model ID is decoded from an `LLMId`, and a genai `Client` is built per request.

Current state at `1978f6ec9`:

- Provider config: `AgentProvider` with `extra_headers: Vec<(String, String)>` —
  `app/src/settings/ai.rs:1415`. Persisted in `settings.toml` as an array of pairs; API keys live
  separately in `AgentProviderSecrets`.
- Provider lookup: `lookup_byop` returns `(AgentProvider, api_key, model_id)` —
  `app/src/ai/agent_providers/mod.rs:127`.
- Main stream: `ByopTarget` carries the provider and task IDs —
  `app/src/ai/blocklist/controller/response_stream.rs:341`; `into_input` builds `ByopOutputInput` —
  `response_stream.rs:354`; the stream is produced by `generate_byop_output` —
  `app/src/ai/agent_providers/chat_stream.rs:3278`.
- Header application today: `build_chat_options` ends with
  `opts.with_extra_headers(extra_headers)` — `chat_stream.rs:3034` (signature) and `3162` (apply).
  genai merges these over the adapter's own headers, so user values win
  (`lib/rust-genai/src/client/headers.rs:32`, `chat/chat_options.rs:190`).
- Conversation identity on a BYOP request: `params.byop_conversation_id: Option<AIConversationId>` —
  `app/src/ai/agent/api.rs:199`, always set from `conversation.id` at `api.rs:498`. The server token
  is a different field (`params.conversation_token`) and is absent on the first turn.
- `AIConversationId` — `crates/ai_types/src/agent.rs:9`, a UUIDv4 wrapper with `Display`.
- One-shots: `OneshotConfig` — `app/src/ai/agent_providers/oneshot.rs:29`; request shaping in
  `build_oneshot_request` — `oneshot.rs:64`; config resolution in `resolve_active_ai_oneshot`
  (`oneshot.rs:171`) and `resolve_next_command_oneshot` (`oneshot.rs:196`). `build_oneshot_request`
  never applies `extra_headers` today, so title generation and active-AI calls send none.
- Title generation: `TitleGenInput` — `chat_stream.rs:3243`; built by `pending_title_generation` —
  `response_stream.rs:249`; executed by `generate_title_via_byop` — `chat_stream.rs:4496`, which
  constructs an `OneshotConfig`.

No genai changes are needed: `Headers` is a `HashMap<String, String>` and `with_extra_headers`
accepts `Vec<(String, String)>` (`lib/rust-genai/src/client/headers.rs:63`,
`lib/rust-genai/src/chat/chat_options.rs:190`).

## Design

Add a pure, string-only resolver module and call it at the two points where headers leave the
settings model. Keep template text in `settings.toml`; resolve per request.

```
settings.toml → AISettings.agent_providers → lookup_byop
                                            ├─ main stream: ByopTarget (params.byop_conversation_id)
                                            │   → ByopOutputInput{extra_headers, conversation_id}
                                            │   → resolve_headers → build_chat_options
                                            └─ title gen: TitleGenInput{extra_headers, conversation_id}
                                                → OneshotConfig → resolve_headers
                                                → build_oneshot_request
→ ChatOptions.extra_headers → genai Headers merge → reqwest
```

Settled decisions this implements (chosen during design review):

- `{conversationId}` is the local `AIConversationId`, not the server conversation token.
- Exact, case-sensitive match; header values only; every occurrence; unknown `{...}` left literal.
- Table-based variable set so more variables are additive; no env expansion; no `base_url`
  templating.
- A row whose known variable has no value is omitted, not sent empty.
- Applied on the main stream and on one-shots; one-shots without a conversation omit the row.
- No feature flag; two commits.
- No UI change; documentation lives in the settings doc comment and README.
- Resolved values are never logged.
- Empty-name and empty-resolved-value rows are skipped.

## Changes

### 1. New module `app/src/ai/agent_providers/header_vars.rs`

Register in `app/src/ai/agent_providers/mod.rs` next to the other internal modules
(`pub(crate) mod header_vars;`).

```rust
use crate::ai::agent::conversation::AIConversationId;

/// Values available to `{...}` placeholders in provider header values.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HeaderVars {
    pub(crate) conversation_id: Option<AIConversationId>,
}

impl HeaderVars {
    pub(crate) fn with_conversation_id(mut self, id: Option<AIConversationId>) -> Self { ... }
}

/// Expands supported placeholders in header values and drops rows that cannot be sent.
pub(crate) fn resolve_headers(
    headers: &[(String, String)],
    vars: &HeaderVars,
) -> Vec<(String, String)>
```

Resolution rules, applied per row in order:

1. Drop the row if `name.trim()` is empty.
2. Scan `value` left to right. On `{`, find the next `}`:
   - inner text exactly `conversationId` → if `vars.conversation_id` is `Some`, append the UUID and
     continue after the `}`; if it is `None`, drop the whole row (missing-value rule).
   - any other inner text, or no `}` → append `{` literally and continue scanning after it.
3. Drop the row if the resulting value is empty.
4. Otherwise emit `(name.clone(), resolved)`.

Notes for the implementer:

- No regex, no `format!`-style parsing: a manual scan keeps literal-brace behavior obvious and
  avoids treating `{{`/`}}` specially.
- Do not trim or otherwise normalize values; only the empty-name check uses `trim()`.
- Do not log anything from this module. No values, no names.
- Duplicate names are left in place; genai's `Headers` map already collapses them last-wins, exactly
  as today.

### 2. Main stream plumbing

`app/src/ai/blocklist/controller/response_stream.rs`:

- Add `conversation_id: Option<AIConversationId>` to `ByopOutputInput`
  (`app/src/ai/agent_providers/chat_stream.rs:3253`).
- In `ByopTarget::into_input` (`response_stream.rs:354`), populate it with
  `params.byop_conversation_id`. `ByopTarget` does not need a new field; `params` is already in
  scope and `AIConversationId` is `Copy`.

`app/src/ai/agent_providers/chat_stream.rs`:

- Add `use crate::ai::agent::conversation::AIConversationId;` (`response_stream.rs:17` already does;
  neither `chat_stream.rs` nor `oneshot.rs` imports it today).
- Destructure `conversation_id` in `generate_byop_output` (`chat_stream.rs:3276`).
- Immediately before the `build_chat_options` call (`chat_stream.rs:3330`), replace the raw headers:

  ```rust
  let extra_headers = super::header_vars::resolve_headers(
      &extra_headers,
      &HeaderVars::default().with_conversation_id(conversation_id),
  );
  ```

- Leave `build_chat_options`'s signature and body unchanged. It is called from roughly ten tests in
  this file that pass `vec![]`; keeping the signature avoids touching them.
- The existing `conversation_id: Option<&str>` parameter of `build_chat_options` is the *server*
  conversation token used for `prompt_cache_key`. Do not repurpose or rename it in this change;
  pass the new local ID only through `extra_headers`.

### 3. One-shot plumbing

`app/src/ai/agent_providers/oneshot.rs`:

- Add the same `AIConversationId` import.
- Add to `OneshotConfig` (`oneshot.rs:31`):

  ```rust
  pub extra_headers: Vec<(String, String)>,
  pub conversation_id: Option<AIConversationId>,
  ```

- In `build_oneshot_request` (`oneshot.rs:64`), after the existing options are built:

  ```rust
  let headers = super::header_vars::resolve_headers(
      &cfg.extra_headers,
      &HeaderVars::default().with_conversation_id(cfg.conversation_id),
  );
  if !headers.is_empty() {
      chat_opts = chat_opts.with_extra_headers(headers);
  }
  ```

- `resolve_active_ai_oneshot` (`oneshot.rs:171`) and `resolve_next_command_oneshot`
  (`oneshot.rs:196`): pass `extra_headers: provider.extra_headers` (the provider is already owned
  and partially moved) and `conversation_id: None`.

`app/src/ai/agent_providers/chat_stream.rs`:

- Add to `TitleGenInput` (`chat_stream.rs:3243`) `conversation_id: Option<AIConversationId>` and
  `extra_headers: Vec<(String, String)>`.
- In `generate_title_via_byop` (`chat_stream.rs:4496`), map both into the `OneshotConfig`.

`app/src/ai/blocklist/controller/response_stream.rs`:

- In `pending_title_generation` (`response_stream.rs:249`), fill the new `TitleGenInput` fields from
  `provider.extra_headers` (the title model's provider, which may differ from the chat provider) and
  `params.byop_conversation_id`.

No changes are needed in `app/src/ai/agent_providers/active_ai/mod.rs`: it consumes `OneshotConfig`
values returned by the resolvers and will simply send static headers with no conversation
placeholder, which is the intended behavior.

### 4. Documentation

- Extend the doc comment on `AgentProvider::extra_headers` (`app/src/settings/ai.rs:1407`) to
  name `{conversationId}`, state that it is case-sensitive and values-only, and state the
  omit-when-unavailable rule. Keep the comment's existing language and tone. This field feeds the
  generated JSON schema, so the note is visible to settings tooling.
- Add a short "Bring your own model provider" section to `README.md` (below "About", before
  "Installation"): where providers are configured (`Settings → Agents → Agent providers`), that
  provider metadata lives in `settings.toml` while API keys live in secure storage, and that header
  values may contain `{conversationId}`. Note that the README is upstream Warp's, so this is a
  deliberate fork-local section — keep it brief.

### 5. Tests

New `app/src/ai/agent_providers/header_vars_tests.rs`, declared at the bottom of `header_vars.rs`:

```rust
#[cfg(test)]
#[path = "header_vars_tests.rs"]
mod tests;
```

Resolver cases:

- `substitutes_conversation_id_exactly`
- `substitutes_every_occurrence_in_a_value`
- `substitutes_next_to_literal_text`
- `leaves_unknown_tokens_literal`
- `leaves_other_casing_literal` (`{conversationid}`, `{conversation_id}`, `{ conversationId }`)
- `never_templates_header_names`
- `omits_row_when_conversation_id_is_missing`
- `keeps_other_rows_when_one_is_omitted`
- `skips_empty_header_names`
- `passes_through_values_without_placeholders`
- `handles_unbalanced_braces_literally`

Request-shaping cases:

- In `chat_stream.rs`'s `cache_boundary_stability_tests` module, add
  `resolved_conversation_header_reaches_chat_options`: resolve headers for a fixed
  `AIConversationId`, call `build_chat_options`, and assert the resulting
  `ChatOptions.extra_headers` contains the UUID value. `ChatOptions.extra_headers` is a public
  `Option<Headers>` field (`lib/rust-genai/src/chat/chat_options.rs:72`) and `Headers::iter()` is
  available for assertions.
- New `app/src/ai/agent_providers/oneshot_tests.rs` (declared inside `oneshot.rs` with the same
  `#[path]` pattern): `oneshot_applies_static_headers`,
  `oneshot_substitutes_conversation_header`, `oneshot_omits_unresolved_conversation_header`.

No integration-test frameworks are involved; these are pure unit tests plus one `ChatOptions`
assertion.

## Commits

1. `feat(ai): resolve {conversationId} in provider header values`
   — `header_vars` module + tests, `ByopOutputInput`/`ByopTarget` plumbing, main-stream call site,
   settings doc comment.
2. `feat(ai): send provider headers on one-shot requests`
   — `OneshotConfig`/`TitleGenInput` fields, `build_oneshot_request` application, resolver plumbing
   in `pending_title_generation` and the two resolvers, one-shot tests, README section.

No feature flag: the Agent providers page is already gated by `FeatureFlag::AgentMode`
(`app/src/settings_view/agent_providers_page.rs:804`), and a BYOP-specific flag would be cleanup
debt. The second commit is the revert boundary if a strict gateway rejects headers on one-shot
calls.

## Verification

Per the repo's implementation validation order (`AGENTS.md`):

```
cargo nextest run -p warp header_vars
cargo nextest run -p warp oneshot
cargo nextest run -p warp chat_options
cargo clippy -p warp --all-targets --tests -- -D warnings
./script/format
```

Manual smoke test: configure a provider whose `base_url` points at an echo endpoint (or a gateway
that reflects request headers), set `x-session-id = "conv-{conversationId}"`, run two conversations,
and confirm the header differs between them, is identical across turns, and survives an app restart.
Then trigger title generation and confirm the same value with the title provider's headers.

## Risks

- **Typos are silent.** `{conversationID}` is sent literally. Accepted in exchange for never
  breaking literal braces; the doc comment names the exact spelling.
- **No escape for a literal `{conversationId}`.** Accepted; nothing in the provider ecosystem sends
  that string today.
- **One-shot behavior change.** Existing users' static headers will start riding title-generation
  and other one-shot calls. Unusual for gateways to reject unknown headers, but it is the reason
  this ships as its own commit.
- **IDs leave the machine.** The conversation UUID is sent to the configured provider when the user
  adds such a header. This is explicit configuration; nothing is sent by default.
- **Fork/child conversations get new IDs**, so routing treats them as new sessions. This is correct
  for affinity but should be documented in the README note.
- **Duplicate header names** still collapse last-wins in genai's map. Unchanged, but worth noting if
  a user tries to send the same name twice.

## Follow-ups (not in this change)

- `{serverConversationId}` as a second variable, for setups that must match Warp's server-side
  token. Uses the same table.
- `{modelId}`, `{providerId}`, `{taskId}` — trivial once the variable table exists.
- Environment-variable expansion (`{env:FOO}`) if rotating gateway tokens become a requirement; it
  needs its own redaction story.
- UI hint or save-time validation if users report token typos.
