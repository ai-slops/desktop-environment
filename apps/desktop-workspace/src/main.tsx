import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import {
  DockviewReact,
  themeDark,
  type DockviewApi,
  type DockviewReadyEvent,
  type IDockviewPanelProps,
  type SerializedDockview,
} from "dockview-react";
import "dockview-react/dist/styles/dockview.css";
import "./style.css";
import {
  NativeAudio,
  NativeCablePanel,
  NativeWindows,
  NativeJobLog,
} from "./native-panels.tsx";
import type { NativeCommand } from "./native-protocol.ts";
import {
  backend,
  resolvedDeviceId,
  sameOutput,
  type Command,
  type Device,
  type Phase,
} from "./workspace.ts";

const titles = {
  audio: "Desktop Control",
  cable: "Audio Cable",
  windows: "Window Manager",
  diagnostics: "진단 · 실행 기록",
};
type PanelId = keyof typeof titles;
const layoutKey = "desktop-workspace.layout.v1";
const useWorkspace = () =>
  useSyncExternalStore(backend.subscribe, backend.getSnapshot);
const send = (command: Command) => {
  void backend.dispatch(command);
};
const phaseLabel: Record<Phase, string> = {
  stopped: "중지됨",
  starting: "초기화 중",
  running: "테스트 실행 중",
  failed: "실패",
  recovering: "장치 복구 중",
};

