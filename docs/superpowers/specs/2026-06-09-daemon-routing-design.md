# 데몬 라우팅 확장 설계 — impact / diff / inconsistencies

> **상태: 구현 완료 (v0.1.3, 커밋 `e5fbc33` · `999fae2`).** 이 문서는 작업 전 설계 기록으로 보존한다.
> 실제 구현과의 차이 두 가지: (1) in-process 경로의 project root canonical 통일(`999fae2`)은 이 설계 이후 추가됐다. (2) §2의 `effective_depth = 1` 제거(forward된 `depth` 신뢰)로 MCP `impact`의 `transitive` 플래그는 출력 라벨만 바꾸게 됐다 — 기본은 `depth=5` 전이 폐포를 반환하므로 직접(depth-1) 의존성만 보려면 `depth:1`을 명시해야 한다.

## 배경

18개 CLI 명령 전수 감사 결과, 데몬 IPC 경로(thin-client)를 추가로 태울 수 있는 read-analysis 후보는 `impact` / `diff` / `inconsistencies` 세 개다. 셋 다 데몬 핸들러가 존재하지만 현재 CLI는 매 호출마다 `build_graph`로 그래프를 in-process 재빌드하고, 데몬 핸들러는 CLI 출력을 그대로 재현하지 못한다(드롭인 불가). 기존에 안전하게 라우팅된 `orphans`는 `render_orphans` 공유 렌더러를 단일 진실로 두어 CLI·데몬·extension JSON을 통일했다 — 이 패턴을 세 명령에 적용한다.

## 목표 / 비목표

**목표**
- `impact` / `diff` / `inconsistencies`를 데몬 캐시 경로로 가속한다.
- 모든 소비자(CLI / MCP / VSCode extension)의 출력을 불변으로 유지하거나 단일 형식으로 통일한다.
- thin-client 라우팅 패턴의 복붙을 공통 헬퍼로 제거한다(기존 `query`/`orphans`/`stats` 포함).

**비목표**
- 데몬 핸들러가 없는 명령(`export`/`batch` 등)에 새 프로토콜을 추가하지 않는다.
- `orphans`/`stats`의 freshness 정책(watcher-only)을 바꾸지 않는다.
- extension의 `"diff"`(rich per-file) 계약을 바꾸지 않는다.

## 소비자 지도 (census)

| 명령 | 데몬 핸들러 | 직접 소비자 | 변경 안전성 |
|---|---|---|---|
| `impact` | `cached_impact` | MCP만(`response.body`를 텍스트로 그대로 전달). extension은 `impact_batch`(별개 메서드) | 자유 — CLI 출력에 맞춰 수정 가능 |
| `inconsistencies` | `cached_inconsistencies` | extension `diagnosticsProvider`가 `output_format:json` 직접 파싱 + MCP | extension 묶임 → canonical 단일화 + extension TS 수정 |
| `diff` | `dispatch_diff_with_git`(`"diff"`) | extension 5곳(gutter/commitWarning/fileDecoration/reviewPreview/diffImpact) | 보존 필수 → CLI는 별도 메서드 |

MCP(`handle_tool_call`)는 데몬 `response.body`를 그대로 MCP 텍스트 결과로 넘기므로 출력 형식 변경에 영향받지 않는다(오히려 CLI와 통일되면 개선).

## 핵심 원칙: 공유 렌더러

각 명령에 대해 `pub(crate) fn render_<cmd>(...)`를 단일 진실로 추출하고, CLI in-process 경로와 데몬 `cached_<cmd>` 핸들러가 **동일 함수**를 호출한다. `--output-format {human,llm,json}`이 어느 경로에서든 바이트 동일한 출력을 내는 것이 동치성 계약이다. `render_orphans`(dispatch.rs)가 모범 사례.

## 구성요소

### 1. 공통 thin-client 헬퍼
`try_daemon(method, params, globals) -> Option<String>` — `ensure_running` → `send` → `resp.ok`면 `Some(body)`, 아니면 `None`. `query`/`orphans`/`stats`/`impact`/`inconsistencies`가 공유한다. 기존 3개는 동작 보존(회귀 테스트)을 전제로 이 헬퍼로 마이그레이션한다. 재현 불가 케이스(예: `--expand`, 비어있지 않은 `--lang` 일부)는 호출부에서 게이트해 in-process로 폴백한다.

