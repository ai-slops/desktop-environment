// UI-facing contract. A future Rust bridge implements this interface; panels
// must never call native APIs or depend on a particular desktop shell.
export type Device = {
  id: string;
  name: string;
  detail: string;
  exclusive?: boolean;
};
export type Issue = {
  code: string;
  operation: string;
  message: string;
  remedy: string;
  requestId: string;
};
export type Trace = {
  id: string;
  time: string;
  operation: string;
  status: "ok" | "pending" | "error" | "cancelled";
  message: string;
};
export type Phase = "stopped" | "starting" | "running" | "failed";
export type WindowItem = {
  id: string;
  title: string;
  app: string;
  selected: boolean;
  region: string;
};
export type Preview = {
  revision: number;
  view: string;
  changes: { id: string; title: string; region: string }[];
};
export type WorkspaceState = {
  mode: "fixture";
  revision: number;
  devices: Device[];
  audio: {
    source: string;
    target: string;
    phase: Phase;
    scenario: "normal" | "exclusive" | "disconnected";
    error?: Issue;
  };
  cable: { output: string; volume: number; muted: boolean; phase: Phase };
  windows: WindowItem[];
  view: "coding" | "monitoring";
  windowRevision: number;
  preview?: Preview;
  applied?: string;
  traces: Trace[];
};
export type Command =
  | { kind: "audio.select"; role: "source" | "target"; id: string }
  | { kind: "audio.scenario"; scenario: WorkspaceState["audio"]["scenario"] }
  | { kind: "audio.start" }
  | { kind: "audio.stop" }
  | { kind: "cable.output"; id: string }
  | { kind: "cable.volume"; value: number }
  | { kind: "cable.mute"; muted: boolean }
  | { kind: "cable.start" }
  | { kind: "cable.stop" }
  | { kind: "windows.select"; id: string; selected: boolean }
  | { kind: "windows.view"; view: WorkspaceState["view"] }
  | { kind: "windows.preview" }
  | { kind: "windows.apply" };
export type CommandResult = { requestId: string; ok: boolean; issue?: Issue };
export interface WorkspaceBackend {
  getSnapshot(): WorkspaceState;
  subscribe(listener: () => void): () => void;
  dispatch(command: Command): Promise<CommandResult>;
}
export const devices: Device[] = [
  {
    id: "default",
    name: "Windows 기본 출력",
    detail: "테스트 케이블 · 2ch / 48kHz",
  },
  {
    id: "headphones",
    name: "헤드폰",
    detail: "High Definition Audio · 2ch / 48kHz",
  },
  { id: "capture-card", name: "캡처 카드", detail: "HDMI Audio · 2ch / 48kHz" },
  {
    id: "cable",
    name: "가상 케이블 입력",
    detail: "Fixture playback endpoint · 2ch / 48kHz",
  },
];
export const resolvedDeviceId = (id: string) =>
  id === "default" ? "cable" : id;
export const sameOutput = (source: string, target: string) =>
  resolvedDeviceId(source) === resolvedDeviceId(target);
