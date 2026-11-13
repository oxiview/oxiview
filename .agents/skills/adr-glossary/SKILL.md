---
name: adr-glossary
description: Use when deciding whether an ADR is needed, creating or superseding ADRs, maintaining the glossary, or resolving conflicts between user terminology and project definitions.
---

# ADRs and terminology

## Decision records

Apply the [Git conventions](../../../docs/conventions/git-workflow.md): hard to reverse, surprising without context, and involving real trade-offs. Write an ADR only when all three criteria hold; otherwise explain in code or the change description.

1. Read the [ADR index](../../../docs/adr/README.md) and related decisions.
2. Scan all numbered files in `docs/adr/`, increment the highest number, pad to four digits, and create `NNNN-english-short-name.md`. Do not infer the number from the file count or index alone; coordinate numbering when collaborating.
3. Follow the existing `status` frontmatter. Briefly state the decision, context, and trade-offs; add Status / Considered Options / Consequences as needed.
4. When superseding an ADR, preserve its historical rationale, set it to `status: superseded`, and link the new ADR in a Status section. The new record links back and states the scope of replacement; use `status: accepted` once accepted.
5. Update the number, title, link, and dimension in `docs/adr/README.md` in the same change. Run `cargo xtask verify` to check links.

## Glossary

Read [GLOSSARY.md](../../../GLOSSARY.md) first. If the user uses a conflicting term, immediately identify the canonical term and difference in meaning, then continue; do not silently preserve ambiguity.

Add entries under the appropriate group:

```markdown
**English term**:
Define only what it is.
_Avoid_: discouraged terms, easily confused terms
```

Keep implementation details, procedures, and design rationale in architecture documents, conventions, or ADRs. Omit `_Avoid_` when there are no discouraged terms. When renaming, update related documentation and comments while preserving historical ADR rationale.
