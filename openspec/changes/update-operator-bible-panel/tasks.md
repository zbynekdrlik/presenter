1. [x] Update the Bible sidebar header in `crates/presenter-server/src/ui/bible.rs` to render `Bibles (count)` with the count button wired to the existing dashboard surfacing action and remove the helper subtitle copy.
2. [x] Add the `+` control beside the header that launches the Bible import/add flow, reusing the same handler the libraries view uses.
3. [x] Align the translations list styling/padding with the Libraries component and confirm the list sits at the top of the sidebar with matching spacing tokens.
4. [x] Extend the Playwright operator Bible spec to assert the `Bibles` header, count button, and `+` control render.
5. [x] Run `sudo -E ./scripts/dev/verify-and-refresh.sh` and address any failures before submitting the change.
