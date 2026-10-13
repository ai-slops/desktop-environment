import type {
  Command,
  CommandResult,
  Issue,
  Phase,
  WorkspaceBackend,
  WorkspaceState,
} from "./workspace.ts";
import type { NativeSnapshot } from "./native-protocol.ts";
export function createLiveBackend(): WorkspaceBackend {
  let state: WorkspaceState = {
    mode: "native",
    connection: "connecting",
    revision: 0,
    devices: [],
    audio: { source: "", target: "", phase: "stopped", scenario: "normal" },
    cable: { output: "", volume: 0, muted: false, phase: "stopped" },
    windows: [],
    view: "coding",
    windowRevision: 0,
    traces: [],
  };
  const listeners = new Set<() => void>();
  let token: string | undefined;
  let pendingPoll = false;
  let nativeRevision = -1;
  let stopped = false;
  let issue: Issue | undefined;
  const update = (patch: Partial<WorkspaceState>) => {
    state = { ...state, ...patch, revision: state.revision + 1 };
    for (const listener of listeners) listener();
  };
  const apply = (native: NativeSnapshot) => {
    if (
      !Array.isArray(native.endpoints) ||
      !native.jobs ||
      !native.settings ||
      !Number.isFinite(native.revision)
    )
      throw new Error(
        "호환되지 않는 Rust 서비스 응답입니다. 앱을 다시 빌드하세요.",
      );
    if (native.revision < nativeRevision) return;
    nativeRevision = native.revision;
    const config = native.settings.last_used;
    const outputs = native.endpoints.filter(
      (endpoint) => endpoint.flow === "playback",
    );
    const phase = (value: string): Phase =>
      ["running", "starting", "failed", "recovering"].includes(value)
        ? (value as Phase)
        : "stopped";
    const workerIssue = native.jobs.audio.error
      ? {
          requestId: "audio-worker",
          operation: "audio.start",
          code: native.jobs.audio.error.includes("0x8889000A")
            ? "AUDCLNT_E_DEVICE_IN_USE"
            : "AUDIO_WORKER_FAILED",
          message: native.jobs.audio.error,
          remedy: "원본·대상 장치와 실행 로그를 확인하고 다시 시작하세요.",
        }
      : undefined;
    update({
      native,
      connection: "connected",
      serviceIssue: issue,
      devices: [
        {
          id: "default",
          name: "Windows 기본 출력",
          detail:
            outputs.find((endpoint) => endpoint.isDefault)?.name ??
            "기본 출력 없음",
        },
        ...outputs.map((endpoint) => ({
          id: endpoint.id,
          name: endpoint.name,
          detail: endpoint.isDefault ? "현재 기본 출력" : "연결됨",
        })),
      ],
      audio: {
        ...state.audio,
        source: config.audio.source,
        target: config.audio.target,
        phase: phase(native.jobs.audio.phase),
        error: workerIssue,
      },
      cable: {
        output: native.cableSettings.output_id,
        volume: native.cableSettings.muted
          ? 0
          : Math.round(100 * 10 ** (native.cableSettings.gain_db / 20)),
        muted: native.cableSettings.muted,
        phase: phase(native.jobs.cable.phase),
      },
      traces: native.traces.map((trace) => ({
        ...trace,
        time: new Date(trace.timestamp).toISOString(),
      })),
    });
  };
  const request = async (command: {
    kind: string;
    [key: string]: unknown;
  }): Promise<CommandResult> => {
    const id = crypto.randomUUID();
    const interactive = command.kind !== "snapshot";
    if (interactive)
      update({ pendingCommands: (state.pendingCommands ?? 0) + 1 });
    try {
      if (!token) {
        const response = await fetch("/api/bootstrap", { cache: "no-store" });
        if (!response.ok)
          throw new Error(
            "실사용 서비스가 연결되지 않았습니다. Desktop Workspace 실행 스크립트로 앱을 시작하세요.",
          );
        const bootstrap = await response.json();
        if (bootstrap.version !== 1 || typeof bootstrap.token !== "string")
          throw new Error("지원하지 않는 서비스 버전입니다.");
        token = bootstrap.token;
        nativeRevision = -1;
        if (issue?.code === "SERVICE_DISCONNECTED") issue = undefined;
      }
      const response = await fetch("/api/command", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Workspace-Token": token!,
        },
        body: JSON.stringify({ id, command }),
      });
      const envelope = await response.json();
      if (!response.ok)
        throw new Error(
          envelope.error?.message ?? `서비스 HTTP 오류 ${response.status}`,
        );
      if (envelope.id !== id)
        throw new Error("서비스 응답 요청 ID가 일치하지 않습니다.");
      if (command.kind !== "snapshot")
        issue = envelope.ok
          ? undefined
          : {
              requestId: id,
              operation: command.kind,
              code: envelope.error?.code ?? "COMMAND_FAILED",
              message: envelope.error?.message ?? "명령이 실패했습니다.",
              remedy: "표시된 원인과 실행 기록을 확인하세요.",
            };
      if (envelope.data?.state) apply(envelope.data.state);
      if (command.kind === "shutdown" && envelope.ok) {
        stopped = true;
        update({ connection: "disconnected", serviceIssue: undefined });
      }
      return {
        requestId: id,
        ok: !!envelope.ok,
        issue,
        result: envelope.data?.result,
      };
    } catch (error) {
      token = undefined;
      issue = {
        requestId: id,
        operation: command.kind,
        code: "SERVICE_DISCONNECTED",
        message: error instanceof Error ? error.message : String(error),
        remedy: "앱 서비스를 다시 시작하세요.",
      };
      update({ connection: "disconnected", serviceIssue: issue });
      return { requestId: id, ok: false, issue };
    } finally {
      if (interactive)
        update({
          pendingCommands: Math.max(0, (state.pendingCommands ?? 1) - 1),
        });
    }
  };
  const poll = async () => {
    if (pendingPoll || stopped) return;
    pendingPoll = true;
    await request({ kind: "snapshot" });
    pendingPoll = false;
  };
  void poll();
  const interval = setInterval(() => {
    void poll();
  }, 750);
  window.addEventListener("pagehide", () => clearInterval(interval), {
    once: true,
  });
  return {
    getSnapshot: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    dispatch(command: Command) {
      return command.kind === "native.command"
        ? request(command.command)
        : Promise.resolve({ requestId: "unsupported", ok: false });
    },
  };
}