export function createFixtureBackend(delayMs = 140): WorkspaceBackend {
  let state: WorkspaceState = {
    mode: "fixture",
    revision: 0,
    devices,
    audio: {
      source: "default",
      target: "capture-card",
      phase: "stopped",
      scenario: "normal",
    },
    cable: { output: "headphones", volume: 35, muted: false, phase: "stopped" },
    windows: [
      {
        id: "browser",
        title: "프로젝트 자료",
        app: "Chrome",
        selected: true,
        region: "현재 위치",
      },
      {
        id: "editor",
        title: "desktop-environment",
        app: "VS Code",
        selected: true,
        region: "현재 위치",
      },
      {
        id: "terminal",
        title: "개발 서버",
        app: "Terminal",
        selected: false,
        region: "현재 위치",
      },
    ],
    view: "coding",
    windowRevision: 0,
    traces: [],
  };
  const listeners = new Set<() => void>();
  let requestSequence = 0;
  let audioEpoch = 0;
  const update = (patch: Partial<WorkspaceState>) => {
    state = { ...state, ...patch, revision: state.revision + 1 };
    for (const listener of listeners) listener();
  };
  const trace = (
    id: string,
    operation: string,
    status: Trace["status"],
    message: string,
  ) => {
    update({
      traces: [
        ...state.traces,
        { id, operation, status, message, time: new Date().toISOString() },
      ].slice(-100),
    });
  };
  const result = (id: string, ok = true, issue?: Issue): CommandResult => ({
    requestId: id,
    ok,
    issue,
  });
  const fail = (
    id: string,
    operation: string,
    code: string,
    message: string,
    remedy: string,
  ) => {
    const issue: Issue = { requestId: id, operation, code, message, remedy };
    trace(id, operation, "error", `${code} · ${message}`);
    return issue;
  };
  return {
    getSnapshot: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    async dispatch(command) {
      const id = `fixture-${++requestSequence}`;
      switch (command.kind) {
        case "audio.select": {
          if (!state.devices.some((device) => device.id === command.id)) {
            return result(
              id,
              false,
              fail(
                id,
                command.kind,
                "DEVICE_NOT_FOUND",
                "장치를 찾을 수 없습니다.",
                "장치 목록을 새로 조회하세요.",
              ),
            );
          }
          ++audioEpoch;
          update({
            audio: {
              ...state.audio,
              [command.role]: command.id,
              phase: "stopped",
              error: undefined,
            },
          });
          break;
        }
        case "audio.scenario":
          ++audioEpoch;
          update({
            audio: {
              ...state.audio,
              scenario: command.scenario,
              phase: "stopped",
              error: undefined,
            },
          });
          break;
        case "audio.start": {
          if (
            state.audio.phase === "running" ||
            state.audio.phase === "starting"
          ) {
            return result(
              id,
              false,
              fail(
                id,
                command.kind,
                "ALREADY_RUNNING",
                "이미 실행 중인 작업입니다.",
                "현재 작업을 중지한 뒤 다시 시작하세요.",
              ),
            );
          }
          const epoch = ++audioEpoch;
          update({
            audio: { ...state.audio, phase: "starting", error: undefined },
          });
          trace(id, command.kind, "pending", "가상 장치 초기화");
          await new Promise((resolve) => setTimeout(resolve, delayMs));
          if (epoch !== audioEpoch) {
            trace(id, command.kind, "cancelled", "설정 변경 또는 중지로 취소");
            return result(id, false);
          }
          let issue: Issue | undefined;
          if (sameOutput(state.audio.source, state.audio.target)) {
            issue = fail(
              id,
              command.kind,
              "FEEDBACK_PREVENTED",
              "원본과 대상이 같은 장치입니다.",
              "서로 다른 출력 장치를 선택하세요.",
            );
          } else if (state.audio.scenario === "exclusive") {
            issue = fail(
              id,
              command.kind,
              "AUDCLNT_E_DEVICE_IN_USE",
              "원본 장치를 독점 사용 중입니다. (0x8889000A)",
              "원본 장치를 사용하는 앱을 공유 모드로 전환하거나 장치 사용을 중지하세요.",
            );
          } else if (state.audio.scenario === "disconnected") {
            issue = fail(
              id,
              command.kind,
              "DEVICE_DISCONNECTED",
              "선택한 대상의 연결이 끊겼습니다.",
              "연결을 확인하고 장치를 다시 선택하세요.",
            );
          }
          update({
            audio: {
              ...state.audio,
              phase: issue ? "failed" : "running",
              error: issue,
            },
          });
          if (!issue)
            trace(
              id,
              command.kind,
              "ok",
              "테스트 라우팅 실행 · 실제 재생 없음",
            );
          return result(id, !issue, issue);
        }
        case "audio.stop":
          ++audioEpoch;
          update({
            audio: { ...state.audio, phase: "stopped", error: undefined },
          });
          break;
        case "cable.output":
          if (!state.devices.some((device) => device.id === command.id))
            return result(id, false);
          update({
            cable: { ...state.cable, output: command.id, phase: "stopped" },
          });
          break;
        case "cable.volume":
          if (
            !Number.isFinite(command.value) ||
            command.value < 0 ||
            command.value > 100
          )
            return result(id, false);
          update({
            cable: { ...state.cable, volume: Math.round(command.value) },
          });
          break;
        case "cable.mute":
          update({ cable: { ...state.cable, muted: command.muted } });
          break;
        case "cable.start":
          if (resolvedDeviceId(state.cable.output) === "cable")
            return result(
              id,
              false,
              fail(
                id,
                command.kind,
                "FEEDBACK_PREVENTED",
                "케이블을 같은 케이블로 모니터링할 수 없습니다.",
                "다른 출력을 선택하세요.",
              ),
            );
          update({ cable: { ...state.cable, phase: "running" } });
          break;
        case "cable.stop":
          update({ cable: { ...state.cable, phase: "stopped" } });
          break;
        case "windows.select":
          update({
            windows: state.windows.map((window) =>
              window.id === command.id
                ? { ...window, selected: command.selected }
                : window,
            ),
            preview: undefined,
            applied: undefined,
            windowRevision: state.windowRevision + 1,
          });
          break;
        case "windows.view":
          update({
            view: command.view,
            preview: undefined,
            applied: undefined,
            windowRevision: state.windowRevision + 1,
          });
          break;
        case "windows.preview": {
          const selected = state.windows.filter((window) => window.selected);
          if (!selected.length)
            return result(
              id,
              false,
              fail(
                id,
                command.kind,
                "EMPTY_SELECTION",
                "배치할 창을 선택하세요.",
                "창 목록에서 체크박스를 선택하세요.",
              ),
            );
          const regions =
            state.view === "coding"
              ? ["왼쪽 50%", "오른쪽 50%", "하단 30%"]
              : ["메인 70%", "오른쪽 30%", "하단 30%"];
          update({
            preview: {
              revision: state.windowRevision,
              view: state.view,
              changes: selected.map((window, index) => ({
                id: window.id,
                title: window.title,
                region: regions[index % regions.length],
              })),
            },
            applied: undefined,
          });
          break;
        }
        case "windows.apply": {
          if (!state.preview || state.preview.revision !== state.windowRevision)
            return result(
              id,
              false,
              fail(
                id,
                command.kind,
                "STALE_PREVIEW",
                "현재 선택에 대한 미리보기가 없습니다.",
                "배치 미리보기를 다시 생성하세요.",
              ),
            );
          const preview = state.preview;
          update({
            windows: state.windows.map((window) => ({
              ...window,
              region:
                preview.changes.find((change) => change.id === window.id)
                  ?.region ?? window.region,
            })),
            applied: `${preview.changes.length}개 가상 창에 배치를 적용했습니다.`,
            preview: undefined,
            windowRevision: state.windowRevision + 1,
          });
          break;
        }
      }
      trace(
        id,
        command.kind,
        "ok",
        command.kind.startsWith("windows")
          ? "가상 창 설정 변경"
          : "테스트 설정 변경",
      );
      return result(id);
    },
  };
}
export const backend = createFixtureBackend();
