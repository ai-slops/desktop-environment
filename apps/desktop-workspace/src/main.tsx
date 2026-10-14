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
import { VolumeControl } from "./volume.tsx";
import { installTearOff } from "./shell.ts";
import {
  NativeAudio,
  NativeDisplay,
  NativeCablePanel,
  NativeWindows,
  NativeJobLog,
  NativeLaunch,
  PaneStateScope,
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
  audio: "Audio Repeater",
  display: "Display Relay",
  cable: "Audio Cable",
  windows: "Window Manager",
  diagnostics: "진단 · 실행 기록",
};
type PanelId = keyof typeof titles;
const layoutKey = "desktop-workspace.tabs.v3";
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
        <VolumeControl
          value={cable.volume}
          commit={(value) => send({ kind: "cable.volume", value })}
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
  scope,
}: {
  type: "audio" | "display" | "cable" | "windows";
  dockProps: IDockviewPanelProps;
  scope?: string;
}) {
  const state = useWorkspace();
  const invoke = (command: NativeCommand) =>
    backend.dispatch({ kind: "native.command", command });
  const label = `${titles[type]} 패널`;
  return (
    <PaneStateScope.Provider value={scope ?? dockProps.api.id}>
      <section className="panel-body" aria-label={label}>
        <PanelTools {...dockProps} />
        {state.native ? (
          <fieldset
            className="native-content"
            disabled={
              state.connection !== "connected" || !!state.pendingCommands
            }
          >
            {type === "audio" ? (
              <NativeAudio data={state.native} invoke={invoke} />
            ) : type === "display" ? (
              <NativeDisplay data={state.native} invoke={invoke} />
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
              연결이 끊긴 경우 Desktop Workspace 실행 스크립트로 앱을
              시작하세요.
            </p>
          </div>
        )}
      </section>
    </PaneStateScope.Provider>
  );
}

function DisplayPanel(props: IDockviewPanelProps) {
  return <NativePanel type="display" dockProps={props} />;
}
const components = {
  audio: AudioPanel,
  display: DisplayPanel,
  cable: CablePanel,
  windows: WindowsPanel,
  diagnostics: DiagnosticsPanel,
};

type Widget = { id: string; type: PanelId; view: "full" | "launch" };
function seedWidgets(seed?: PanelId): Widget[] {
  return seed ? [{ id: crypto.randomUUID(), type: seed, view: "full" }] : [];
}
function widgetsFrom(params: {
  widgets?: Widget[];
  seed?: PanelId;
  layout?: SerializedDockview;
}): Widget[] {
  if (params.widgets) return params.widgets;
  if (params.layout)
    return Object.values(params.layout.panels)
      .filter(
        (panel) =>
          !!panel.contentComponent &&
          Object.hasOwn(titles, panel.contentComponent),
      )
      .map((panel) => ({
        id: crypto.randomUUID(),
        type: panel.contentComponent as PanelId,
        view: "full",
      }));
  return seedWidgets(params.seed);
}
function EditableTab(
  props: IDockviewPanelProps<{
    widgets?: Widget[];
    seed?: PanelId;
    layout?: SerializedDockview;
  }>,
) {
  const state = useWorkspace();
  const [widgets, setWidgets] = useState<Widget[]>(() =>
    widgetsFrom(props.params),
  );
  const [kind, setKind] = useState<PanelId>(props.params.seed ?? "audio");
  const [view, setView] = useState<"full" | "launch">("launch");
  const [tabName, setTabName] = useState(props.api.title ?? "새 탭");
  const update = (next: Widget[]) => {
    setWidgets(next);
    props.api.updateParameters({ widgets: next });
  };
  useEffect(() => {
    props.api.updateParameters({ widgets });
  }, []);
  const invoke = (command: NativeCommand) =>
    backend.dispatch({ kind: "native.command", command });
  return (
    <div className="workspace-tab">
      <div className="pane-toolbar">
        <input
          aria-label="탭 이름"
          className="tab-name-input"
          value={tabName}
          onChange={(event) => setTabName(event.target.value)}
          onBlur={() => {
            const name = tabName.trim() || props.api.title || "새 탭";
            setTabName(name);
            props.api.setTitle(name);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
        />
        <select
          aria-label="추가할 구성요소"
          value={kind}
          onChange={(event) => setKind(event.target.value as PanelId)}
        >
          {Object.entries(titles).map(([id, title]) => (
            <option key={id} value={id}>
              {title}
            </option>
          ))}
        </select>
        <select
          aria-label="추가할 내용"
          value={view}
          onChange={(event) => setView(event.target.value as "full" | "launch")}
        >
          <option value="launch">프리셋 · 시작</option>
          <option value="full">전체 설정</option>
        </select>
        <button
          disabled={widgets.length >= 32}
          onClick={() =>
            update([...widgets, { id: crypto.randomUUID(), type: kind, view }])
          }
        >
          내용 추가
        </button>
      </div>
      <div className="tab-content">
        {widgets.map((widget, index) => {
          const Component = components[widget.type];
          return (
            <article className="tab-widget" key={widget.id}>
              <header>
                <strong>{titles[widget.type]}</strong>
                <div>
                  <button
                    aria-label={titles[widget.type] + " 위로"}
                    disabled={index === 0}
                    onClick={() => {
                      const next = [...widgets];
                      [next[index - 1], next[index]] = [
                        next[index],
                        next[index - 1],
                      ];
                      update(next);
                    }}
                  >
                    ↑
                  </button>
                  <button
                    aria-label={titles[widget.type] + " 아래로"}
                    disabled={index === widgets.length - 1}
                    onClick={() => {
                      const next = [...widgets];
                      [next[index + 1], next[index]] = [
                        next[index],
                        next[index + 1],
                      ];
                      update(next);
                    }}
                  >
                    ↓
                  </button>
                  <button
                    aria-label={titles[widget.type] + " 내용 제거"}
                    onClick={() =>
                      update(widgets.filter((item) => item.id !== widget.id))
                    }
                  >
                    ×
                  </button>
                </div>
              </header>
              {widget.type === "diagnostics" ? (
                <DiagnosticsPanel {...props} />
              ) : state.mode === "native" && state.native ? (
                widget.view === "launch" ? (
                  <fieldset
                    className="native-content launch-content"
                    aria-label={titles[widget.type] + " 프리셋 · 시작"}
                    disabled={
                      state.connection !== "connected" ||
                      !!state.pendingCommands
                    }
                  >
                    <NativeLaunch
                      data={state.native}
                      invoke={invoke}
                      type={widget.type}
                    />
                  </fieldset>
                ) : (
                  <NativePanel
                    type={widget.type}
                    dockProps={props}
                    scope={props.api.id + "/" + widget.id}
                  />
                )
              ) : (
                <Component {...props} />
              )}
            </article>
          );
        })}
      </div>
    </div>
  );
}
const tabComponents = { tab: EditableTab };
function initialLayout(api: DockviewApi) {
  api.clear();
  for (const seed of ["audio", "display", "cable", "windows"] as PanelId[]) {
    api.addPanel({
      id: seed,
      component: "tab",
      title: titles[seed],
      params: { seed },
      ...(api.activePanel
        ? {
            position: {
              referencePanel: api.activePanel.id,
              direction: "within" as const,
            },
          }
        : {}),
    });
  }
  api.getPanel("audio")?.api.setActive();
}
function validLayout(data: unknown): data is SerializedDockview {
  if (!data || typeof data !== "object") return false;
  const candidate = data as SerializedDockview;
  return (
    !!candidate.grid &&
    !!candidate.panels &&
    !candidate.popoutGroups?.length &&
    Object.values(candidate.panels).every(
      (panel) =>
        panel.contentComponent === "tab" &&
        (!panel.params?.widgets ||
          (Array.isArray(panel.params.widgets) &&
            panel.params.widgets.length <= 32 &&
            panel.params.widgets.every(
              (widget: Widget) =>
                Object.hasOwn(titles, widget.type) &&
                ["full", "launch"].includes(widget.view),
            ))),
    )
  );
}
function Workspace() {
  const state = useWorkspace();
  const api = useRef<DockviewApi | null>(null);
  const subscriptions = useRef<{ dispose(): void }[]>([]);
  const [issue, setIssue] = useState("");
  useEffect(() => () => subscriptions.current.forEach((s) => s.dispose()), []);
  const ready = (event: DockviewReadyEvent) => {
    const dock = event.api;
    api.current = dock;
    let restored = false;
    try {
      const raw =
        localStorage.getItem(layoutKey) ??
        localStorage.getItem("desktop-workspace.tabs.v2") ??
        localStorage.getItem("desktop-workspace.layout.v1");
      if (raw) {
        const layout = JSON.parse(raw);
        for (const panel of Object.values(layout.panels ?? {}) as {
          contentComponent: string;
          params?: {
            widgets?: Widget[];
            seed?: PanelId;
            layout?: SerializedDockview;
          };
        }[]) {
          const old = panel.contentComponent;
          if (
            old === "workspace" ||
            old === "tab" ||
            Object.hasOwn(titles, old)
          ) {
            panel.contentComponent = "tab";
            panel.params = {
              widgets: widgetsFrom({
                ...panel.params,
                seed: Object.hasOwn(titles, old)
                  ? (old as PanelId)
                  : panel.params?.seed,
              }),
            };
          }
        }
        if (validLayout(layout)) {
          dock.fromJSON(layout);
          restored = true;
        }
      }
    } catch {}
    if (!restored) initialLayout(dock);
    const requested = new URLSearchParams(location.search).get("panel");
    if (requested) dock.getPanel(requested)?.api.setActive();
    subscriptions.current = [
      installTearOff(dock),
      dock.onDidLayoutChange(() => {
        if (dock.getPopouts().length) return;
        try {
          localStorage.setItem(layoutKey, JSON.stringify(dock.toJSON()));
        } catch {
          setIssue("탭 배치를 저장할 수 없습니다.");
        }
      }),
      dock.onDidOpenPopoutWindowFail(() => setIssue("새 창을 열 수 없습니다.")),
    ];
  };
  return (
    <div className="app-shell compact-shell">
      <div className="compact-toolbar">
        <button
          onClick={() => {
            const dock = api.current;
            if (!dock) return;
            dock.addPanel({
              id: crypto.randomUUID(),
              component: "tab",
              title: "새 탭",
              params: {},
              ...(dock.activePanel
                ? {
                    position: {
                      referencePanel: dock.activePanel.id,
                      direction: "within",
                    },
                  }
                : {}),
            });
          }}
        >
          탭 추가
        </button>
        <details>
          <summary>설정</summary>
          <div className="compact-menu">
            <button
              onClick={() => {
                if (api.current) initialLayout(api.current);
              }}
            >
              기본 탭 복원
            </button>
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
            <button
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
              onClick={() =>
                void backend.dispatch({
                  kind: "native.command",
                  command: { kind: "shutdown" },
                })
              }
            >
              서비스 중지
            </button>
            {window.desktopShell && (
              <button onClick={() => window.desktopShell?.close()}>
                창 닫기
              </button>
            )}
          </div>
        </details>
      </div>
      {issue && (
        <div role="alert" className="service-error">
          {issue}
        </div>
      )}
      {state.serviceIssue && (
        <div role="alert" className="service-error">
          {state.serviceIssue.message}
        </div>
      )}
      {!!state.native?.errors.length && (
        <div role="alert" className="service-error">
          {state.native.errors.join(" · ")}
        </div>
      )}
      <div className="workspace-body">
        <main className="dock-area">
          <DockviewReact
            components={tabComponents}
            onReady={ready}
            theme={themeDark}
          />
        </main>
      </div>
    </div>
  );
}
createRoot(document.getElementById("root")!).render(<Workspace />);