### 2. impact
- `cached_impact` 수정:
  - non-transitive일 때 `effective_depth = 1` 하드코딩 제거 → forward된 `depth`를 신뢰(CLI는 `hop_limit`).
  - `project root`를 주입받아(`server.rs`가 `cached_orphans`처럼 root 전달) `--lang` 필터와 `PathExcluder`를 reachable·affected_tests에 적용.
  - JSON top-level key `seed` → `symbol`, `nodes[]` 배열 복원, full 4-factor risk(`visibility_score`/`caller_factor`/`module_factor`/`impact_kind_factor` + 전체 `affected_tests`).
  - seed 매칭 exact → exact-then-substring 폴백.
  - `Llm` 분기 추가(markdown 테이블 + `### Affected tests`).
- `render_impact` 추출 → `impact.rs`와 `cached_impact` 공유.
- `impact.rs::run()`에 `try_daemon` 경로 추가.

### 3. inconsistencies
- `render_inconsistencies` 추출: `strip_marker`, kebab `category.label()`, canonical JSON 형식.
- **Canonical JSON 단일화**: stripped name + kebab category + `file` + `line` + `count` 래퍼. CLI·데몬·extension이 공유.
- `cached_inconsistencies`가 `project root`를 받아 `--lang`/`PathExcluder` 적용, `--category doc-drift`일 때 `find_doc_param_drift`를 데몬에서도 호출(현재 로컬 전용 → 데몬 핸들러로 승격).
- `inconsistencies.rs::run()`에 `try_daemon` 경로 추가.
- **extension 수정**: `InconsistenciesResponse` 타입 + `buildDiagnostics`를 canonical JSON에 맞춘다.

### 4. diff
- extension용 `dispatch_diff_with_git`(`"diff"`)은 **그대로 보존**.
- CLI diff는 계산(touched union + reachable count)·출력이 달라 **새 데몬 메서드** `"diff_summary"`로 분리: `diff.rs`의 계산 로직과 `render_diff`를 데몬 핸들러로 이식. `to`/`max_depth`/`exclude_tests`를 forward하고 honor.
- `diff.rs::run()`에 `try_daemon("diff_summary", ...)` 경로 추가.
- 셋 중 가장 무겁다(새 메서드 + git-aware healing).

### 5. healing
- `server.rs`의 on-demand healing 블록을 **새 3개 메서드(`impact`/`inconsistencies`/`diff_summary`)로만** 확장한다(`no_heal` 존중).
- `orphans`/`stats`는 현행 watcher-only 유지(기존 동작 불변).
- `diff_summary`는 git 변경 파일을 heal 후보에 포함해, 편집 직후 실행 시 stale blast radius를 방지한다.

## 출력 동치성 계약

- `impact`/`inconsistencies`/`diff`: 로컬 in-process 출력 == 데몬 출력(human/llm/json 모두), 바이트 동일.
- `inconsistencies` json: canonical 형식 == extension `InconsistenciesResponse`.
- 데몬이 구조적으로 재현 불가한 경우만 client-side 게이트로 in-process 폴백(최소화).

## 테스트 전략

- 명령별 로컬==데몬 동치성 테스트(human/llm/json).
- impact: depth(transitive/비-transitive), `--lang`, PathExcluder, seed substring 폴백, full-risk JSON.
- inconsistencies: 카테고리별, doc-drift 데몬 경로, strip_marker, canonical JSON 스키마.
- diff: `to`/`max_depth`/`exclude_tests`, healing으로 갓 수정한 파일 반영.
- extension: canonical `InconsistenciesResponse` 호환(타입/파싱).
- 공통 헬퍼 마이그레이션 후 query/orphans/stats 회귀 무.

## 리스크

- `diff_summary` 신규 메서드와 git-aware healing이 가장 큰 작업.
- healing 확장이 새 3개 메서드의 매 요청에 변경 파일 체크를 추가한다(query는 이미 동일 비용).
- extension TS 변경(`InconsistenciesResponse`/`buildDiagnostics`)으로 extension 빌드/테스트 확인 필요.
- `server.rs` dispatch 분기에 impact/inconsistencies/diff_summary의 root 주입 경로 추가.
