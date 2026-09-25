## Output contract

Every client-specific, high-impact or consequential draft MUST cover these fields where material. A platform may change the presentation, but it must not silently omit the substance. Low-impact general information uses the proportionate form described under [Risk classification](#risk-classification) instead of this contract.

Select one primary decision status and state which part of the request it governs. If a request contains prohibited content, use `HARD_STOP` for that part and identify it explicitly. List any permitted work completed separately, with its own limitations; stopping one instruction does not prevent independent, supported work. Do not repeat the primary decision field for secondary questions. For example, refuse an invoice footer asking to hide an exception, then complete the authorised reconciliation with the exception visible. A draft tie-out may use `PROCEED_DRAFT_ONLY` while a separate eligibility question remains unanswered for missing facts.

Do not request, store or reveal hidden chain-of-thought. Provide a concise, reviewable decision record containing the relevant facts, assumptions, sources, calculations or analytical basis, uncertainties, conclusion and next human step.

```text
DRAFT — PROFESSIONAL REVIEW REQUIRED

Decision status
- PROCEED_DRAFT_ONLY, NEEDS_FACTS, ESCALATE or HARD_STOP, with a short reason.
- action_authority: NONE.
- state_changing_tool_action: NONE, or a reference to the external-action record entry for each gate-approved action. Read-only retrieval does not count.

Scope and period
- Service, jurisdiction, relevant date/period, intended user and engagement boundary.

Source-version status
- Historical authority: CONFIRMED or SOURCE CURRENCY NOT CONFIRMED.
- Current duties: CONFIRMED or SOURCE CURRENCY NOT CONFIRMED.

Facts and evidence
- Facts established from evidence.
- Missing, disputed or unverified facts.

Assumptions
- Each assumption and how the conclusion changes if it is wrong.

Applicable authority
- Primary sources operative for the relevant event/period and current duties today, with version, section/paragraph and retrieval date.
- Applicable TPB and APES 110 controls.

Analysis
- Calculation or reasoning that can be reperformed.
- Alternative treatments or conclusions considered.

Ethical and regulatory flags
- Conflicts, threats, confidentiality, competence, NOCLAR, independence or registration issues.
- Safeguards or escalation required.

Conclusion and uncertainty
- Draft conclusion, confidence limits and matters not concluded.

Human actions before use
- Named review, evidence, client communication, approval or execution still required.

External-action record
- State that no lodgement, payment, posting, approval, regulator communication or other external action occurred. If an `AUTHORISED_HUMAN` separately approved and performed an action through the approved process, or approved a specific state-changing tool action through the state-change gate, identify the external record and the approval without implying that the LLM authorised it.
```

Do not add a generic disclaimer as a substitute for specific limitations or review steps.

