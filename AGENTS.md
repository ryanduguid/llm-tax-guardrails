# DrDebits agent routing

DrDebits provides versioned guardrails for AI coding assistants (Claude Code, Cursor, Codex, Antigravity) working on Australian taxation and accounting tasks. The guide is normative; this file only routes into it. Where this file and [drdebits.md](./drdebits.md) differ, the guide controls.

## Routing

1. Load [drdebits.md](./drdebits.md) as persistent context and follow its "How to use this file" section.
2. Ground every tax assertion per the guide's Source status and Authority and conflicts sections. Different source types carry different authority; the guide sets out which binds whom.
3. Reperform or verify calculations per the Mandatory workflow. Prefer deterministic computation over generated arithmetic for monetary and statutory figures.
4. Where the guide's Non-negotiable stops, Intake gate or Risk classification call for escalation, stop and hand the matter to the authorised human. Conflict-of-interest, tax-scheme and ambiguous-fact triggers are defined there, not here.
5. Retrieve [reference/tpb-catalogue.md](./reference/tpb-catalogue.md) and [reference/apes-110-map.md](./reference/apes-110-map.md) when a routing decision needs them.
6. Retrieve [reference/ai-vendor-assurance.md](./reference/ai-vendor-assurance.md) when the task assesses an AI product, vendor or deployment before it reaches client work. The guide's AI-specific TPB rules set the boundary; the checklist supplies the questions and the evidence each one needs.
