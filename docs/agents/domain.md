# Domain Docs

This is a single-context repository. Before exploring or changing an area, read the domain documentation relevant to the work:

- `CONTEXT.md` at the repository root defines Virtui's canonical domain language.
- `docs/adr/` contains system-wide architectural decisions.

If either location is absent, proceed silently. Domain documentation is created lazily when terms or decisions are resolved.

## Use canonical language

Use terms as defined in `CONTEXT.md` in issue titles, implementation plans, tests, and documentation. Avoid synonyms that the glossary explicitly rejects. If a needed domain concept is missing, reconsider whether the new term is necessary or raise it through the domain-modeling workflow.

## Respect architectural decisions

Read ADRs relevant to the area being changed. If proposed work contradicts an ADR, surface the conflict explicitly rather than silently overriding the recorded decision.
