export const meta = {
  name: 'hve-design-pipeline',
  description: 'HVE 設計パイプライン: ARD → AAS → AAD-WEB を順次実行する',
  phases: [
    { title: 'ARD', detail: '要件定義（事業分析 → ユースケース → アプリカタログ → 要件）' },
    { title: 'AAS', detail: 'アーキテクチャ設計（アーキ選定 → DDD → データモデル → テスト戦略）' },
    { title: 'AAD-WEB', detail: 'Web 詳細設計（画面設計 → サービス設計 → テストスペック → 一貫性レビュー）' },
  ],
}

const ARTIFACT_CHECK_SCHEMA = {
  type: 'object',
  properties: {
    exists: { type: 'boolean', description: '必須ファイルが全て存在するか' },
    missing: {
      type: 'array',
      items: { type: 'string' },
      description: '欠落しているファイルパス',
    },
  },
  required: ['exists', 'missing'],
}

// --- Phase 1: ARD ---
phase('ARD')
log('ARD（要件定義）を開始します')

const ardParams = args || {}
const companyName = ardParams.company_name || 'TBD'
const targetBusiness = ardParams.target_business || ''

const ardResult = await agent(
  `HVE ARD ワークフローを実行してください。

パラメータ:
- company_name: ${companyName}
- target_business: ${targetBusiness || '未指定（事業ポートフォリオスキャンから開始）'}
- survey_base_date: ${ardParams.survey_base_date || 'TBD'}
- survey_period_years: ${ardParams.survey_period_years || 'TBD'}
- target_region: ${ardParams.target_region || 'TBD'}
- analysis_purpose: ${ardParams.analysis_purpose || 'TBD'}
- include_kpi_okr: ${ardParams.include_kpi_okr || false}

スキルファイルを Read して手順に従ってください:
1. Read skills/hve-ard/SKILL.md でワークフロー全体像を確認
2. 各ステップの templates/ を Read してプロンプトを確認
3. DAG の依存関係に従って逐次実行（fan-out ステップは Agent tool で並列化）
4. 全ステップ完了後、作成したファイル一覧を報告`,
  { label: 'ARD', phase: 'ARD' }
)

log('ARD 完了。成果物を確認します')

const ardCheck = await agent(
  `ARD の必須成果物が存在するか確認してください:
- docs/catalog/use-case-catalog.md
- docs/catalog/app-catalog.md
- docs/architectural-requirements-app-*.md（最低 1 ファイル）

各ファイルの存在を ls で確認し、結果を報告してください。`,
  { label: 'ARD:verify', phase: 'ARD', schema: ARTIFACT_CHECK_SCHEMA }
)

if (!ardCheck.exists) {
  log(`ARD の必須成果物が欠落しています: ${ardCheck.missing.join(', ')}`)
  return { status: 'failed', phase: 'ARD', missing: ardCheck.missing }
}

// --- Phase 2: AAS ---
phase('AAS')
log('AAS（アーキテクチャ設計）を開始します')

const aasResult = await agent(
  `HVE AAS ワークフローを実行してください。

前提: ARD が完了済み。以下のファイルが存在します:
- docs/catalog/app-catalog.md
- docs/catalog/use-case-catalog.md
- docs/architectural-requirements-app-*.md

スキルファイルを Read して手順に従ってください:
1. Read skills/hve-aas/SKILL.md でワークフロー全体像を確認
2. 各ステップの templates/ を Read してプロンプトを確認
3. Step 1 → 2.1 → 2.2 → 3.1 → 3.2 → 4 → 5 → 6 → 7 → 8 の順で逐次実行
4. 全ステップ完了後、作成したファイル一覧を報告`,
  { label: 'AAS', phase: 'AAS' }
)

log('AAS 完了。成果物を確認します')

const aasCheck = await agent(
  `AAS の必須成果物が存在するか確認してください:
- docs/catalog/app-arch-catalog.md
- docs/catalog/domain-analytics.md
- docs/catalog/service-catalog.md
- docs/catalog/data-model.md
- docs/catalog/service-catalog-matrix.md
- docs/catalog/test-strategy.md
- docs/catalog/persona-catalog.md

各ファイルの存在を ls で確認し、結果を報告してください。`,
  { label: 'AAS:verify', phase: 'AAS', schema: ARTIFACT_CHECK_SCHEMA }
)

if (!aasCheck.exists) {
  log(`AAS の必須成果物が欠落しています: ${aasCheck.missing.join(', ')}`)
  return { status: 'failed', phase: 'AAS', missing: aasCheck.missing }
}

// --- Phase 3: AAD-WEB ---
phase('AAD-WEB')
log('AAD-WEB（Web 詳細設計）を開始します')

const aadResult = await agent(
  `HVE AAD-WEB ワークフローを実行してください。

前提: AAS が完了済み。docs/catalog/ に各種カタログが存在します。

スキルファイルを Read して手順に従ってください:
1. Read skills/hve-aad-web/SKILL.md でワークフロー全体像を確認
2. 各ステップの templates/ を Read してプロンプトを確認
3. DAG に従って実行（fan-out ステップは Agent tool で並列化）
4. Step 3（一貫性レビュー）は全 Step 2.x 完了後に実行
5. 全ステップ完了後、作成したファイル一覧を報告`,
  { label: 'AAD-WEB', phase: 'AAD-WEB' }
)

log('AAD-WEB 完了。成果物を確認します')

const aadCheck = await agent(
  `AAD-WEB の必須成果物が存在するか確認してください:
- docs/catalog/screen-catalog-APP-*.md（最低 1 ファイル）
- docs/screen/（最低 1 ファイル）
- docs/services/（最低 1 ファイル）
- docs/test-specs/（最低 1 ファイル）
- docs/catalog/screen-service-consistency-report.md

各ディレクトリの ls で確認し、結果を報告してください。`,
  { label: 'AAD-WEB:verify', phase: 'AAD-WEB', schema: ARTIFACT_CHECK_SCHEMA }
)

if (!aadCheck.exists) {
  log(`AAD-WEB の必須成果物が欠落しています: ${aadCheck.missing.join(', ')}`)
  return { status: 'failed', phase: 'AAD-WEB', missing: aadCheck.missing }
}

log('HVE 設計パイプライン完了')
return { status: 'completed', phases: ['ARD', 'AAS', 'AAD-WEB'] }
