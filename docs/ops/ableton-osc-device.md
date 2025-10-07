# Ableton OSC (Connection Kit)

Presenter now relies on Ableton Live’s built-in **Connection Kit → OSC Send** Max for Live device. The stock plugin keeps the host/port settings when you save a preset, so we no longer distribute a custom patch.

## Install once

1. Install the free [Ableton Connection Kit](https://www.ableton.com/en/packs/connection-kit/) if it is not already available in Live’s browser.
2. In Live’s `Packs` panel locate **Connection Kit → Max MIDI Effect → OSC Send**.
3. Drag the device onto the MIDI track that fires Presenter cues (usually the AbleSet automation track).

## Configure the device

1. Set **Host** to `presenter.lan` (or the direct IP of the presenter controller when you are off the church network).
2. Set **Port** to `39051`. This is the same value shown in Settings → Ableton Control under “OSC Listener Port”.
3. Leave the **Address** at `/note` and the **Velocity** checkbox enabled so velocity values transmit the slide index as before.
4. Click the disk icon on the device to save a preset named `Presenter OSC Send`. This freezes the host/port values so reopening the Live Set restores them automatically. You can also right-click and choose **Save as Default Preset** if every set will target Presenter.

## Daily workflow

1. Load the automation Live Set in Ableton or AbleSet.
2. Ensure the **Presenter OSC Send** preset is on the track; the host/port fields should already show `presenter.lan` and `39051`.
3. Arm the track or route AbleSet’s automation to that device.
4. When Ableton fires MIDI notes 19 or 20, the OSC device forwards them to Presenter with the same velocity values. Presenter resolves the current NEW LEVEL song via AbleSet and triggers slides in the background.

## Troubleshooting

- **Host/Port reset** – Reapply the preset or set it as the default preset so Live loads the saved configuration. If the fields still clear, delete the device, insert a fresh instance, set the values, and resave the preset.
- **No OSC traffic** – From the Ableton machine run `ping presenter.lan` (or use the IP address). If it fails, verify the network path or temporarily use the IP in the Host field.
- **Presenter shows velocity 0** – Remember that note-off messages arrive with velocity 0 after each cue. Presenter ignores them for slide selection; the “Last note” status will briefly show the note with velocity 0 between cues.

Keep this document updated whenever the OSC workflow changes.
