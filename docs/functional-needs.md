# Functional Needs

## Lyrics
- Rapid search across catalog, set lists, and history.
- Support for alternate arrangements, key changes, and responsive line breaks.
- Operator view should preview next slide and permit one-click or shortcut jumps.

## Bible
- Offline cache of approved translations with fast reference lookup.
- Formatting presets for scripture slides (single verse, passage, responsive reading).
- Highlight keywords or responsive parts for stage prompts.

## Timers
- Global service clock with overruns/underruns visual indicators.
- Segment timers tied to plan items, supporting pause/adjust mid-service.
- Stage alerts for time warnings (flash, color shift, optional audio ping).
- OBS-friendly countdown overlay with transparent background for live streams.

## Stage Displays
- Layout builder that places lyrics, timers, notes, and alerts in flexible regions.
- Single stage display endpoint (`/stage`) whose mode is driven from the operator header, keeping OBS browser sources and on-stage monitors aligned without URL churn.
- Per-display profiles (e.g., main stage monitor vs. choir screen).
- Remote preview on tablets/phones for production leads.
- Persistent heartbeat and latency visibility so operators can confirm live links without advancing slides.
- Live synchronization with Resolume Arena decks, including auto-discovery of clips and alternating A/B lanes for main, translation, and Bible outputs.

## External Integrations
- Manage multiple Resolume Arena connections (DNS hostnames on port 8090 by default) from a
  centralized settings surface.
- Cache clip metadata and warn operators when expected `#main-a`, `#bible-clear`, etc. tokens are
  missing so they can fix naming before service.
- Stream countdown text into a dedicated `#timer` clip on each deck without triggering playback.
- Allow clip name modifiers (e.g., `-u`, `-re`) so formatting tweaks happen directly in Resolume without duplicating slides.
- Guarantee sub-100 ms trigger-to-clip latency with health indicators when latency drifts.
- Accept OSC note/velocity commands (mirroring ProPresenter MIDI auto-fill: notes 18/19/20) so Ableton cues routed through Max for Live can select playlists, presentations, and trigger slides without reprogramming show files.
- Provide operator controls for AbleSet automation: a global enable/disable toggle (default on) and a
  separate follow toggle (default off) that lets automation drive slides in the background while
  keeping the UI steady unless follow is explicitly engaged.
- Surface the current AbleSet song title and slide index in the operator header so teams can verify
  background triggers even when follow is disabled.

## Data Model Considerations
- Versioned song data to track edits and reuse arrangements.
- Scripture references stored as structured data (book, chapter, verses) for reliable rendering.
- Timer templates stored per service type (Sunday AM, youth night, rehearsal).

## Operational Workflow
1. Build plan with songs, scripture, and service segments.
2. Assign stage display profiles per venue.
3. Run service from control UI, track live status, and push cues to displays.
4. Capture operator notes/session log for future refinement.

## Open Questions
- Which UI stack best supports multi-display output on Windows/macOS?
- How should we sync data between control machine and backup devices? Offline-first or cloud-first?
- Do we need mobile remote control or limit to primary console?
