# Privacy Threat Model (LINDDUN)

The Workbench can process source code, prompts, provider account metadata, tool
results, and audit history that identify developers or people represented in a
repository. This LINDDUN model covers every category using the required
lowercase tokens.

| ID | Category | Threat | Affected Data | Mitigation |
|----|----------|--------|---------------|------------|
| PT-01 | linking | session, Git, tool, and provider events are correlated into a broader developer activity profile | session identifiers, repository metadata, routing history | scope identifiers to one local store, exclude them from metric labels, and make exports explicit |
| PT-02 | identifying | prompts or artifacts reveal a developer or data subject to a provider that did not need the data | names, emails, source code, issue content, provider account metadata | show destination and context before dispatch, minimize context by role, and redact persisted snapshots |
| PT-03 | non-repudiation | durable audit history proves a person approved or initiated sensitive work beyond the required purpose | approval actor, timestamps, tool decisions, session history | document the audit purpose, apply configurable retention, and restrict export and deletion operations |
| PT-04 | detecting | another local process infers that a person, project, or provider session exists | IPC endpoints, process list, state paths, timing | use same-user IPC permissions, opaque identifiers, private state directories, and no network listener by default |
| PT-05 | data-disclosure | credentials, prompts, source, model output, or tool results leak through logs, errors, clients, or providers | keychain references, prompt bodies, artifacts, provider diagnostics | never persist credential values, log metadata by default, enforce client and tool grants, and redact error details |
| PT-06 | unawareness | a user does not realize which model, tools, context, or external service will receive a prompt | routing plan, context sources, tool grants, provider privacy mode | display the routing plan and approvals before dispatch and retain the decision in session history |
| PT-07 | non-compliance | retained prompts or cross-provider transfers exceed project policy or provider privacy commitments | event database, artifacts, exports, provider requests | attach policy and redacted config snapshots to sessions, validate retention rules, and audit every external dispatch |
