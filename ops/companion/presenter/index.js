const { InstanceBase, InstanceStatus, runEntrypoint } = require('@companion-module/base')
const WebSocket = require('ws')

const VARIABLE_DEFINITIONS = [
  'stage_layout_code',
  'stage_layout_name',
  'stage_layout_description',
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
  'timer_countdown_remaining_hms',
  'timer_countdown_remaining_mmss',
  'timer_countdown_remaining_hhmm',
  'timer_countdown_remaining_readable',
  'timer_preach_state',
  'timer_preach_elapsed_seconds',
  'timer_preach_elapsed_hms',
  'timer_preach_elapsed_mmss',
  'timer_preach_elapsed_hhmm',
  'timer_preach_elapsed_readable',
  'bible_translation_code',
  'bible_translation_name',
  'bible_reference',
  'bible_text',
  'bible_triggered_at',
  'live_ws_connected',
]

const STAGE_LAYOUT_CHOICES = [
  { id: 'worship-snv', label: 'WORSHIP SNV' },
  { id: 'worship-pp', label: 'WORSHIP PP' },
  { id: 'timer', label: 'TIMER' },
  { id: 'preach', label: 'PREACH' },
]

const BASE_COMMANDS = [
  { id: 'timer.start_countdown', command: 'timer.start_countdown', label: 'Timer: start countdown' },
  { id: 'timer.pause_countdown', command: 'timer.pause_countdown', label: 'Timer: pause countdown' },
  { id: 'timer.reset_countdown', command: 'timer.reset_countdown', label: 'Timer: reset countdown' },
  { id: 'timer.set_countdown_target', command: 'timer.set_countdown_target', label: 'Timer: set countdown duration (HH:MM)' },
  { id: 'timer.start_preach', command: 'timer.start_preach', label: 'Timer: start preach' },
  { id: 'timer.reset_preach', command: 'timer.reset_preach', label: 'Timer: reset preach' },
  { id: 'stage.layout', command: 'stage.layout', label: 'Stage: set layout' },
  { id: 'bible.trigger', command: 'bible.trigger', label: 'Bible: trigger passage' },
  { id: 'bible.clear', command: 'bible.clear', label: 'Bible: clear passage' },
]

const LAYOUT_COMMANDS = STAGE_LAYOUT_CHOICES.map((choice) => ({
  id: `stage.layout.${choice.id}`,
  command: 'stage.layout',
  label: `Stage: layout ${choice.label}`,
  layoutCode: choice.id,
}))

const COMMANDS = [...BASE_COMMANDS, ...LAYOUT_COMMANDS]

function parseDurationHhMm(value) {
  if (typeof value !== 'string') {
    return null
  }
  const trimmed = value.trim()
  if (!/^\d{1,2}:\d{2}$/.test(trimmed)) {
    return null
  }
  const [hoursStr, minutesStr] = trimmed.split(':')
  const hours = Number.parseInt(hoursStr, 10)
  const minutes = Number.parseInt(minutesStr, 10)
  if (!Number.isFinite(hours) || !Number.isFinite(minutes)) {
    return null
  }
  if (minutes < 0 || minutes > 59 || hours < 0 || hours > 99) {
    return null
  }
  const totalSeconds = (hours * 60 + minutes) * 60
  if (totalSeconds <= 0) {
    return null
  }
  return totalSeconds
}

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
        type: 'static-text',
        id: 'info',
        label: 'Presenter Companion Service',
        width: 12,
        value:
          'Enable the Companion websocket inside Presenter (Settings → Services) and assign a unique port. Point this module at the matching host and port; defaults target the demo container.',
      },
      {
        type: 'textinput',
        id: 'host',
        label: 'Host or IP',
        width: 6,
        default: '10.77.9.21',
        placeholder: '10.77.9.21',
      },
      {
        type: 'number',
        id: 'port',
        label: 'Port',
        width: 3,
        min: 1,
        max: 65535,
        default: 18175,
      },
      {
        type: 'checkbox',
        id: 'secure',
        label: 'Use TLS (wss://)',
        width: 3,
        default: false,
      },
      {
        type: 'textinput',
        id: 'token',
        label: 'Token (optional)',
        width: 6,
        default: '',
      },
      {
        type: 'number',
        id: 'reconnect',
        label: 'Auto-reconnect (ms)',
        width: 6,
        default: 2000,
        min: 0,
      },
    ]
  }

  async init(config) {
    this.config = config
    this.updateStatus(InstanceStatus.Connecting)
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
      this.updateStatus(InstanceStatus.BadConfig, 'Missing host or port')
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
        this.updateStatus(InstanceStatus.Ok)

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
        this.updateStatus(InstanceStatus.Disconnected, `Closed ${event.code}`)
        this._updateVariable('live_ws_connected', 'false')
        this._scheduleReconnect()
      })

      this.ws.addEventListener('error', (err) => {
        this.log('error', `WebSocket error: ${err.message}`)
      })
    } catch (error) {
      this.log('error', `Connection error: ${error}`)
      this.updateStatus(InstanceStatus.ConnectionFailure, error.message)
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
      const commandId = cmd.command || cmd.id
      const options = cmd.layoutCode ? [] : this._commandOptionsFor(commandId)
      actions[cmd.id] = {
        name: cmd.label,
        options,
        callback: (event) => this._sendCommand(cmd, event.options || {}),
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
            label: 'Countdown duration (HH:MM)',
            placeholder: '00:15',
            default: '00:15',
          },
        ]
      case 'stage.layout':
        return [
          {
            type: 'dropdown',
            id: 'code',
            label: 'Stage layout',
            default: 'worship-snv',
            choices: STAGE_LAYOUT_CHOICES,
            allowCustom: true,
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

  _sendCommand(descriptor, options = {}) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      this.log('error', 'Not connected to Presenter; cannot send command')
      return
    }

    const info = typeof descriptor === 'string' ? { id: descriptor, command: descriptor } : descriptor
    const command = info.command || info.id
    const layoutCode = info.layoutCode

    let payload = {}

    switch (command) {
      case 'timer.set_countdown_target': {
        const seconds = parseDurationHhMm(options.target || '')
        if (typeof seconds !== 'number' || !Number.isFinite(seconds)) {
          this.log('error', 'Invalid duration. Use HH:MM, e.g. 00:30 for 30 minutes.')
          return
        }
        const targetDate = new Date(Date.now() + seconds * 1000)
        payload = {
          target: targetDate.toISOString(),
        }
        break
      }
      case 'stage.layout': {
        const code = layoutCode || options.code || 'worship-snv'
        payload = {
          code,
        }
        break
      }
      case 'bible.trigger': {
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
      }
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
