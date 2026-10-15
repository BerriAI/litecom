# Litecom UI

Use the same visual language as [LiteReview](https://github.com/BerriAI/litereview/blob/main/docs/UI.md): Geist typography, white surfaces, thin neutral borders, restrained weights, and violet primary actions. The shared tokens and component rules live in `src/web/style.css`; page markup lives in `src/web/app.js`. Both are embedded in the executable.

## Components

- One primary action per task. Request approval is **Approve / Decline**. Approval authorizes the requested work and sending its result; there is no second sharing step.
- Agent access details use native `details` / `summary`. Keep the request and its approval controls visible without expanding anything.
- Buttons, inputs, selects, response panels, notices, status labels and disclosure controls use shared CSS rules. Do not add page-specific colors, radii or typefaces.
- Use flat surfaces and spacing. Color conveys a primary action or a real outcome. Avoid decorative badges, gradients, metrics and shadows.
- Match LiteReview's light appearance even when the operating system uses dark mode. A future dark theme must be deliberate and verified across every component.
- Text wraps, controls grow, and mobile inputs use 16px text. Keep at least 44px touch targets. Test 320px, 390px, 768px and desktop widths, keyboard navigation, and 200% text size.
- Use native accessible controls, visible labels and focus indicators. Announce errors, retain form edits, and keep errors visible while the person retries.
- Fonts are bundled locally. [Geist](licenses/geist-OFL.txt) and [Geist Mono](licenses/geist-mono-OFL.txt) use the SIL Open Font License 1.1; no third-party font request is needed.

## References

The implementation uses [LiteLLM's UI skill](https://github.com/BerriAI/litellm-internal-skills/blob/main/litellm-based-ui-design/SKILL.md) and the [better-interface review](https://github.com/jakubkrehel/skills/blob/main/skills/better-interface/SKILL.md), including its accessibility, layout, writing, typography, colors and UI-polish checks. LiteReview is the selected visual reference; no private skill is required to contribute using the rules above.

Run `scripts/ui-smoke.py`, `scripts/preview-smoke.py`, and `scripts/onboarding-smoke.py` as described in [Contributing](../CONTRIBUTING.md). Inspect the resulting desktop and mobile screenshots as well as the test result. Fixture delivery does not prove SMS or Slack delivery.
