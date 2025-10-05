# Presenter OSC Send (Ableton Live)

This custom Max for Live MIDI device replaces the Ableton Connection Kit “OSC/MIDI Send” plugin so that the Presenter endpoint configuration survives saving and reopening a Live Set.

## Why we replaced the stock device

The Connection Kit device forgets its target host/port whenever a set is saved and reopened, forcing operators to re-enter the Presenter IP and OSC port before every service. That regression caused issue [#35](https://github.com/zbynekdrlik/presenter/issues/35).

`Presenter OSC Send` embeds the destination fields in the Live Set via `pattrstorage`, so the values reload automatically with the session.

## Installation

1. **Prerequisites** – Ableton Live Suite (with Max for Live) and Max 8.5+ installed locally.
2. **Copy the patch** – In this repository locate `ableton/presenter-osc-send.maxpat` (you can also download it from *Settings → Ableton Bridge* inside Presenter).
3. **Open in Max** – Double-click the file (or open from Max: `File → Open…`).
4. **Export as a Max for Live device** – In Max choose `File → Save As…`, set the format to *Max for Live MIDI Effect* (`.amxd`), and save it inside your `Ableton User Library/Presets/MIDI Effects/Presenter/` folder. Name the device `Presenter OSC Send.amxd`.
5. **(Optional) Create a default preset** – Drag the saved device into Ableton and click the disk icon to store a preset so it appears in Live’s browser.

## Using the device

1. Drop the device on the Ableton MIDI track that drives Presenter.
2. Set **OSC host** to your Presenter endpoint (the Max patch defaults to `presenter.lan`; for demos you can still reach it at `10.77.9.21`).
3. Leave **OSC port** at `39051` unless you have changed the Presenter OSC bridge port.
4. Arm the track / reroute AbleSet as usual. MIDI notes now forward to Presenter over OSC.
5. Save the Live Set. Reopen it and confirm the host/port fields reload automatically.

The device still passes MIDI through to the rest of the track so existing arrangements keep working.

## Version control

Check the exported `.amxd` into your project repo (e.g. `ableton/devices/`) so other operators can drag the same device into their Ableton User Library without re-exporting.
If you ever change the default host/port, update the `.maxpat` in the repo so the download served from Presenter stays aligned.

## Troubleshooting

- **No OSC traffic** – Confirm the Presenter host is reachable from the Ableton machine (`ping 10.77.9.21`) and the port matches the OSC bridge setting in Presenter.
- **Host/port fields empty after export** – Reopen the `.maxpat` in Max, set the desired defaults, then re-export the `.amxd`. The `pattrstorage` object captures the values present at export time.
- **Device missing in Live Browser** – Rescan: `Preferences → Library → Rescan Plug-Ins`. Alternatively drag the `.amxd` from Finder/Explorer directly onto a track.

## Repository layout

- `ableton/presenter-osc-send.maxpat` – Source patch for the Max device (text-based, friendly to git).
- `docs/ops/ableton-osc-device.md` – This guide.

Keep this documentation synced if the device gains additional parameters.
