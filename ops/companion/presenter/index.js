const { InstanceBase, InstanceStatus, Regex, runEntrypoint } = require('@companion-module/base')
const WebSocket = require('ws')

const VARIABLE_DEFINITIONS = [
  'stage_presentation_id',
  'stage_presentation_name',
  'stage_current_slide_id',
  'stage_current_main',
  'stage_current_translation',
  'stage_current_stage',
  'stage_current_group',
  'stage_next_slide_id',
  'stage_next_main',
  'stage_next_translation',
  'stage_next_stage',
  'stage_next_group',
  'timer_countdown_state',
  'timer_countdown_target',
  'timer_countdown_remaining_seconds',
  'timer_preach_state',
  'timer_preach_elapsed_seconds',
  'bible_translation_code',
  'bible_translation_name',
  'bible_reference',
  'bible_text',
  'bible_triggered_at',
  'live_ws_connected',
]

const COMMANDS = [
  { id: 'timer.start_countdown', label: 'Timer: start countdown' },
  { id: 'timer.pause_countdown', label: 'Timer: pause countdown' },
  { id: 'timer.reset_countdown', label: 'Timer: reset countdown' },
  { id: 'timer.set_countdown_target', label: 'Timer: set countdown target (ISO)' },
  { id: 'timer.start_preach', label: 'Timer: start preach' },
  { id: 'timer.reset_preach', label: 'Timer: reset preach' },
  { id: 'stage.set', label: 'Stage: set slide' },
  { id: 'bible.trigger', label: 'Bible: trigger passage' },
  { id: 'bible.clear', label: 'Bible: clear passage' },
]

class PresenterInstance extends InstanceBase {
  constructor(internal) {
    super(internal)
    this.ws = null
    this.reconnectTimer = null
    this.variables = new Map()
  }

  getConfigFields() {
    return [
      {
        type: 'textinput',
        id: 'host',
        label: 'Presenter host / IP',
        regex: Regex.HOSTNAME,
        default: '10.77.9.21',
      },
      {
        type: 'number',
        id: 'port',
        label: 'Port',
        min: 1,
        max: 65535,
        default: 18175,
      },
      {
        type: 'checkbox',
        id: 'secure',
        label: 'Use TLS (wss://)',
        default: false,
      },
      {
        type: 'textinput',
        id: 'token',
        label: 'Companion token (optional)',
        default: '',
      },
      {
        type: 'number',
        id: 'reconnect',
        label: 'Auto-reconnect (ms)',
        default: 2000,
        min: 0,
      },
    ]
  }

  async init(config) {
    this.config = config
    this.setStatus(InstanceStatus.Connecting)
    this._setupVariables()
    this._setupActions()
    this._setupFeedbacks()
    this._connect()
  }

