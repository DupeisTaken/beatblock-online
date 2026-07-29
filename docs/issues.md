# Creating issues

Use GitHub issues for reproducible defects, focused improvements, documentation
gaps, and questions that are not answered by the existing guides. Choose the
template that matches the issue:

- [Bug report](https://github.com/DupeisTaken/beatblock-online/issues/new?template=bug_report.yml)
- [Beatblock compatibility report](https://github.com/DupeisTaken/beatblock-online/issues/new?template=beatblock_compatibility.yml)
- [Feature request](https://github.com/DupeisTaken/beatblock-online/issues/new?template=feature_request.yml)
- [Enhancement request](https://github.com/DupeisTaken/beatblock-online/issues/new?template=enhancement_request.yml)
- [Documentation change](https://github.com/DupeisTaken/beatblock-online/issues/new?template=documentation.yml)
- [Question](https://github.com/DupeisTaken/beatblock-online/issues/new?template=question.yml)

Before opening an issue, search the open and closed issues, check the newest
release, and review the relevant troubleshooting guide.

Keep each issue to one problem or request. A focused issue is easier to confirm,
prioritize, and close without losing related discussion.

## Choose an issue tag

GitHub calls these tags **labels**. Each issue form applies the matching label
automatically. Compatibility reports receive both `bug` and `compatibility`:

| Label           | Use it for                                                                 |
| --------------- | -------------------------------------------------------------------------- |
| `bug`           | Behavior that is broken, incorrect, or unexpectedly different from a guide |
| `feature`       | A new user-facing or developer capability                                  |
| `enhancement`   | A focused improvement to existing behavior                                 |
| `documentation` | Missing, unclear, or inaccurate documentation                              |
| `question`      | A usage or development question not answered by the documentation          |
| `compatibility` | An unverified Beatblock build or an update that breaks integration         |

Maintainers may add or replace labels during triage:

| Label              | Meaning                                                     |
| ------------------ | ----------------------------------------------------------- |
| `duplicate`        | An existing issue already tracks the same work              |
| `invalid`          | The report cannot be acted on as written or is out of scope |
| `wontfix`          | The project does not plan to implement the request          |
| `help wanted`      | Maintainer or contributor help would move the issue forward |
| `good first issue` | The work is bounded and suitable for a first contribution   |

Do not add a label prefix such as `[Bug]` to the title. Labels provide that
classification; the title should describe the observable problem or requested
outcome.

## Write a useful title

Use a short, specific title that remains understandable in a search result.
Describe what happens and where it happens.

- Good: `Player remains reconnecting after the host closes the room`
- Good: `Document the frp firewall requirements`
- Avoid: `It does not work`
- Avoid: `Feature request`

## Report a bug

Include enough information for another person to reproduce and investigate the
problem:

1. **Summary:** What failed and what impact did it have?
2. **Version and environment:** Beatblock Online release, Beatblock version,
   Windows version, and the relevant role (Host, Player, Spectator, or
   Commentator). Include the install adapter, network path, and OBS version when
   they matter.
3. **Steps to reproduce:** List the smallest repeatable sequence, including room
   settings, chart type, and role assignments where relevant.
4. **Expected behavior:** What should have happened?
5. **Actual behavior:** What happened instead, including the exact visible
   error or status.
6. **Evidence:** Attach a minimal log excerpt, screenshot, recording, or
   timestamp when available. Say whether the problem happens every time.

Copy error text exactly, but remove room passwords, public addresses, usernames,
filesystem user names, tokens, and other personal or secret data. Do not attach
an entire log when a short excerpt around the failure is enough.

Use the dedicated compatibility form when a newer Beatblock build is accepted
but unverified, or when an update breaks installation or runtime behavior.
Include the complete displayed Beatblock version and bracketed build token.
Newer builds remain unverified until they have been explicitly validated.

## Request a feature or enhancement

Describe the problem or use case before proposing an implementation. Explain
who benefits, the desired outcome, and any alternatives or workarounds already
tried. Keep the request narrow enough to evaluate independently; split unrelated
capabilities into separate issues. Use `feature` when the outcome introduces a
new capability and `enhancement` when it improves behavior that already exists.

For UI changes, describe the affected workspace and input methods. For protocol,
networking, installer, or OBS changes, note compatibility and migration concerns
that the proposal may need to address.

## Request a documentation change or ask a question

For documentation, link to the affected page or heading, quote only the small
part that is unclear, and describe the correction or missing audience need.

For a question, explain the goal, what the documentation suggested, and what was
already tried. Questions that reveal a repeatable defect may be relabeled as a
bug during triage.

## After filing

Respond to requests for missing details and test proposed fixes against the same
environment. Maintainers may edit the title, labels, or scope to keep the issue
searchable and actionable. Close the issue if it is resolved by configuration,
an existing release, or another tracked issue.
