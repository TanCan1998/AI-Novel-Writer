import { createGenerationRuntime } from '../../generation/generation-runtime'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useLLMStore } from '../../../stores/llm-store'
import { useProjectStore } from '../../../stores/project-store'
import { useWorkflowStore } from '../../../stores/workflow-store'
import type { CharacterRosterEntry, CharacterRosterSnapshot } from '../../../shared/character-roster'
import { migrateLegacyCharacterRoster } from '../architecture-workflow'
import { buildLegacyRosterTask, buildLegacyRosterJsonRepairTask } from '../../../shared/legacy-roster-generation-pure'
import { installTauriInternals, type TauriInternalsHandle } from '../../../../test/helpers/tauri-internals'

const projectPath = 'C:\\novels\\legacy-roster'
const projectSession = {
  projectId: 'legacy-project',
  leaseId: 'legacy-lease',
  projectPath,
}

const originalGenerateStream = useLLMStore.getState().generateStream
const originalDefaultModelId = useLLMStore.getState().defaultModelId

const repairedEntries: CharacterRosterEntry[] = [
  {
    name: '沈砺',
    role: 'protagonist',
    gender: '男',
    age: '二十六岁',
    appearance: '左手缺一截食指，常穿旧工装',
    personality: '克制而执拗',
    background: '矿场事故幸存者',
    abilities: '机械维修',
    motivation: '查清矿场事故真相',
    relationships: [{ target: '顾湘', relation: '互相试探的盟友' }],
    arc: '从独自复仇到相信同伴',
    notes: '旧图谱未使用可解析标题',
  },
  {
    name: '顾湘',
    role: 'supporting',
    gender: '女',
    age: '二十七岁',
    appearance: '戴银框护目镜',
    personality: '冷静敏锐',
    background: '矿区检修员',
    abilities: '数据解密',
    motivation: '保护失踪的弟弟',
    relationships: [{ target: '沈砺', relation: '互相试探的盟友' }],
    arc: '从旁观者变为行动者',
    notes: '',
  },
]

const pendingLegacyRoster: CharacterRosterSnapshot = {
  schemaVersion: 1,
  revision: 0,
  migrationState: 'legacy_markdown_pending',
  status: 'legacy_repair_required',
  entries: [],
  renderedMarkdown: '',
  projectionHash: '',
  factHash: '',
  nameOnlyFactHash: '',
  legacyMarkdown: '矿场事故后，沈砺与顾湘从互相怀疑走向共同调查。这里故意没有 Markdown 标题或角色列表。',
}

const readyRoster: CharacterRosterSnapshot = {
  schemaVersion: 1,
  revision: 1,
  migrationState: 'ready',
  status: 'ready',
  entries: repairedEntries,
  renderedMarkdown: '# 角色图谱\n\n## 主角：沈砺\n\n## 配角：顾湘',
  projectionHash: 'projection-hash',
  factHash: 'fact-hash',
  nameOnlyFactHash: 'fact-hash',
  legacyMarkdown: pendingLegacyRoster.legacyMarkdown,
}

function activeProject() {
  return {
    id: projectSession.projectId,
    sessionLease: projectSession.leaseId,
    name: '旧项目',
    path: projectPath,
    novelConfig: { genre: '科幻' },
    characterStates: '',
    createdAt: '',
    updatedAt: '',
  }
}

