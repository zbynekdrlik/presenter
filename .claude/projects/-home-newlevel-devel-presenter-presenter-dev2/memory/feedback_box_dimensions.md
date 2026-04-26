---
name: Box dimensions are user's decision
description: Never change stage box positions or dimensions without user approval — only implement rendering rules
type: feedback
---

Never change box sizes, positions, or dimensions for stage layout boxes without explicit user instruction. Box dimensions are a design decision the user defines. My role is to implement the rendering rules (autofit, line-height, overflow) within whatever box the user specifies.

**Why:** User explicitly said "i dont want from you to change box sizes by yourself, that is something what i should define."

**How to apply:** When text doesn't fill a box due to width/height constraints, explain the constraint and ask the user what dimensions they want — don't adjust them yourself.
