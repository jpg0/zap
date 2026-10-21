# PRODUCT.md — `{conversationId}` in provider header values

## Summary

Users who call their own model providers (BYOP) can configure arbitrary extra HTTP headers per
provider. Those headers are literal strings today, so every conversation through a provider sends
the same values. This change lets a header value contain the placeholder `{conversationId}`, which
is replaced at request time with the ID of the agent conversation the request belongs to.

The motivating case is gateways that pin one conversation to one backend or prefix cache — sticky
routing. Affinity has to hold from the very first request of a conversation, and it must not change
between turns.

## Problem

`AgentProvider::extra_headers` is a `Vec<(String, String)>` merged into BYOP requests as-is
(`app/src/settings/ai.rs:1412`, applied in `app/src/ai/agent_providers/chat_stream.rs:3163`). A
gateway that wants "all turns of this conversation go to the same upstream" has no way to express
that: a constant header value either routes everything to one backend or nothing at all.

Warp already has a stable per-conversation identifier locally: `AIConversationId`, a UUIDv4 created
when the conversation is created (`crates/ai_types/src/agent.rs:9`). It is always present on a BYOP
request (`params.byop_conversation_id`, `app/src/ai/agent/api.rs:194`), survives restarts because it
is persisted with the conversation, and is unique per conversation, including forks and child
agents.

The server-assigned conversation token is not a substitute: it comes from
`conversation.server_conversation_token`, so it is absent on the first request of every
conversation and can remain absent forever when the user never talks to Warp's servers.

## Behavior

### Supported placeholder

- The only supported token in this change is `{conversationId}`.
- It is recognized in **header values** only. Header names are always literal.
- Matching is exact and case-sensitive: `{conversationId}` resolves; `{conversationid}`,
  `{conversation_id}` and `{ conversationId }` do not.
- Every occurrence in a value is replaced, including multiple occurrences and occurrences embedded
  in surrounding text (`conv-{conversationId}-a`, `{conversationId}{conversationId}`).
- Any other `{...}` sequence is left exactly as written. There is no escape syntax; a literal
  `{conversationId}` cannot be sent from a templated value.

### Value

`{conversationId}` resolves to the conversation's local `AIConversationId`, rendered as a
hyphenated lowercase UUID. Properties the routing use case depends on:

- Stable for the whole conversation, across turns and across app restarts.
- Present on the first request of a conversation.
- Distinct per conversation. A forked conversation and a child agent orchestration conversation get
  their own ID, so routing treats them as new conversations.
- Never the server conversation token and never a per-turn or per-request value.

### When the value is unavailable

If a request has no conversation ID, a header whose value contains `{conversationId}` is **not
sent**. It is not sent empty, and no fallback value is substituted. Other headers on the same
provider are unaffected.

### Which requests

| Request | Placeholder resolves |
| --- | --- |
| Main agent conversation stream (chat completions, tool calling) | Yes |
| Conversation title generation (one-shot) | Yes — the conversation being titled |
| Prompt suggestions / NLD predict / relevant files / next command (one-shots) | No — these are per-terminal with no conversation, so any `{conversationId}` header is omitted |

Headers come from the provider that serves the request, not from the provider that serves the chat.
Title generation can be configured to use a different provider; that provider's headers and
placeholder resolution apply to the title request. This is also the first change that sends
`extra_headers` on one-shot requests at all: previously title generation and the active-AI one-shots
sent no custom headers.

### Malformed rows

Rows whose header name is empty, or whose value is empty after resolution, are skipped. A
half-filled row created with the settings page's "Add Header" button therefore no longer breaks
every request from that provider.

### Configuration surface

- The settings page is unchanged: no hint, validation, or autocomplete is added. The stored value
  keeps the placeholder text in `settings.toml`.
- The supported token is documented in the doc comment on `AgentProvider::extra_headers` (which also
  populates the generated JSON schema) and in a short README note about bring-your-own-provider
  configuration.
- Resolved header values are never logged. The settings page keeps no new UI, and no new log lines
  are added by this change.

Example configuration:

```toml
[[agent_providers]]
id = "6f2d5f0e-..."
name = "Gateway"
base_url = "https://gateway.example.com/v1/"
api_type = "OpenAi"
extra_headers = [["x-session-id", "conv-{conversationId}"]]
```

A request in conversation `0b0a...` then carries `x-session-id: conv-0b0a...`; a request from a
one-shot with no conversation (a prompt suggestion, say) carries no `x-session-id` at all.

## Goals

- Let a provider header carry the current conversation's local ID.
- Keep affinity stable across turns, restarts, and the first request.
- Behave uniformly on the main stream and on title generation, and predictably omit elsewhere.
- Keep the change small: no new settings fields, no new UI, no schema migration.

## Non-goals

- Environment-variable expansion (`{env:FOO}`) or any other external value source.
- Templating the provider `base_url`, provider name, model IDs, or header names.
- Exposing the server conversation token, task/run ID, model ID, or terminal session ID as
  placeholders. The resolver is table-based so these are cheap to add later; this change ships only
  `{conversationId}`.
- Changing Warp-hosted (non-BYOP) requests.
- Per-turn or per-request identifiers, retry nonces, or signing headers.
- UI discoverability work, save-time validation, or logging of resolved values.
- Making header values secret: they remain plaintext in `settings.toml`, as today.

## Acceptance criteria

1. A provider with `x-session-id = "conv-{conversationId}"` sends a different value for two
   different conversations, the same value on every turn of one conversation (including after
   restarting the app), and the same value on the first request as on later turns.
2. A conversation forked from another gets a new value, not the parent's.
3. `{conversationId}` inside a header name, and any unknown `{token}`, reach the provider literally.
4. A header containing `{conversationId}` is absent from requests that have no conversation
   (prompt suggestions, NLD predict, relevant files, next command).
5. Title generation for a conversation sends the same resolved value as the main stream, using the
   title model's provider configuration.
6. Headers without placeholders keep their current behavior on both the stream and one-shot paths.
7. Rows with an empty name or an empty resolved value are skipped instead of producing a request
   error.
8. No resolved header value appears in logs.

## Validation

Automated: resolver unit tests plus a test that the resolved value reaches the provider request
options; see `TECH.md`.

Manual: point a provider at an echo/mock endpoint that returns request headers, run two
conversations, and confirm the resolved `x-session-id` differs between them and is stable across
turns and restarts.
