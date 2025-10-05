{
  "patcher": {
    "fileversion": 1,
    "appversion": {
      "major": 8,
      "minor": 5,
      "revision": 0,
      "architecture": "x64",
      "modernui": 1
    },
    "classnamespace": "box",
    "rect": [
      62.0,
      85.0,
      800.0,
      520.0
    ],
    "bglocked": 0,
    "defrect": [
      0.0,
      0.0,
      800.0,
      520.0
    ],
    "openrect": [
      188.0,
      134.0,
      800.0,
      520.0
    ],
    "openinpresentation": 0,
    "default_fontsize": 12.0,
    "default_fontface": 0,
    "default_fontname": "Arial",
    "gridonopen": 0,
    "gridsize": [
      15.0,
      15.0
    ],
    "gridsnaponopen": 1,
    "toolbarvisible": 1,
    "boxanimatetime": 200,
    "enablehscroll": 1,
    "enablevscroll": 1,
    "boxes": [
      {
        "box": {
          "id": "obj-in",
          "maxclass": "inlet",
          "patching_rect": [
            60.0,
            110.0,
            25.0,
            25.0
          ]
        }
      },
      {
        "box": {
          "id": "obj-trigger",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            105.0,
            50.0,
            22.0
          ],
          "text": "t l l"
        }
      },
      {
        "box": {
          "id": "obj-out",
          "maxclass": "outlet",
          "patching_rect": [
            210.0,
            110.0,
            25.0,
            25.0
          ]
        }
      },
      {
        "box": {
          "id": "obj-midiparse",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            150.0,
            66.0,
            22.0
          ],
          "text": "midiparse"
        }
      },
      {
        "box": {
          "id": "obj-unpack",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            190.0,
            80.0,
            22.0
          ],
          "text": "unpack 0 0 0"
        }
      },
      {
        "box": {
          "id": "obj-pak",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            230.0,
            70.0,
            22.0
          ],
          "text": "pak i i i"
        }
      },
      {
        "box": {
          "id": "obj-prepend",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            270.0,
            90.0,
            22.0
          ],
          "text": "prepend /note"
        }
      },
      {
        "box": {
          "id": "obj-udpsend",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            310.0,
            130.0,
            22.0
          ],
          "text": "udpsend 127.0.0.1 39051"
        }
      },
      {
        "box": {
          "id": "obj-host-label",
          "maxclass": "comment",
          "patching_rect": [
            360.0,
            100.0,
            80.0,
            20.0
          ],
          "text": "OSC host",
          "fontsize": 12.0
        }
      },
      {
        "box": {
          "id": "obj-host",
          "maxclass": "textedit",
          "patching_rect": [
            360.0,
            120.0,
            200.0,
            24.0
          ],
          "fontname": "Arial",
          "fontsize": 12.0,
          "text": "127.0.0.1",
          "varname": "host_input",
          "pastemode": 1,
          "wordwrap": 0,
          "parameter_enable": 0
        }
      },
      {
        "box": {
          "id": "obj-route-text",
          "maxclass": "newobj",
          "patching_rect": [
            580.0,
            120.0,
            68.0,
            22.0
          ],
          "text": "route text"
        }
      },
      {
        "box": {
          "id": "obj-thost",
          "maxclass": "newobj",
          "patching_rect": [
            660.0,
            120.0,
            52.0,
            22.0
          ],
          "text": "t s s"
        }
      },
      {
        "box": {
          "id": "obj-send-host",
          "maxclass": "newobj",
          "patching_rect": [
            720.0,
            105.0,
            90.0,
            22.0
          ],
          "text": "s host_current"
        }
      },
      {
        "box": {
          "id": "obj-prepend-host",
          "maxclass": "newobj",
          "patching_rect": [
            720.0,
            145.0,
            90.0,
            22.0
          ],
          "text": "prepend host"
        }
      },
      {
        "box": {
          "id": "obj-recv-host",
          "maxclass": "newobj",
          "patching_rect": [
            720.0,
            185.0,
            92.0,
            22.0
          ],
          "text": "r host_current"
        }
      },
      {
        "box": {
          "id": "obj-port-label",
          "maxclass": "comment",
          "patching_rect": [
            360.0,
            170.0,
            80.0,
            20.0
          ],
          "text": "OSC port",
          "fontsize": 12.0
        }
      },
      {
        "box": {
          "id": "obj-port",
          "maxclass": "newobj",
          "patching_rect": [
            360.0,
            190.0,
            80.0,
            22.0
          ],
          "text": "live.numbox",
          "varname": "port_input",
          "parameter_enable": 0,
          "minimum": 1.0,
          "maximum": 65535.0,
          "floatoutput": 0,
          "valueof": [
            39051.0
          ]
        }
      },
      {
        "box": {
          "id": "obj-int",
          "maxclass": "newobj",
          "patching_rect": [
            450.0,
            190.0,
            35.0,
            22.0
          ],
          "text": "i"
        }
      },
      {
        "box": {
          "id": "obj-tport",
          "maxclass": "newobj",
          "patching_rect": [
            500.0,
            190.0,
            45.0,
            22.0
          ],
          "text": "t i i"
        }
      },
      {
        "box": {
          "id": "obj-send-port",
          "maxclass": "newobj",
          "patching_rect": [
            560.0,
            175.0,
            88.0,
            22.0
          ],
          "text": "s port_current"
        }
      },
      {
        "box": {
          "id": "obj-prepend-port",
          "maxclass": "newobj",
          "patching_rect": [
            560.0,
            215.0,
            88.0,
            22.0
          ],
          "text": "prepend port"
        }
      },
      {
        "box": {
          "id": "obj-recv-port",
          "maxclass": "newobj",
          "patching_rect": [
            560.0,
            255.0,
            90.0,
            22.0
          ],
          "text": "r port_current"
        }
      },
      {
        "box": {
          "id": "obj-autopattr",
          "maxclass": "newobj",
          "patching_rect": [
            360.0,
            250.0,
            70.0,
            22.0
          ],
          "text": "autopattr"
        }
      },
      {
        "box": {
          "id": "obj-pattrstorage",
          "maxclass": "newobj",
          "patching_rect": [
            360.0,
            290.0,
            230.0,
            22.0
          ],
          "text": "pattrstorage store @autorestore 1 @outputmode 1"
        }
      },
      {
        "box": {
          "id": "obj-route-store",
          "maxclass": "newobj",
          "patching_rect": [
            360.0,
            330.0,
            180.0,
            22.0
          ],
          "text": "route host_input port_input"
        }
      },
      {
        "box": {
          "id": "obj-help",
          "maxclass": "comment",
          "patching_rect": [
            360.0,
            380.0,
            340.0,
            50.0
          ],
          "fontsize": 11.0,
          "text": "Presenter OSC Send: drop on a MIDI track and route Ableton notes. Host/port are stored with the Live Set via pattrstorage."
        }
      }
    ],
    "lines": [
      {
        "patchline": {
          "source": [
            "obj-in",
            0
          ],
          "destination": [
            "obj-trigger",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-trigger",
            0
          ],
          "destination": [
            "obj-out",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-trigger",
            1
          ],
          "destination": [
            "obj-midiparse",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-midiparse",
            0
          ],
          "destination": [
            "obj-unpack",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-unpack",
            0
          ],
          "destination": [
            "obj-pak",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-unpack",
            1
          ],
          "destination": [
            "obj-pak",
            1
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-unpack",
            2
          ],
          "destination": [
            "obj-pak",
            2
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-pak",
            0
          ],
          "destination": [
            "obj-prepend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-prepend",
            0
          ],
          "destination": [
            "obj-udpsend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-recv-host",
            0
          ],
          "destination": [
            "obj-prepend-host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-prepend-host",
            0
          ],
          "destination": [
            "obj-udpsend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-recv-port",
            0
          ],
          "destination": [
            "obj-prepend-port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-prepend-port",
            0
          ],
          "destination": [
            "obj-udpsend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-host",
            0
          ],
          "destination": [
            "obj-route-text",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-route-text",
            0
          ],
          "destination": [
            "obj-thost",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-thost",
            0
          ],
          "destination": [
            "obj-send-host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-thost",
            1
          ],
          "destination": [
            "obj-prepend-host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-port",
            0
          ],
          "destination": [
            "obj-int",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-int",
            0
          ],
          "destination": [
            "obj-tport",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-tport",
            0
          ],
          "destination": [
            "obj-send-port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-tport",
            1
          ],
          "destination": [
            "obj-prepend-port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-pattrstorage",
            1
          ],
          "destination": [
            "obj-route-store",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-route-store",
            0
          ],
          "destination": [
            "obj-send-host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-route-store",
            1
          ],
          "destination": [
            "obj-send-port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-send-host",
            0
          ],
          "destination": [
            "obj-prepend-host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-send-port",
            0
          ],
          "destination": [
            "obj-prepend-port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-send-host",
            0
          ],
          "destination": [
            "obj-recv-host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "obj-send-port",
            0
          ],
          "destination": [
            "obj-recv-port",
            0
          ]
        }
      }
    ]
  }
}