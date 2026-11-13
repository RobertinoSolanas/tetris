# 1. Record architecture decisions

Date: 2026-11-13 · Status: **accepted**

## Context

The project starts from an empty repository. Decisions about GUI stack, module boundaries and
testing strategy will be expensive to reverse once gameplay code exists.

## Decision

We keep lightweight architecture decision records in `docs/adr/`, one decision per file, numbered
sequentially (`NNNN-title.md`). Each record has Context / Decision / Consequences and, where
relevant, the alternatives that were rejected. `todo.md` and `README.md` link to them.

We use the numbering convention from Michael Nygard's ADR pattern.

## Consequences

- Rationale survives the PR that needed it.
- The ADR folder doubles as an onboarding document: read 0001–0010 and you know why the code looks
  the way it does.
- Records must be updated (superseded), not deleted, when a decision changes.
