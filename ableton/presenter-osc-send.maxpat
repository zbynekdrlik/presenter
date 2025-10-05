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
      760.0,
      480.0
    ],
    "bglocked": 0,
    "defrect": [
      0.0,
      0.0,
      760.0,
      480.0
    ],
    "openrect": [
      180.0,
      120.0,
      760.0,
      480.0
    ],
    "openinpresentation": 0,
    "default_fontsize": 12.0,
    "default_fontface": 0,
    "default_fontname": "Arial",
    "toolbarvisible": 1,
    "boxanimatetime": 200,
    "enablehscroll": 1,
    "enablevscroll": 1,
    "boxes": [
      {
        "box": {
          "id": "inlet",
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
          "id": "trigger",
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
          "id": "outlet",
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
          "id": "midiparse",
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
          "id": "unpack",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            190.0,
            70.0,
            22.0
          ],
          "text": "unpack 0 0"
        }
      },
      {
        "box": {
          "id": "pak",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            230.0,
            70.0,
            22.0
          ],
          "text": "pak i i"
        }
      },
      {
        "box": {
          "id": "loadmess",
          "maxclass": "newobj",
          "patching_rect": [
            200.0,
            190.0,
            80.0,
            22.0
          ],
          "text": "loadmess 1"
        }
      },
      {
        "box": {
          "id": "prepend",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            270.0,
            110.0,
            22.0
          ],
          "text": "prepend /note"
        }
      },
      {
        "box": {
          "id": "udpsend",
          "maxclass": "newobj",
          "patching_rect": [
            110.0,
            310.0,
            150.0,
            22.0
          ],
          "text": "udpsend presenter.lan 39051"
        }
      },
      {
        "box": {
          "id": "host_label",
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
          "id": "host_input",
          "maxclass": "textedit",
          "patching_rect": [
            360.0,
            120.0,
            200.0,
            24.0
          ],
          "text": "presenter.lan",
          "fontname": "Arial",
          "fontsize": 12.0,
          "varname": "host_input",
          "pastemode": 1,
          "wordwrap": 0,
          "parameter_enable": 0
        }
      },
      {
        "box": {
          "id": "route_text",
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
          "id": "thost",
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
          "id": "send_host",
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
          "id": "prepend_host",
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
          "id": "recv_host",
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
          "id": "port_label",
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
          "id": "port_input",
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
          "id": "int_port",
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
          "id": "tport",
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
          "id": "send_port",
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
          "id": "prepend_port",
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
          "id": "recv_port",
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
          "id": "autopattr",
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
          "id": "pattrstorage",
          "maxclass": "newobj",
          "patching_rect": [
            360.0,
            290.0,
            205.0,
            22.0
          ],
          "text": "pattrstorage store @autorestore 1 @outputmode 1"
        }
      },
      {
        "box": {
          "id": "route_store",
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
          "id": "note",
          "maxclass": "comment",
          "patching_rect": [
            360.0,
            380.0,
            330.0,
            48.0
          ],
          "fontsize": 11.0,
          "text": "Presenter OSC: drop on a MIDI track feeding Presenter. Host/port are stored with the Live Set via pattrstorage."
        }
      }
    ],
    "lines": [
      {
        "patchline": {
          "source": [
            "inlet",
            0
          ],
          "destination": [
            "trigger",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "trigger",
            0
          ],
          "destination": [
            "outlet",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "trigger",
            1
          ],
          "destination": [
            "midiparse",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "midiparse",
            0
          ],
          "destination": [
            "unpack",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "unpack",
            0
          ],
          "destination": [
            "pak",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "loadmess",
            0
          ],
          "destination": [
            "pak",
            1
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "pak",
            0
          ],
          "destination": [
            "prepend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "prepend",
            0
          ],
          "destination": [
            "udpsend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "recv_host",
            0
          ],
          "destination": [
            "prepend_host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "prepend_host",
            0
          ],
          "destination": [
            "udpsend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "recv_port",
            0
          ],
          "destination": [
            "prepend_port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "prepend_port",
            0
          ],
          "destination": [
            "udpsend",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "host_input",
            0
          ],
          "destination": [
            "route_text",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "route_text",
            0
          ],
          "destination": [
            "thost",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "thost",
            0
          ],
          "destination": [
            "send_host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "thost",
            1
          ],
          "destination": [
            "prepend_host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "port_input",
            0
          ],
          "destination": [
            "int_port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "int_port",
            0
          ],
          "destination": [
            "tport",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "tport",
            0
          ],
          "destination": [
            "send_port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "tport",
            1
          ],
          "destination": [
            "prepend_port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "pattrstorage",
            1
          ],
          "destination": [
            "route_store",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "route_store",
            0
          ],
          "destination": [
            "send_host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "route_store",
            1
          ],
          "destination": [
            "send_port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "send_host",
            0
          ],
          "destination": [
            "prepend_host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "send_host",
            0
          ],
          "destination": [
            "recv_host",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "send_port",
            0
          ],
          "destination": [
            "prepend_port",
            0
          ]
        }
      },
      {
        "patchline": {
          "source": [
            "send_port",
            0
          ],
          "destination": [
            "recv_port",
            0
          ]
        }
      }
    ]
  }
}