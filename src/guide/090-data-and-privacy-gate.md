## Data and privacy gate

The host system must apply this gate before client information reaches the model. Use only an approved model, connector and storage environment. Confirm the engagement purpose, authority, required client permission or other lawful basis, recipients, processing and storage locations, retention, training use, subcontractors and access controls. Supply only the minimum necessary data; remove credentials and unnecessary identifiers, redact TFNs unless essential and specifically approved, and pseudonymise or de-identify information where practical.

Minimise client information in prompts, tool calls, logs, workpapers and outputs. An authorised client deliverable may contain necessary confidential information only within the approved engagement and recipient scope; this does not authorise reuse, secondary disclosure, model training or retention outside that scope.

If client or other protected information has already been received without confirmed authority or through an unapproved system:

1. stop further substantive processing
2. do not echo, transmit, copy or persist the information
3. do not attempt deletion or incident notification through a tool without action-specific approval
4. alert the authorised privacy or security lead through the approved process without repeating unnecessary data
5. follow the organisation’s incident, containment, deletion and retention procedures.

### Automated-decision transparency check

This check was added after release v0.4.0 and is not in that release. From 10 December 2026, APP 1.7 to 1.9 of the *Privacy Act 1988* require an APP entity’s privacy policy to describe certain uses of a computer program in decisions. They apply to decisions made after commencement, including on that day, whether the arrangement, the use of the personal information or its collection happened before or after it ([*Privacy and Other Legislation Amendment Act 2024*](https://www.legislation.gov.au/C2024A00128/asmade) Sch 1 items 88 and 89). Compilation No 104 of the Act, current on 30 October 2026, shows the amendment as uncommenced, so read the amending Act with it until a later compilation incorporates the amendment.

Where the firm has no recorded answers for a tool and its use, raise these questions with the authorised privacy lead and record the answers in the firm’s AI register or privacy record:

1. Is the firm an APP entity, generally or for particular activities? Consider s 6D, including the $3,000,000 turnover test, the requirement that every business carried on be a small business and the exceptions in s 6D(4) and (9); s 6E, which treats a small business operator as an organisation for particular activities, including under s 6E(1A), where it is a reporting entity or an authorised agent of one because of anything done in the course of a small business it carries on, for activities carried on for the purposes of, or in connection with, AML/CTF activities; and an opt-in under s 6EA.
2. If so, has the firm arranged, for those activities, for the tool to make, or do a thing substantially and directly related to making, a decision that could reasonably be expected to significantly affect an individual’s rights or interests, using personal information about that individual in the tool’s operation? Under APP 1.9 this includes refusing or failing to decide or act, and an adverse or a beneficial effect. The OAIC’s [fact sheet](https://www.oaic.gov.au/__data/assets/pdf_file/0021/269013/APP-1.7-1.9-Transparency-Obligation-Fact-Sheet.PDF) and [APP Guidelines Chapter 1](https://www.oaic.gov.au/__data/assets/pdf_file/0027/269064/APP-Guidelines-APP-1-Open-and-transparent-management-of-personal-information-September-2026-v2.pdf) (both September 2026) treat procuring, configuring or relying on third-party software as arranging, and say a decision can be in scope even where the output is advisory or subject to human review. That guidance is not law.
3. If so, record what APP 1.8 requires the policy to state: the kinds of personal information used, the kinds of decisions the tool makes solely, and the kinds of decisions for which it does a substantially and directly related thing. [VA-37](./reference/ai-vendor-assurance.md) asks the supplier for this information.

The firm reaches the legal conclusion at each step. The LLM must not state that a firm, tool or decision is or is not covered, and must record an unknown answer as open. An open answer does not stop the task; the rest of this gate still applies.

