## Why
- Operators report the Bible sidebar under `/ui/operator` wastes vertical space and lacks the controls now standard in the worship Libraries list, forcing extra navigation during services.
- Issue #68 requires the Bible panel to expose the same affordances (count, dashboard shortcut, add button) and remove the outdated helper copy.

## What Changes
- Rename the "Translations" header to "Bibles" and relocate the list to the top of the sidebar with spacing that matches the worship Libraries component.
- Surface the live Bible translation count beside the header, reuse the existing library dashboard picker for its click action, and expose a `+` control that launches the Bible import/new flow.
- Remove the helper subtitle copy and update styling tokens so typography, padding, and row sizing mirror the Libraries list.
- Extend automated UI coverage to assert the new header label and `+` control render in the Bible view.

## Impact
- No backend or schema changes; work is isolated to the operator UI and shared components.
- Reuses existing dashboard surfacing interaction, maintaining consistency and minimizing new state wiring.
- Playwright coverage must be updated to protect the renamed header and control visibility before shipping.
