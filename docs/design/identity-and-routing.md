# One person, several agents

Additional requirements confirmed during implementation:

- A verified phone number anchors the person's Litecom account. Stable internal IDs preserve contacts when the number changes.
- Every connected agent authenticates as an independently revocable connection of that account. No shared bearer secret is copied between harnesses.
- All of your agents can ask, receive, and check the same contacts and requests.
- “Get Moe” selects Moe's default agent. “Get Moe's Devin” selects his named cloud agent. The default can be local or cloud-hosted.
- Cloud agents do not require the owner's computer to be awake. Their availability and credentials are checked independently.
- The initial question remains “Where do you want to approve requests?” Phone, Slack, and Email are notification choices. Phone verification establishes identity; Slack/email connection is additional work when selected.

Revised onboarding targets: Phone 7 controls; Email 9; Slack 8 plus provider sign-in/install controls. These replace the previous phone-optional counts and remain unmeasured until the real flows are implemented.

Devin provides an authenticated cloud session API. Its [current authentication documentation](https://docs.devin.ai/api-reference/authentication) requires a personal access token or service-user credential; do not advertise a verified OAuth flow without evidence. A provider that cannot enforce preparation-only permissions must not start an autonomous run before the owner's explicit execution approval. Results still go through Litecom's sharing boundary.

Self-requests use the same routing and approval machinery. “Ask me” chooses your default; “ask my Devin” selects your named agent. No contact invitation is needed within one account.

The client, relay, and background service are implemented in Rust. Browser pages use small static JavaScript and CSS assets bundled into the binary.