let tauriInternals: TauriInternalsHandle
function installVela(dbCommands: Record<string, unknown>) {
  const lease = {
    leaseId: 'legacy-model-lease',
    modelId: 'legacy-repair-model',
    provider: 'custom',
    protocol: 'openai',
    modelName: 'legacy-repair-model',
    modelRevision: 'a'.repeat(64),
    endpointFingerprint: 'b'.repeat(64),
    capabilityEvidence: {
      source: {
        contextWindowTokens: 'unknown',
        maxOutputTokens: 'user-operational-cap',
        featureFlags: 'unknown',
      },
      subjectFingerprint: 'c'.repeat(64),
      contextWindowTokens: null,
      maxOutputTokens: 8192,
      reasoning: null,
      structuredOutput: true,
      usage: null,
    },
    createdAt: 1000,
    expiresAt: 61_000,
  }
  tauriInternals = installTauriInternals({
    commands: {
      llm_begin_execution_lease: { success: true, lease },
      llm_close_execution_lease: { success: true },
      fs_list_dir: (args: Record<string, unknown>) => {
        if (args.dirPath === `${projectPath}/.lore/skills`) return []
        throw new Error(`测试未配置桌面目录：${String(args.dirPath)}`)
      },
      fs_check_exists: false,
      ...dbCommands,
    },
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  useProjectStore.setState({ currentProject: activeProject() as never })
  useWorkflowStore.setState({ activeRuns: [], history: [], globalLogs: [] })
  useLLMStore.setState({ defaultModelId: 'legacy-repair-model' })
})

afterEach(() => {
  useProjectStore.setState({ currentProject: null })
  useWorkflowStore.setState({ activeRuns: [], history: [], globalLogs: [] })
  useLLMStore.setState({
    defaultModelId: originalDefaultModelId,
    generateStream: originalGenerateStream,
  })
  tauriInternals.uninstall()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('legacy character roster repair public workflow seam', () => {
  it('repairs a zero-card legacy graph through one structured roster commit without parsing Markdown headings', async () => {
    const modelResponse = JSON.stringify({ schemaVersion: 1, entries: repairedEntries })
    const generateStream = vi.fn((
      _messages: Parameters<typeof originalGenerateStream>[0],
      callbacks: Parameters<typeof originalGenerateStream>[1],
    ) => {
      callbacks.onChunk?.(modelResponse)
      callbacks.onDone?.(modelResponse, undefined, 'stop')
      return Promise.resolve('legacy-repair-request')
    })
    useLLMStore.setState({ generateStream })

    installVela({
      db_character_roster_read: pendingLegacyRoster,
      db_character_roster_commit: {
        success: true,
        receipt: {
          operationId: expect.any(String),
          payloadHash: 'payload-hash',
          revision: 1,
          idempotent: false,
          snapshot: readyRoster,
        },
      },
    })

    await migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })

    expect(generateStream).toHaveBeenCalledOnce()
    expect(generateStream.mock.calls[0]?.[0]).toEqual(buildLegacyRosterTask({ legacyMarkdown: pendingLegacyRoster.legacyMarkdown!, genre: '科幻' }).messages)
    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'db_character_roster_commit',
      {
        request: expect.objectContaining({
          expectedRevision: 0,
          schemaVersion: 1,
          entries: repairedEntries,
          intent: 'legacy_repair',
        }),
        expectedProjectPath: projectPath,
        projectSession,
      },
      undefined,
    )
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command)).not.toContain('db_character_save_all')
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command)).not.toContain('db_project_core_update')
    expect(useWorkflowStore.getState().history[0]).toMatchObject({
      type: 'post_process',
      status: 'completed',
      steps: [expect.objectContaining({ status: 'completed' })],
    })
  })

  it('allows exactly one JSON syntax repair without overriding the configured model sampling, then commits through the same roster seam', async () => {
    const malformed = '{"schemaVersion":1,"entries":['
    const repaired = JSON.stringify({ schemaVersion: 1, entries: repairedEntries })
    const generateStream = vi.fn((
      _messages: Parameters<typeof originalGenerateStream>[0],
      callbacks: Parameters<typeof originalGenerateStream>[1],
    ) => {
      const output = generateStream.mock.calls.length === 1 ? malformed : repaired
      callbacks.onDone?.(output, undefined, 'stop')
      return Promise.resolve(`legacy-json-${generateStream.mock.calls.length}`)
    })
    useLLMStore.setState({ generateStream })
    installVela({
      db_character_roster_read: pendingLegacyRoster,
      db_character_roster_commit: {
        success: true,
        receipt: { operationId: 'repair-json', payloadHash: 'hash', revision: 1, idempotent: false, snapshot: readyRoster },
      },
    })

    await migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })

    expect(generateStream).toHaveBeenCalledTimes(2)
    expect(generateStream.mock.calls[1]?.[0]).toEqual(buildLegacyRosterJsonRepairTask(malformed).messages)
    const repairOptions = (generateStream.mock.calls as unknown as unknown[][])[1]?.[3]
    expect(repairOptions).toMatchObject({
      purpose: 'legacy-character-roster-json-repair',
    })
    expect(repairOptions).not.toHaveProperty('temperature')
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command).filter(command => command.startsWith('db_character_roster'))).toEqual([
      'db_character_roster_read',
      'db_character_roster_read',
      'db_character_roster_commit',
    ])
  })

  it('fails closed without parsing or committing when bounded structured continuations remain truncated', async () => {
    const generateStream = vi.fn((
      _messages: Parameters<typeof originalGenerateStream>[0],
      callbacks: Parameters<typeof originalGenerateStream>[1],
    ) => {
      callbacks.onDone?.('{"schemaVersion":1,"entries":[', undefined, 'length')
      return Promise.resolve('legacy-truncated')
    })
    useLLMStore.setState({ generateStream })
    installVela({ db_character_roster_read: pendingLegacyRoster })

    await expect(migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })).rejects.toThrow('已自动续写 2 次，尚未完整生成')
    expect(generateStream).toHaveBeenCalledTimes(3)
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command).filter(command => command.startsWith('db_character_roster'))).toEqual(['db_character_roster_read'])
  })
  it('keeps the existing three initial plus three syntax-repair request ceiling in one runtime', async () => {
    const generateStream = vi.fn((
      _messages: Parameters<typeof originalGenerateStream>[0],
      callbacks: Parameters<typeof originalGenerateStream>[1],
    ) => {
      const count = generateStream.mock.calls.length
      callbacks.onDone?.(count === 6 ? JSON.stringify({ schemaVersion: 1, entries: repairedEntries }) : '{"schemaVersion":1,"entries":[',
        undefined, count === 3 || count === 6 ? 'stop' : 'length')
      return Promise.resolve(`bounded-${count}`)
    })
    useLLMStore.setState({ generateStream })
    installVela({
      db_character_roster_read: pendingLegacyRoster,
      db_character_roster_commit: { success: true, receipt: {
        operationId: 'six-requests', payloadHash: 'hash', revision: 1, idempotent: false, snapshot: readyRoster,
      } },
    })
    await migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })
    expect(generateStream).toHaveBeenCalledTimes(6)
    expect((generateStream.mock.calls as unknown as unknown[][]).map(call => (call[3] as { purpose: string }).purpose)).toEqual([
      ...Array(3).fill('legacy-character-roster-repair'), ...Array(3).fill('legacy-character-roster-json-repair'),
    ])
    expect(tauriInternals.invoke.mock.calls.filter(([command]) => command === 'db_character_roster_commit')).toHaveLength(1)
  })

  it('adopts protected existing cards without a model call, then rebuilds only the read-only projection', async () => {
    const existingCards: CharacterRosterSnapshot = {
      ...pendingLegacyRoster,
      migrationState: 'legacy_cards_preserved',
      status: 'inconsistent',
      entries: repairedEntries,
    }
    const adoptedRoster: CharacterRosterSnapshot = {
      ...readyRoster,
      legacyMarkdown: existingCards.legacyMarkdown,
    }
    const generateStream = vi.fn()
    useLLMStore.setState({ generateStream })
    installVela({
      db_character_roster_read: existingCards,
      db_character_roster_commit: {
        success: true,
        receipt: { operationId: 'adopt-cards', payloadHash: 'hash', revision: 1, idempotent: false, snapshot: adoptedRoster },
      },
    })

    await migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })

    expect(generateStream).not.toHaveBeenCalled()
    expect(tauriInternals.invoke).toHaveBeenCalledWith(
      'db_character_roster_commit',
      {
        request: expect.objectContaining({ intent: 'legacy_cards_adoption', entries: repairedEntries }),
        expectedProjectPath: projectPath,
        projectSession,
      },
      undefined,
    )
  })

  it('does not commit a completed candidate when the frozen project session changes', async () => {
    const response = JSON.stringify({ schemaVersion: 1, entries: repairedEntries })
    let finishGeneration: (() => void) | undefined
    const generateStream = vi.fn((
      _messages: Parameters<typeof originalGenerateStream>[0],
      callbacks: Parameters<typeof originalGenerateStream>[1],
    ) => {
      finishGeneration = () => callbacks.onDone?.(response, undefined, 'stop')
      return Promise.resolve('legacy-switch')
    })
    useLLMStore.setState({ generateStream })
    installVela({ db_character_roster_read: pendingLegacyRoster })

    const execution = migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })
    await vi.waitFor(() => expect(generateStream).toHaveBeenCalledOnce())
    useProjectStore.setState({
      currentProject: {
        ...activeProject(),
        path: 'C:\\novels\\other-project',
        sessionLease: 'other-lease',
      } as never,
    })
    finishGeneration?.()

    await expect(execution).rejects.toThrow('当前项目已切换，旧角色图谱修复已停止以避免写入错误项目')
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command).filter(command => command.startsWith('db_character_roster'))).toEqual(['db_character_roster_read'])
  })

  it('does not commit when cancellation reaches the repair before its atomic boundary', async () => {
    let finishGeneration: (() => void) | undefined
    const generateStream = vi.fn((
      _messages: Parameters<typeof originalGenerateStream>[0],
      callbacks: Parameters<typeof originalGenerateStream>[1],
    ) => {
      finishGeneration = () => callbacks.onDone?.(JSON.stringify({ schemaVersion: 1, entries: repairedEntries }), undefined, 'stop')
      return Promise.resolve('legacy-cancel')
    })
    useLLMStore.setState({ generateStream })
    installVela({ db_character_roster_read: pendingLegacyRoster })

    const execution = migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })
    await vi.waitFor(() => expect(generateStream).toHaveBeenCalledOnce())
    const runId = useWorkflowStore.getState().activeRuns[0]?.id
    expect(runId).toBeTruthy()
    useWorkflowStore.getState().cancelWorkflow(runId)
    finishGeneration?.()

    await expect(execution).rejects.toThrow('工作流已取消')
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command).filter(command => command.startsWith('db_character_roster'))).toEqual(['db_character_roster_read'])
  })

  it('leaves semantic validation to the atomic seam and never falls back to Markdown parsing', async () => {
    const generateStream = vi.fn((
      _messages: Parameters<typeof originalGenerateStream>[0],
      callbacks: Parameters<typeof originalGenerateStream>[1],
    ) => {
      callbacks.onDone?.('{"schemaVersion":1,"entries":[]}', undefined, 'stop')
      return Promise.resolve('legacy-semantic-invalid')
    })
    useLLMStore.setState({ generateStream })
    installVela({
      db_character_roster_read: pendingLegacyRoster,
      db_character_roster_commit: { success: false, error: '角色名单不能为空' },
    })

    await expect(migrateLegacyCharacterRoster(projectPath, { generationDependencies: { createRuntime: options => createGenerationRuntime(options) } })).rejects.toThrow('角色名单不能为空')
    expect(generateStream).toHaveBeenCalledOnce()
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command).filter(command => command.startsWith('db_character_roster'))).toEqual([
      'db_character_roster_read',
      'db_character_roster_read',
      'db_character_roster_commit',
    ])
    expect(tauriInternals.invoke.mock.calls.map(([command]) => command)).not.toContain('db_character_save_all')
  })
})