function Status({ phase }: { phase: Phase }) {
  return (
    <span className={`status ${phase}`}>
      <i />
      {phaseLabel[phase]}
    </span>
  );
}
function PanelTools(props: IDockviewPanelProps) {
  const [location, setLocation] = useState(props.api.location.type);
  const [error, setError] = useState("");
  useEffect(() => {
    const subscription = props.api.onDidLocationChange(() =>
      setLocation(props.api.location.type),
    );
    return () => subscription.dispose();
  }, [props.api]);
  return (
    <div className="panel-tools">
      <span className="eyebrow">
        {location === "popout"
          ? "분리된 창 · 동일한 작업 상태"
          : "탭을 끌어 분할하거나 다른 창으로 이동"}
      </span>
      {location === "popout" ? (
        <button
          onClick={() => {
            const reference =
              props.containerApi.groups.find(
                (group) => group.api.location.type === "grid",
              ) ?? props.containerApi.addGroup();
            props.api.group.api.moveTo({ group: reference, position: "right" });
          }}
        >
          주 창으로 합치기 ↙
        </button>
      ) : (
        <button
          onClick={async () => {
            const panel = props.containerApi.getPanel(props.api.id);
            if (!panel) return;
            const opened = await props.containerApi.addPopoutGroup(panel, {
              popoutUrl: "/popout.html",
              position: { left: 180, top: 100, width: 760, height: 680 },
            });
            if (!opened)
              setError(
                "새 창을 열지 못했습니다. 이 사이트의 팝업을 허용해 주세요.",
              );
          }}
        >
          새 창으로 이동 ↗
        </button>
      )}
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
    </div>
  );
}
function DeviceChoices({
  label,
  devices,
  selected,
  onSelect,
}: {
  label: string;
  devices: Device[];
  selected: string;
  onSelect: (id: string) => void;
}) {
  return (
    <fieldset className="choices">
      <legend>
        {label} <small>하나 선택</small>
      </legend>
      {devices.map((device) => (
        <label
          key={device.id}
          className={`choice ${selected === device.id ? "selected" : ""}`}
        >
          <input
            type="radio"
            name={label}
            value={device.id}
            checked={selected === device.id}
            onChange={() => onSelect(device.id)}
          />
          <span className="choice-copy">
            <strong>{device.name}</strong>
            <small>{device.detail}</small>
          </span>
          {selected === device.id && (
            <span className="selected-badge">선택됨</span>
          )}
        </label>
      ))}
    </fieldset>
  );
}
function AudioPanel(props: IDockviewPanelProps) {
  const state = useWorkspace();
  if (state.mode === "native")
    return <NativePanel type="audio" dockProps={props} />;
  const audio = state.audio;
  const name = (id: string) =>
    state.devices.find((device) => device.id === id)?.name ?? id;
  const invalid = sameOutput(audio.source, audio.target);
  return (
    <section className="panel-body" aria-label="Desktop Control 패널">
      <PanelTools {...props} />
      <div className="section-heading">
        <div>
          <h1>오디오 출력 복제</h1>
          <p>원본의 소리를 다른 출력으로 복제합니다.</p>
        </div>
        <Status phase={audio.phase} />
      </div>
      <div className="route-summary" aria-label="현재 선택">
        <span>{name(audio.source)}</span>
        <b>→</b>
        <span>{name(audio.target)}</span>
      </div>
      <div className="device-columns">
        <DeviceChoices
          label="원본 출력"
          devices={state.devices}
          selected={audio.source}
          onSelect={(id) => send({ kind: "audio.select", role: "source", id })}
        />
        <DeviceChoices
          label="대상 출력"
          devices={state.devices}
          selected={audio.target}
          onSelect={(id) => send({ kind: "audio.select", role: "target", id })}
        />
      </div>
      {invalid && (
        <p role="alert" className="inline-error">
          원본과 대상이 같은 출력입니다. 다른 대상을 선택하세요.
        </p>
      )}
      {audio.error && (
        <div className="error-card" role="alert">
          <strong>{audio.error.message}</strong>
          <code>{audio.error.code}</code>
          <p>{audio.error.remedy}</p>
          <small>
            요청 {audio.error.requestId} · {audio.error.operation}
          </small>
        </div>
      )}
      <div className="action-row">
        <button
          className="primary"
          disabled={
            invalid || audio.phase === "running" || audio.phase === "starting"
          }
          onClick={() => send({ kind: "audio.start" })}
        >
          테스트 라우팅 시작
        </button>
        <button
          disabled={audio.phase !== "running" && audio.phase !== "starting"}
          onClick={() => send({ kind: "audio.stop" })}
        >
          중지
        </button>
      </div>
      <div className="fixture-controls">
        <label htmlFor="audio-scenario">오류 재현 시나리오</label>
        <select
          id="audio-scenario"
          value={audio.scenario}
          onChange={(event) =>
            send({
              kind: "audio.scenario",
              scenario: event.target.value as typeof audio.scenario,
            })
          }
        >
          <option value="normal">정상 초기화</option>
          <option value="exclusive">장치 독점 사용 · 0x8889000A</option>
          <option value="disconnected">대상 연결 끊김</option>
        </select>
        <p>가상 장치만 사용합니다. 선택을 변경하면 테스트 작업이 중지됩니다.</p>
      </div>
      <div className="feature-note">
        <strong>디스플레이 미러링</strong>
        <p>
          실제 미러링 영상은 네이티브 창에 유지합니다. 웹 UI에서 미러링을
          실행하는 기능은 아직 연결되지 않았습니다.
        </p>
      </div>
    </section>
  );
}
function CablePanel(props: IDockviewPanelProps) {
  const state = useWorkspace();
  if (state.mode === "native")
    return <NativePanel type="cable" dockProps={props} />;
  const cable = state.cable;
  const invalid = resolvedDeviceId(cable.output) === "cable";
  return (
    <section className="panel-body" aria-label="Audio Cable 패널">
      <PanelTools {...props} />
      <div className="section-heading">
        <div>
          <h1>가상 케이블 모니터링</h1>
          <p>케이블의 녹음 출력을 선택한 장치로 듣습니다.</p>
        </div>
        <Status phase={cable.phase} />
      </div>
      <div className="cable-path">
        <span className="device-glyph">AC</span>
        <div>
          <strong>테스트 케이블</strong>
          <p>
            Playback → Recording <span className="tag">가상 장치</span>
          </p>
        </div>
      </div>
      <DeviceChoices
        label="모니터링 출력"
        devices={state.devices}
        selected={cable.output}
        onSelect={(id) => send({ kind: "cable.output", id })}
      />
      <div className="volume-control">
        <label htmlFor="volume">
          모니터링 볼륨 <output>{cable.volume}%</output>
        </label>
        <input
          id="volume"
          type="range"
          min="0"
          max="100"
          value={cable.volume}
          onChange={(event) =>
            send({ kind: "cable.volume", value: Number(event.target.value) })
          }
        />
        <label className="check-label">
          <input
            type="checkbox"
            checked={cable.muted}
            onChange={(event) =>
              send({ kind: "cable.mute", muted: event.target.checked })
            }
          />
          음소거 {cable.muted && <span className="selected-badge">켜짐</span>}
        </label>
      </div>
      {invalid && (
        <p className="inline-error" role="alert">
          같은 케이블로 보내면 피드백이 발생합니다. 다른 출력을 선택하세요.
        </p>
      )}
      <div className="action-row">
        <button
          className="primary"
          disabled={invalid || cable.phase === "running"}
          onClick={() => send({ kind: "cable.start" })}
        >
          테스트 모니터링 시작
        </button>
        <button
          disabled={cable.phase !== "running"}
          onClick={() => send({ kind: "cable.stop" })}
        >
          중지
        </button>
      </div>
      <p className="muted">
        이 패널을 이동하거나 닫아도 공유 작업 상태는 유지됩니다.
      </p>
    </section>
  );
}
function WindowsPanel(props: IDockviewPanelProps) {
  const state = useWorkspace();
  if (state.mode === "native")
    return <NativePanel type="windows" dockProps={props} />;
  const selectedCount = state.windows.filter(
    (window) => window.selected,
  ).length;
  return (
    <section className="panel-body" aria-label="Window Manager 패널">
      <PanelTools {...props} />
      <div className="section-heading">
        <div>
          <h1>창 배치</h1>
          <p>선택 → 미리보기 → 적용 순서로 배치합니다.</p>
        </div>
        <span className="tag">가상 창 {state.windows.length}개</span>
      </div>
      <fieldset className="view-options">
        <legend>
          배치 프리셋 <small>하나 선택</small>
        </legend>
        {(["coding", "monitoring"] as const).map((view) => (
          <label
            className={`view-card ${state.view === view ? "selected" : ""}`}
            key={view}
          >
            <input
              type="radio"
              name="배치 프리셋"
              checked={state.view === view}
              onChange={() => send({ kind: "windows.view", view })}
            />
            <span className={`layout-thumbnail ${view}`}>
              <i />
              <i />
            </span>
            <span>
              <strong>{view === "coding" ? "코딩 작업" : "모니터링"}</strong>
              <small>
                {view === "coding" ? "좌우 50 : 50" : "메인 70 : 보조 30"}
              </small>
            </span>
            {state.view === view && (
              <span className="selected-badge">선택됨</span>
            )}
          </label>
        ))}
      </fieldset>
      <fieldset className="window-list">
        <legend>
          배치할 창 <small>여러 개 선택 · {selectedCount}개 선택됨</small>
        </legend>
        {state.windows.map((window) => (
          <label
            className={`window-row ${window.selected ? "selected" : ""}`}
            key={window.id}
          >
            <input
              type="checkbox"
              checked={window.selected}
              onChange={(event) =>
                send({
                  kind: "windows.select",
                  id: window.id,
                  selected: event.target.checked,
                })
              }
            />
            <span>
              <strong>{window.title}</strong>
              <small>{window.app}</small>
            </span>
            <span className="region">{window.region}</span>
          </label>
        ))}
      </fieldset>
      <div className="action-row">
        <button
          disabled={!selectedCount}
          onClick={() => send({ kind: "windows.preview" })}
        >
          배치 미리보기
        </button>
        <button
          className="primary"
          disabled={!state.preview}
          onClick={() => send({ kind: "windows.apply" })}
        >
          가상 창에 적용
        </button>
      </div>
      {state.preview && (
        <div className="preview-card" aria-label="배치 미리보기 결과">
          <strong>적용 예정 · {state.preview.changes.length}개 창</strong>
          {state.preview.changes.map((change) => (
            <div key={change.id}>
              <span>{change.title}</span>
              <b>{change.region}</b>
            </div>
          ))}
          <p>선택을 바꾸면 미리보기가 무효화됩니다.</p>
        </div>
      )}
      {state.applied && (
        <p className="success" role="status">
          {state.applied}
        </p>
      )}
      <p className="muted">
        현재 Windows 창은 이동하지 않습니다. 실제 창 제어는 Rust 서비스 연결 후
        제공됩니다.
      </p>
    </section>
  );
}
function DiagnosticsPanel(props: IDockviewPanelProps) {
  const state = useWorkspace();
  const [errorsOnly, setErrorsOnly] = useState(false);
  const traces = state.traces
    .filter((trace) => !errorsOnly || trace.status === "error")
    .slice()
    .reverse();
  return (
    <section className="panel-body diagnostics" aria-label="진단 패널">
      <PanelTools {...props} />
      <div className="section-heading">
        <div>
          <h1>명령과 실행 기록</h1>
          <p>모든 패널이 같은 요청·상태 기록을 공유합니다.</p>
        </div>
        <label className="check-label">
          <input
            type="checkbox"
            checked={errorsOnly}
            onChange={(event) => setErrorsOnly(event.target.checked)}
          />
          오류만 보기
        </label>
      </div>
      {!traces.length ? (
        <div className="empty-state">
          <strong>실행 기록이 없습니다.</strong>
          <p>
            {state.mode === "fixture" ? "테스트 작업" : "작업"}을 시작하면 요청
            ID, 단계, 오류 원인이 여기에 표시됩니다.
          </p>
        </div>
      ) : (
        <div className="trace-list" role="log" aria-label="명령 기록">
          {traces.map((trace, index) => (
            <div
              className={`trace ${trace.status}`}
              key={`${trace.id}-${index}`}
            >
              <time>
                {new Date(trace.time).toLocaleTimeString("ko-KR", {
                  hour12: false,
                })}
              </time>
              <span className="trace-status">{trace.status}</span>
              <div>
                <strong>{trace.operation}</strong>
                <p>{trace.message}</p>
              </div>
              <code>{trace.id}</code>
            </div>
          ))}
        </div>
      )}
      {state.native && (
        <div className="native-job-records">
          <small>설정 폴더: {state.native.configRoot}</small>
          <h2>오디오</h2>
          <NativeJobLog job={state.native.jobs.audio} />
          <h2>케이블</h2>
          <NativeJobLog job={state.native.jobs.cable} />
          {state.native.jobs.mirrors.map((job, index) => (
            <div key={index}>
              <h2>미러링 {index + 1}</h2>
              <NativeJobLog job={job} />
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
function NativePanel({
  type,
  dockProps,
}: {
  type: "audio" | "cable" | "windows";
  dockProps: IDockviewPanelProps;
}) {
  const state = useWorkspace();
  const invoke = (command: NativeCommand) =>
    backend.dispatch({ kind: "native.command", command });
  const label = `${titles[type]} 패널`;
  return (
    <section className="panel-body" aria-label={label}>
      <PanelTools {...dockProps} />
      {state.native ? (
        <fieldset
          className="native-content"
          disabled={state.connection !== "connected" || !!state.pendingCommands}
        >
          {type === "audio" ? (
            <NativeAudio data={state.native} invoke={invoke} />
          ) : type === "cable" ? (
            <NativeCablePanel data={state.native} invoke={invoke} />
          ) : (
            <NativeWindows data={state.native} invoke={invoke} />
          )}
        </fieldset>
      ) : (
        <div className="empty-state">
          <h1>{titles[type]}</h1>
          <p>실사용 장치 서비스에 연결하고 있습니다.</p>
          <p>
            연결이 끊긴 경우 Desktop Workspace 실행 스크립트로 앱을 시작하세요.
          </p>
        </div>
      )}
    </section>
  );
}
const components = {
  audio: AudioPanel,
  cable: CablePanel,
  windows: WindowsPanel,
  diagnostics: DiagnosticsPanel,
};
function initialLayout(api: DockviewApi) {
  api.clear();
  api.addPanel({ id: "audio", component: "audio", title: titles.audio });
  api.addPanel({
    id: "cable",
    component: "cable",
    title: titles.cable,
    position: { referencePanel: "audio", direction: "within" },
  });
  api.addPanel({
    id: "windows",
    component: "windows",
    title: titles.windows,
    position: { referencePanel: "audio", direction: "right" },
  });
  api.addPanel({
    id: "diagnostics",
    component: "diagnostics",
    title: titles.diagnostics,
    initialHeight: 240,
    position: { referencePanel: "windows", direction: "below" },
  });
  api.getPanel("audio")?.api.setActive();
}
function validLayout(data: unknown): data is SerializedDockview {
  if (!data || typeof data !== "object") return false;
  const candidate = data as SerializedDockview;
  return (
    !!candidate.grid &&
    !!candidate.panels &&
    Object.entries(candidate.panels).every(
      ([id, panel]) =>
        Object.hasOwn(titles, id) && panel.contentComponent === id,
    ) &&
    !candidate.popoutGroups?.length
  );
}
function Workspace() {
  const state = useWorkspace();
  const apiRef = useRef<DockviewApi | null>(null);
  const subscriptions = useRef<{ dispose(): void }[]>([]);
  const [active, setActive] = useState("audio");
  const [notice, setNotice] = useState("배치가 이 브라우저에 자동 저장됩니다.");
  useEffect(
    () => () => {
      subscriptions.current.forEach((subscription) => subscription.dispose());
    },
    [],
  );
  const ready = (event: DockviewReadyEvent) => {
    const api = event.api;
    apiRef.current = api;
    let restored = false;
    try {
      const stored = localStorage.getItem(layoutKey);
      if (stored) {
        const data: unknown = JSON.parse(stored);
        if (!validLayout(data)) throw new Error("Invalid layout");
        api.fromJSON(data);
        restored = true;
      }
    } catch {
      setNotice("저장된 배치를 읽지 못해 기본 배치로 열었습니다.");
    }
    if (!restored) initialLayout(api);
    subscriptions.current = [
      api.onDidActivePanelChange((event) => setActive(event.panel?.id ?? "")),
      api.onDidLayoutChange(() => {
        // Persist the last complete grid; reopening a page must not spawn windows
        // without a gesture or leave a detached panel missing from the workspace.
        if (api.getPopouts().length) return;
        try {
          localStorage.setItem(layoutKey, JSON.stringify(api.toJSON()));
        } catch {
          setNotice(
            "배치를 저장할 수 없습니다. 브라우저 저장 공간을 확인하세요.",
          );
        }
      }),
      api.onDidOpenPopoutWindowFail(() =>
        setNotice("새 창이 차단되었습니다. 이 사이트의 팝업을 허용해 주세요."),
      ),
    ];
    setActive(api.activePanel?.id ?? "");
  };
  const openPanel = (id: PanelId) => {
    const api = apiRef.current;
    if (!api) return;
    const panel =
      api.getPanel(id) ??
      api.addPanel({ id, component: id, title: titles[id] });
    panel.api.setActive();
    panel.api.getWindow().focus();
  };
  const split = (position: "right" | "bottom") => {
    const api = apiRef.current;
    const panel = api?.activePanel;
    if (!api || !panel) return;
    const reference = api.groups.find(
      (group) =>
        group !== panel.api.group && group.api.location.type === "grid",
    );
    panel.api.moveTo({ group: reference, position });
  };
  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand">
          <span className="brand-mark">▥</span>
          <div>
            <strong>Desktop Workspace</strong>
            <span>데스크톱 도구 · 통합 작업 공간</span>
          </div>
        </div>
        <span
          className={`fixture-badge ${state.mode === "native" ? "native-badge" : ""}`}
        >
          {state.mode === "fixture" ? "TEST WORKSPACE" : "DESKTOP WORKSPACE"}
        </span>
        <div className="workspace-actions">
          {state.mode === "native" && (
            <>
              <button
                disabled={state.connection !== "connected"}
                onClick={() =>
                  void backend.dispatch({
                    kind: "native.command",
                    command: { kind: "stop_all" },
                  })
                }
              >
                모든 작업 중지
              </button>
              <button
                disabled={state.connection !== "connected"}
                onClick={() =>
                  void backend.dispatch({
                    kind: "native.command",
                    command: { kind: "shutdown" },
                  })
                }
              >
                앱 종료
              </button>
            </>
          )}
          <button onClick={() => split("right")}>좌우 분할</button>
          <button onClick={() => split("bottom")}>상하 분할</button>
          <button
            onClick={() => {
              if (apiRef.current) initialLayout(apiRef.current);
            }}
          >
            기본 배치
          </button>
        </div>
      </header>
      <div className="mode-banner">
        <span className="mode-dot" />
        <strong>
          {state.mode === "fixture" ? "테스트 모드" : "실사용 모드"}
        </strong>
        <span>
          {state.mode === "fixture"
            ? "가상 장치와 가상 창을 사용합니다. 실제 오디오·Windows 설정은 변경하지 않습니다."
            : state.connection === "connected"
              ? "실제 장치에 연결됐습니다. 시작·적용 버튼을 눌러야 작업이 실행됩니다. 탭을 닫아도 작업은 유지됩니다."
              : "실사용 서비스에 연결 중이거나 연결이 끊겼습니다. 실제 작업의 시작 버튼은 비활성화됩니다."}
        </span>
        {state.mode === "native" && state.connection === "connected" && (
          <button
            onClick={() =>
              void backend.dispatch({
                kind: "native.command",
                command: { kind: "settings.reload" },
              })
            }
          >
            설정 다시 읽기
          </button>
        )}
      </div>
      {!!state.pendingCommands && (
        <p className="mode-banner" role="status">
          명령 처리 중…
        </p>
      )}
      {state.serviceIssue && (
        <div className="service-error" role="alert">
          <strong>{state.serviceIssue.message}</strong>
          <small>
            {state.serviceIssue.code} · 요청 {state.serviceIssue.requestId}
          </small>
        </div>
      )}
      {!!state.native?.errors.length && (
        <div className="service-error" role="alert">
          {state.native.errors.join(" · ")}
        </div>
      )}
      <div className="workspace-body">
        <nav className="activity-bar" aria-label="제어 패널">
          {(Object.keys(titles) as PanelId[]).map((id, index) => (
            <button
              className={active === id ? "active" : ""}
              key={id}
              aria-label={`${titles[id]} 열기`}
              aria-pressed={active === id}
              title={titles[id]}
              onClick={() => openPanel(id)}
            >
              <span>{["DC", "AC", "WM", "LOG"][index]}</span>
              <small>{["출력", "케이블", "창 배치", "진단"][index]}</small>
            </button>
          ))}
        </nav>
        <main className="dock-area">
          <DockviewReact
            components={components}
            onReady={ready}
            theme={themeDark}
          />
        </main>
      </div>
      <footer className="status-bar">
        <span>
          {state.mode === "fixture"
            ? "● Fixture backend"
            : state.connection === "connected"
              ? "● 장치 서비스 연결됨"
              : "● 서비스 연결 안 됨"}
        </span>
        <span>
          오디오{" "}
          {state.mode === "native" && state.audio.phase === "running"
            ? "실행 중"
            : phaseLabel[state.audio.phase]}{" "}
          · 케이블{" "}
          {state.mode === "native" && state.cable.phase === "running"
            ? "실행 중"
            : phaseLabel[state.cable.phase]}
        </span>
        <span className="layout-notice" role="status">
          {notice}
        </span>
      </footer>
    </div>
  );
}
createRoot(document.getElementById("root")!).render(<Workspace />);
