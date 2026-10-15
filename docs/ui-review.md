# Interface review

Reviewed 2026-10-15. Scope: the bundled browser sign-in, pairing, inbox, request approval/result, contacts, agents, and approval-channel settings. Stack: server-rendered shell with plain JavaScript and shared CSS. Conventions: [UI.md](UI.md), LiteReview's UI guide, [LiteLLM UI skill](https://github.com/BerriAI/litellm-internal-skills/blob/main/litellm-based-ui-design/SKILL.md), and [better-interface](https://github.com/jakubkrehel/skills/blob/main/skills/better-interface/SKILL.md).

## Simplification pass

The 2026-10-15 redesign replaces separate cards with shared rows, uses a compact desktop sidebar, and keeps setup to a channel picker and one copyable command. Optional connection and account controls use native disclosures. Approval still shows the requested action and the effect of approving it.

Verified with the Google/Phone/Slack/Email browser fixtures, the actual local relay and native background connection, and the preview accessibility audit at 320/390/768/1100 px and 200% text size. Browser reliability checks cover stale decisions, recipient switching, pagination, interrupted downloads, and edits during live updates. Provider APIs and native model replies remain fixtures in these checks.

## Coverage

| Domain | Evidence inspected | Result |
| --- | --- | --- |
| Accessibility | `src/web/index.html`, shared controls in `app.js` and `style.css`; axe on seven screens; keyboard skip link, disclosure, and approval | No remaining actionable finding in these checks; screen-reader walkthrough not verified |
| Layout | Login, request, result, inbox, settings, contacts, agents at 320, 390, 768, and 1100 px; enlarged text | No horizontal overflow or clipped controls observed |
| Writing | Sign-in, connection, approval, error, empty, and result states | One approval for work and delivery; recovery actions remain visible |
| Typography | Bundled Geist and Geist Mono, shared type styles, mobile controls | Consistent hierarchy; mobile input text remains readable |
| Colors | Shared ink, neutral surface, purple action, and semantic status styles; rendered axe contrast checks | No contrast violations reported on inspected screens; status has text labels |
| UI | Native buttons, inputs, disclosures, links, and shared cards | Consistent controls; details and focus survive ordinary request refreshes |

## Findings addressed

| Severity | Domain | Location | Before | After | Why |
| --- | --- | --- | --- | --- | --- |
| HIGH | UI | `src/web/app.js`, request refresh | Background refresh could replace a response being edited | Preserve an in-progress edit, including after the field loses focus | Prevent loss of typed work |
| MEDIUM | Writing | `src/web/app.js`, approval/result | Separate execution and sharing instructions | One request approval with automatic final delivery | Match the authorized product flow |
| MEDIUM | Accessibility | `src/web/index.html`, `src/web/style.css`, navigation in `app.js` | Incomplete keyboard landmarks and refresh continuity | Skip link, visible focus, page titles, current-navigation state, and refresh focus handling | Keep orientation and keyboard access consistent |
| MEDIUM | UI | `src/web/style.css` | Inconsistent visual treatment | Shared LiteReview-style type, surfaces, controls, and spacing | Make equivalent actions look and behave consistently |

## Verification

- `scripts/preview-smoke.py --binary target/debug/litecom --axe /path/to/axe.min.js`: passed the first-use flow, one approval, exact download, and clean exit. The optional audit reported zero axe violations on login, request, result, inbox, agents, contacts, and settings. Keyboard navigation reached the main content, toggled details, and approved the request.
- `scripts/reliability-smoke.py --case editing`: passed background-update and blur checks while preserving unsaved text. The test now locates the current request heading rather than its retired label.
- The same browser reliability suite passed pagination, contact routing, stale approvals, interrupted/corrupt downloads, receipt retry, and revocation before the heading-selector correction.
- Visually inspected mobile approval and desktop agent screens, and rendered README and setup/channel documentation at mobile and desktop widths.
- **Not verified:** an actual screen-reader walkthrough, real message-provider delivery, and operating-system installation prompts. The first-use tests use deterministic agent processes and local delivery.

## Verdict

**Approve** for the inspected browser interface. No HIGH interface findings remain in this scope. This review does not certify untested assistive-technology behavior, external providers, or deployment readiness.