  async destroy() {
    if (this.ws) {
      this.ws.removeAllListeners()
      this.ws.terminate()
      this.ws = null
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
  }

  configUpdated(config) {
    this.config = config
    this._connect()
  }

  _connect() {
    if (!this.config.host || !this.config.port) {
      this.setStatus(InstanceStatus.BadConfig, 'Missing host or port')
      return
    }

    if (this.ws) {
      this.ws.removeAllListeners()
      this.ws.terminate()
      this.ws = null
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }

    const scheme = this.config.secure ? 'wss' : 'ws'
    const url = `${scheme}://${this.config.host}:${this.config.port}/companion/ws`

    this.log('debug', `Connecting to ${url}`)

    try {
      this.ws = new WebSocket(url)

      this.ws.addEventListener('open', () => {
        this.log('info', `Connected to Presenter: ${url}`)
        this.setStatus(InstanceStatus.Ok)

        const hello = {
          type: 'hello',
          client: 'Companion',
          instanceName: this.label || 'Companion',
        }
        if (this.config.token) {
          hello.token = this.config.token
        }
        this.ws.send(JSON.stringify(hello))
        this._updateVariable('live_ws_connected', 'true')
      })

      this.ws.addEventListener('message', (event) => {
        try {
          const parsed = JSON.parse(event.data.toString())
          this._handleMessage(parsed)
        } catch (error) {
          this.log('error', `Failed to parse message: ${error}`)
        }
      })

      this.ws.addEventListener('close', (event) => {
        this.log('warn', `Presenter socket closed (${event.code}): ${event.reason}`)
        this.setStatus(InstanceStatus.Disconnected, `Closed ${event.code}`)
        this._updateVariable('live_ws_connected', 'false')
        this._scheduleReconnect()
      })

      this.ws.addEventListener('error', (err) => {
        this.log('error', `WebSocket error: ${err.message}`)
      })
    } catch (error) {
      this.log('error', `Connection error: ${error}`)
      this.setStatus(InstanceStatus.ConnectionFailure, error.message)
      this._scheduleReconnect()
    }
  }

  _scheduleReconnect() {
    if (!this.config.reconnect || this.config.reconnect <= 0) return
    if (this.reconnectTimer) return

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      this._connect()
    }, this.config.reconnect)
  }

  _handleMessage(msg) {
    switch (msg.type) {
      case 'welcome':
        this.log('debug', 'Received welcome from Presenter')
        break
      case 'variables':
        if (Array.isArray(msg.values)) {
          msg.values.forEach(({ name, value }) => {
            this._updateVariable(name, value ?? '')
          })
        }
        break
      case 'ack':
        this.log('debug', `Ack from server: ${msg.command}`)
        break
      case 'error':
        this.log('error', `Presenter error: ${msg.message}`)
        break
      default:
        this.log('debug', `Unhandled message type: ${msg.type}`)
    }
  }

  _updateVariable(name, value) {
    if (!VARIABLE_DEFINITIONS.includes(name)) {
      return
    }
    const previous = this.variables.get(name)
    if (previous !== value) {
      this.variables.set(name, value)
      this.setVariableValues({ [name]: value })
    }
  }

  _setupVariables() {
    const defs = VARIABLE_DEFINITIONS.map((name) => ({ variableId: name, name }))
    this.setVariableDefinitions(defs)
  }

  _setupActions() {
    const actions = {}

    COMMANDS.forEach((cmd) => {
      actions[cmd.id] = {
        name: cmd.label,
        options: this._commandOptionsFor(cmd.id),
        callback: (event) => this._sendCommand(cmd.id, event.options),
      }
    })

    this.setActionDefinitions(actions)
  }

  _commandOptionsFor(commandId) {
    switch (commandId) {
      case 'timer.set_countdown_target':
        return [
          {
            type: 'textinput',
            id: 'target',
            label: 'ISO datetime (yyyy-mm-ddThh:mm:ssZ)',
            placeholder: '2025-10-05T18:00:00Z',
            default: '',
          },
        ]
      case 'stage.set':
        return [
          {
            type: 'textinput',
            id: 'presentationId',
            label: 'Presentation ID',
            default: '',
          },
          {
            type: 'textinput',
            id: 'currentSlideId',
            label: 'Current slide ID',
            default: '',
          },
          {
            type: 'textinput',
            id: 'nextSlideId',
            label: 'Next slide ID (optional)',
            default: '',
          },
          {
            type: 'checkbox',
            id: 'blank',
            label: 'Blank outputs',
            default: false,
          },
        ]
      case 'bible.trigger':
        return [
          {
            type: 'textinput',
            id: 'translation',
            label: 'Translation code',
            default: 'KJV',
          },
          {
            type: 'textinput',
            id: 'book',
            label: 'Book',
            default: 'John',
          },
          {
            type: 'number',
            id: 'chapter',
            label: 'Chapter',
            default: 3,
            min: 1,
          },
          {
            type: 'number',
            id: 'verseStart',
            label: 'Verse start',
            default: 16,
            min: 1,
          },
          {
            type: 'number',
            id: 'verseEnd',
            label: 'Verse end (optional)',
            default: 0,
            min: 0,
          },
        ]
      default:
        return []
    }
  }

  _setupFeedbacks() {
    const feedbacks = {}

    VARIABLE_DEFINITIONS.forEach((name) => {
      feedbacks[`text_${name}`] = {
        type: 'advanced',
        name: `Text equals: ${name}`,
        options: [
          {
            type: 'textinput',
            id: 'value',
            label: 'Expected value',
            default: '',
          },
        ],
        callback: (feedback) => {
          const expected = feedback.options.value ?? ''
          const current = this.variables.get(name) ?? ''
          return current === expected
        },
        style: {
          color: 0xffffff,
          bgcolor: 0xff0000,
        },
      }
    })

    feedbacks['countdown_running'] = {
      type: 'boolean',
      name: 'Countdown running',
      options: [],
      defaultStyle: {
        color: 0xffffff,
        bgcolor: 0x00ff00,
      },
      callback: () => this.variables.get('timer_countdown_state') === 'running',
    }

    this.setFeedbackDefinitions(feedbacks)
  }

  _sendCommand(command, options = {}) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      this.log('error', 'Not connected to Presenter; cannot send command')
      return
    }

    let payload = {}

    switch (command) {
      case 'timer.set_countdown_target':
        payload = {
          target: options.target,
        }
        break
      case 'stage.set':
        payload = {
          presentationId: options.presentationId || null,
          currentSlideId: options.currentSlideId || null,
          nextSlideId: options.nextSlideId || null,
          blank: Boolean(options.blank),
        }
        break
      case 'bible.trigger':
        payload = {
          translation: options.translation || 'KJV',
          book: options.book || 'John',
          chapter: Number(options.chapter) || 3,
          verseStart: Number(options.verseStart) || 1,
        }
        if (Number(options.verseEnd) > 0) {
          payload.verseEnd = Number(options.verseEnd)
        }
        break
      default:
        payload = {}
    }

    const envelope = {
      type: 'command',
      command,
      payload,
    }

    this.ws.send(JSON.stringify(envelope))
  }
}

runEntrypoint(PresenterInstance)
