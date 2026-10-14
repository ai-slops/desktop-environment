import {
  useEffect,
  useId,
  createContext,
  useContext,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type SetStateAction,
} from "react";
import type { CommandResult } from "./workspace.ts";
import { VolumeControl, readablePercent } from "./volume.tsx";
import type {
  NativeCommand,
  NativeEndpoint,
  NativeJob,
  NativeSnapshot,
  NativeConfiguration,
  NativeNode,
  NativePlan,
  MirrorSettings,
} from "./native-protocol.ts";
export type NativePanelProps = {
  data: NativeSnapshot;
  invoke(command: NativeCommand): Promise<CommandResult>;
};
// Dockview may remount a panel while moving it between windows. Keep unsaved
// selections and edits in the owning page, independently of the panel's DOM.
const panelState = new Map<string, unknown>();
export const PaneStateScope = createContext("");
const panelListeners = new Set<() => void>();
function usePanelState<T>(
  key: string,
  initial: T,
): [T, (value: SetStateAction<T>) => void] {
  key = useContext(PaneStateScope) + ":" + key;
  if (!panelState.has(key)) panelState.set(key, initial);
  const value = useSyncExternalStore(
    (listener) => {
      panelListeners.add(listener);
      return () => {
        panelListeners.delete(listener);
      };
    },
    () => panelState.get(key) as T,
  );
  return [
    value,
    (next) => {
      const previous = panelState.get(key) as T;
      const result =
        typeof next === "function" ? (next as (value: T) => T)(previous) : next;
      if (Object.is(previous, result)) return;
      panelState.set(key, result);
      panelListeners.forEach((listener) => listener());
    },
  ];
}
const active = (job: NativeJob) =>
  ["running", "starting", "recovering"].includes(job.phase);
export function NativeStatus({ job }: { job: NativeJob }) {
  const names: Record<string, string> = {
    stopped: "중지됨",
    starting: "초기화 중",
    running: "실행 중",
    recovering: "장치 복구 중",
    failed: "실패",
  };
  return (
    <span className={`status ${job.phase}`}>
      <i />
      {names[job.phase] ?? job.phase}
    </span>
  );
}
export function NativeJobLog({ job }: { job: NativeJob }) {
  return (
    <>
      {job.error && (
        <div role="alert" className="error-card">
          <strong>작업이 종료됐습니다.</strong>
          <pre>{job.error}</pre>
          {job.error.includes("0x8889000A") && (
            <p>
              장치 사용 충돌 또는 루프백 제한입니다. 독점 모드 설정과 가상
              케이블의 Loopback 지원을 확인하세요.
            </p>
          )}
        </div>
      )}
      <details className="job-log" open={job.phase === "failed"}>
        <summary>실행 로그 · {job.logs.length}줄</summary>
        <pre>{job.logs.join("\n") || "아직 실행 로그가 없습니다."}</pre>
      </details>
    </>
  );
}
function Choices({
  name,
  outputs,
  selected,
  choose,
  defaultAlias = false,
  savedLabel = "",
}: {
  name: string;
  outputs: NativeEndpoint[];
  selected: string;
  choose(id: string): void;
  defaultAlias?: boolean;
  savedLabel?: string;
}) {
  const items = defaultAlias
    ? [
        {
          id: "default",
          name: "Windows 기본 출력",
          flow: "playback" as const,
          isDefault: false,
        },
        ...outputs,
      ]
    : outputs;
  const radioName = useId();
  const missing = selected && !items.some((output) => output.id === selected);
  return (
    <fieldset className="choices">
      <legend>
        {name}
        <small>하나 선택</small>
      </legend>
      {missing && (
        <label className="choice selected disconnected">
          <input type="radio" checked disabled />
          <span className="choice-copy">
            <strong>{savedLabel || selected}</strong>
            <small>연결 안 됨 · 다른 장치를 선택하세요.</small>
          </span>
        </label>
      )}
      {items.map((output) => (
        <label
          className={`choice ${selected === output.id ? "selected" : ""}`}
          key={output.id}
          title={output.id}
        >
          <input
            type="radio"
            name={radioName}
            checked={selected === output.id}
            onChange={() => choose(output.id)}
          />
          <span className="choice-copy">
            <strong>{output.name}</strong>
            <small>
              {output.id === "default"
                ? (outputs.find((output) => output.isDefault)?.name ??
                  "기본 출력 없음")
                : output.isDefault
                  ? "현재 Windows 기본 출력"
                  : "연결됨"}
            </small>
          </span>
          {selected === output.id && (
            <span className="selected-badge">선택됨</span>
          )}
        </label>
      ))}
    </fieldset>
  );
}
function Presets({
  names,
  save,
  load,
  remove,
  overwrite,
}: {
  names: string[];
  save(name: string): void;
  load(name: string): void;
  remove(name: string): void;
  overwrite?(name: string): void;
}) {
  const [name, setName] = useState("");
  const [selected, setSelected] = useState("");
  useEffect(() => {
    if (selected && !names.includes(selected)) setSelected("");
  }, [names, selected]);
  return (
    <fieldset className="preset-controls">
      <legend>프리셋</legend>
      <div className="form-row">
        <input
          aria-label="프리셋 이름"
          placeholder="새 프리셋 이름"
          value={name}
          onChange={(event) => setName(event.target.value)}
          maxLength={80}
        />
        <button disabled={!name.trim()} onClick={() => save(name)}>
          새로 저장
        </button>
      </div>
      <div className="form-row">
        <select
          aria-label="저장된 프리셋"
          value={selected}
          onChange={(event) => {
            setSelected(event.target.value);
            if (event.target.value) load(event.target.value);
          }}
        >
          <option value="">프리셋 선택</option>
          {names.map((name) => (
            <option key={name}>{name}</option>
          ))}
        </select>
        <button
          disabled={!names.includes(selected)}
          onClick={() => load(selected)}
        >
          시작
        </button>
        {overwrite && (
          <button
            disabled={!names.includes(selected)}
            onClick={() => overwrite(selected)}
          >
            덮어쓰기
          </button>
        )}
        <button
          disabled={!names.includes(selected)}
          onClick={() => remove(selected)}
        >
          삭제
        </button>
      </div>
    </fieldset>
  );
}
export function NativeAudio({ data, invoke }: NativePanelProps) {
  const settings = data.settings.last_used;
  const job = data.jobs.audio;
  const outputs = data.endpoints.filter(
    (endpoint) => endpoint.flow === "playback",
  );
  const effective = (id: string) =>
    id === "default"
      ? outputs.find((output) => output.isDefault)?.id
      : outputs.find((output) => output.id === id)?.id;
  const source = effective(settings.audio.source);
  const target = effective(settings.audio.target);
  const invalid = !source || !target || source === target;
  return (
    <>
      <ComponentPresets data={data} invoke={invoke} component="audio" />
      <div className="section-heading">
        <div>
          <h1>오디오 출력 복제</h1>
          <p>원본의 소리를 다른 출력으로 복제합니다.</p>
        </div>
        <NativeStatus job={job} />
      </div>
      <div className="action-row">
        <button onClick={() => void invoke({ kind: "refresh" })}>
          장치 다시 검색
        </button>
        <button
          disabled={!!data.storageError}
          onClick={() => void invoke({ kind: "settings.save" })}
        >
          현재 설정 저장
        </button>
      </div>
      {data.storageError && (
        <p role="alert" className="inline-error">
          설정 파일 저장이 차단됐습니다. {data.storageError}
        </p>
      )}
      <div className="device-columns">
        <Choices
          name="원본 출력"
          outputs={outputs}
          selected={settings.audio.source}
          savedLabel={settings.audio.source_label}
          defaultAlias
          choose={(id) =>
            void invoke({ kind: "audio.select", role: "source", id })
          }
        />
        <Choices
          name="대상 출력"
          outputs={outputs}
          selected={settings.audio.target}
          savedLabel={settings.audio.target_label}
          defaultAlias
          choose={(id) =>
            void invoke({ kind: "audio.select", role: "target", id })
          }
        />
      </div>
      {invalid && (
        <p className="inline-error" role="alert">
          {source === target && source
            ? "원본과 대상이 같은 출력입니다."
            : "연결된 원본과 대상 출력을 선택하세요."}
        </p>
      )}
      <div className="action-row">
        <button
          className="primary"
          disabled={invalid || active(job)}
          onClick={() => void invoke({ kind: "audio.start" })}
        >
          오디오 시작
        </button>
        <button
          disabled={!active(job)}
          onClick={() => void invoke({ kind: "audio.stop" })}
        >
          오디오 중지
        </button>
      </div>
      <p className="muted">
        출력 선택을 바꾸면 이 앱의 오디오 작업이 중지됩니다.
      </p>
      <NativeJobLog job={job} />
    </>
  );
}
function ComponentPresets({
  data,
  invoke,
  component,
}: NativePanelProps & { component: "audio" | "display" }) {
  const prefix = component + "/";
  const presets = data.settings.presets.filter(
    (preset) => !preset.name.includes("/") || preset.name.startsWith(prefix),
  );
  const resolve = (name: string) =>
    presets.find((preset) => preset.name.replace(prefix, "") === name)?.name ??
    prefix + name;
  return (
    <Presets
      names={presets.map((preset) => preset.name.replace(prefix, ""))}
      save={(name) => void invoke({ kind: "preset.save", name: prefix + name })}
      overwrite={(name) =>
        void invoke({
          kind: "preset.save",
          name: resolve(name),
          overwrite: true,
        })
      }
      load={(name) =>
        void invoke({
          kind: "component.preset.run",
          component,
          name: resolve(name),
        })
      }
      remove={(name) =>
        void invoke({ kind: "preset.delete", name: resolve(name) })
      }
    />
  );
}
export function NativeLaunch({
  data,
  invoke,
  type,
}: NativePanelProps & { type: "audio" | "display" | "cable" | "windows" }) {
  const [selected, setSelected] = useState("");
  if (type === "windows") return <WindowLaunch data={data} invoke={invoke} />;
  const prefix = type + "/";
  const presets =
    type === "cable"
      ? data.cableSettings.presets.map((preset) => ({
          id: preset.name,
          name: preset.name,
        }))
      : data.settings.presets
          .filter(
            (preset) =>
              !preset.name.includes("/") || preset.name.startsWith(prefix),
          )
          .map((preset) => ({
            id: preset.name,
            name: preset.name.replace(prefix, ""),
          }));
  const job =
    type === "cable"
      ? data.jobs.cable
      : type === "audio"
        ? data.jobs.audio
        : (data.jobs.mirrors.find(active) ?? data.jobs.mirrors[0]);
  const runPreset = (name: string) =>
    void invoke(
      type === "cable"
        ? { kind: "cable.preset.run", name }
        : { kind: "component.preset.run", component: type, name },
    );
  const start = () => {
    if (selected) return runPreset(selected);
    if (type === "display") {
      data.jobs.mirrors.forEach((mirror, index) => {
        if (
          !active(mirror) &&
          [
            data.settings.last_used.display,
            ...data.settings.last_used.additional_displays,
          ][index].enabled
        )
          void invoke({ kind: "mirror.start", index });
      });
    } else
      void invoke({ kind: type === "audio" ? "audio.start" : "cable.start" });
  };
  return (
    <>
      <NativeStatus job={job} />
      <select
        aria-label="실행할 프리셋"
        value={selected}
        onChange={(event) => {
          setSelected(event.target.value);
          if (event.target.value) runPreset(event.target.value);
        }}
      >
        <option value="">현재 설정</option>
        {presets.map((preset) => (
          <option key={preset.id} value={preset.id}>
            {preset.name}
          </option>
        ))}
      </select>
      <div className="action-row">
        <button onClick={start}>시작</button>
        <button
          disabled={!active(job)}
          onClick={() => {
            if (type === "display")
              data.jobs.mirrors.forEach((mirror, index) => {
                if (active(mirror)) void invoke({ kind: "mirror.stop", index });
              });
            else
              void invoke({
                kind: type === "audio" ? "audio.stop" : "cable.stop",
              });
          }}
        >
          중지
        </button>
      </div>
      {job.error && <p role="alert">{job.error}</p>}
    </>
  );
}
export function NativeBundlePresets({
  data,
  invoke,
  compact = false,
}: NativePanelProps & { compact?: boolean }) {
  const [name, setName] = useState("");
  const [selected, setSelected] = useState("");
  const [included, setIncluded] = useState<string[]>([
    "audio",
    "cable",
    "display",
  ]);
  const [view, setView] = useState("");
  const [slot, setSlot] = useState("");
  const labels: Record<string, string> = {
    audio: "Audio Repeater",
    display: "Display Relay",
    cable: "Audio Cable",
    windows: "Window Manager",
  };
  const presets = data.bundlePresets ?? [];
  if (!data.bundlePresets)
    return (
      <p role="alert">
        통합 프리셋을 사용하려면 설정에서 서비스를 중지한 뒤 앱을 다시
        열어주세요.
      </p>
    );
  const selectedPreset = presets.find((preset) => preset.name === selected);
  const save = (overwrite: boolean) =>
    void invoke({
      kind: "bundle.preset.save",
      name: overwrite ? selected : name,
      components: included,
      window_view: view || null,
      window_slot: slot || null,
      overwrite,
    });
  const apply = (start: boolean) =>
    void invoke({ kind: "bundle.preset.apply", name: selected, start });
  return (
    <>
      {!compact && (
        <>
          <p>
            포함할 기능을 선택하고 현재 설정을 한 번에 저장합니다. 포함하지 않은
            기능의 설정과 작업은 유지됩니다.
          </p>
          <fieldset>
            <legend>포함할 기능</legend>
            {Object.entries(labels).map(([id, label]) => (
              <label className="check-label" key={id}>
                <input
                  type="checkbox"
                  checked={included.includes(id)}
                  onChange={(event) =>
                    setIncluded(
                      event.target.checked
                        ? [...included, id]
                        : included.filter((item) => item !== id),
                    )
                  }
                />
                {label}
              </label>
            ))}
          </fieldset>
          {included.includes("windows") &&
            (!data.wm.connected ? (
              <button onClick={() => void invoke({ kind: "windows.connect" })}>
                창 관리자 연결
              </button>
            ) : (
              <div className="form-row">
                <select
                  aria-label="통합 프리셋 창 배치"
                  value={view}
                  onChange={(event) => setView(event.target.value)}
                >
                  <option value="">창 배치 선택</option>
                  {Object.values(data.wm.configuration?.views ?? {}).map(
                    (item) => (
                      <option key={item.id} value={item.id}>
                        {item.name}
                      </option>
                    ),
                  )}
                </select>
                <select
                  aria-label="통합 프리셋 화면 영역"
                  value={slot}
                  onChange={(event) => setSlot(event.target.value)}
                >
                  <option value="">화면 영역 선택</option>
                  {Object.values(data.wm.configuration?.slots ?? {}).map(
                    (item) => (
                      <option key={item.id} value={item.id}>
                        {item.name}
                      </option>
                    ),
                  )}
                </select>
              </div>
            ))}
          <div className="form-row">
            <input
              aria-label="통합 프리셋 이름"
              value={name}
              maxLength={80}
              placeholder="예: 작업 / 게임 / 조용히 듣기"
              onChange={(event) => setName(event.target.value)}
            />
            <button
              disabled={
                !name.trim() ||
                !included.length ||
                !!data.bundleStorageError ||
                (included.includes("windows") && (!view || !slot))
              }
              onClick={() => save(false)}
            >
              현재 설정 묶어서 저장
            </button>
          </div>
        </>
      )}
      <select
        aria-label="통합 프리셋 선택"
        value={selected}
        onChange={(event) => {
          setSelected(event.target.value);
          const preset = presets.find(
            (item) => item.name === event.target.value,
          );
          if (preset) {
            setIncluded(preset.components);
            setView(preset.window?.view ?? "");
            setSlot(preset.window?.slot ?? "");
          }
          if (event.target.value)
            void invoke({
              kind: "bundle.preset.apply",
              name: event.target.value,
              start: true,
            });
        }}
      >
        <option value="">통합 프리셋 선택</option>
        {presets.map((preset) => (
          <option key={preset.name}>{preset.name}</option>
        ))}
      </select>
      {selectedPreset && (
        <div className="bundle-status">
          {selectedPreset.components.map((id) => {
            const job =
              id === "audio"
                ? data.jobs.audio
                : id === "cable"
                  ? data.jobs.cable
                  : id === "display"
                    ? (data.jobs.mirrors.find(active) ?? data.jobs.mirrors[0])
                    : undefined;
            return (
              <div key={id}>
                <strong>{labels[id] ?? id}</strong>
                {job ? (
                  <NativeStatus job={job} />
                ) : (
                  <span>{data.wm.connected ? "연결됨" : "연결 안 됨"}</span>
                )}
                {job?.error && <NativeJobLog job={job} />}
              </div>
            );
          })}
        </div>
      )}
      <div className="action-row">
        <button disabled={!selectedPreset} onClick={() => apply(true)}>
          함께 시작
        </button>
        {!compact && (
          <>
            <button disabled={!selectedPreset} onClick={() => apply(false)}>
              설정만 적용 · 포함된 출력 중지
            </button>
            <button
              disabled={
                !selectedPreset ||
                !included.length ||
                !!data.bundleStorageError ||
                (included.includes("windows") && (!view || !slot))
              }
              onClick={() => save(true)}
            >
              덮어쓰기
            </button>
            <button
              disabled={!selectedPreset || !!data.bundleStorageError}
              onClick={() =>
                void invoke({ kind: "bundle.preset.delete", name: selected })
              }
            >
              삭제
            </button>
          </>
        )}
      </div>
      {data.bundleStorageError && (
        <p role="alert">
          통합 프리셋 파일 저장이 차단됐습니다. {data.bundleStorageError}
        </p>
      )}
    </>
  );
}
function WindowLaunch({ data, invoke }: NativePanelProps) {
  const [slot, setSlot] = useState("");
  const [view, setView] = useState("");
  const config = data.wm.configuration;
  const run = async (id: string) => {
    if (!config || !slot || !id) return;
    const preview = await invoke({
      kind: "windows.session",
      command: {
        kind: "preview",
        request: {
          id: crypto.randomUUID(),
          expected_revision: config.revision,
          scope: [slot],
          targets: [
            {
              view: id,
              roots: Object.fromEntries(
                Object.keys(config.views[id].roots).map((root) => [root, slot]),
              ),
            },
          ],
          mode: "open",
          retain: [],
          selected_tabs: {},
          focus: null,
        },
      },
    });
    if (preview.ok)
      await invoke({
        kind: "windows.session",
        command: { kind: "apply", plan: (preview.result as NativePlan).plan },
      });
  };
  if (!data.wm.connected)
    return (
      <button onClick={() => void invoke({ kind: "windows.connect" })}>
        창 관리자 연결
      </button>
    );
  return (
    <>
      <select
        aria-label="배치할 화면 영역"
        value={slot}
        onChange={(event) => setSlot(event.target.value)}
      >
        <option value="">화면 영역 선택</option>
        {Object.values(config?.slots ?? {}).map((item) => (
          <option key={item.id} value={item.id}>
            {item.name}
          </option>
        ))}
      </select>
      <select
        aria-label="실행할 저장된 배치"
        disabled={!slot}
        value={view}
        onChange={(event) => {
          setView(event.target.value);
          void run(event.target.value);
        }}
      >
        <option value="">저장된 배치 선택</option>
        {Object.values(config?.views ?? {}).map((item) => (
          <option key={item.id} value={item.id}>
            {item.name}
          </option>
        ))}
      </select>
      <button disabled={!slot || !view} onClick={() => void run(view)}>
        시작
      </button>
    </>
  );
}
export function NativeDisplay({ data, invoke }: NativePanelProps) {
  return (
    <>
      <ComponentPresets data={data} invoke={invoke} component="display" />
      <div className="section-heading">
        <h1>Display Relay</h1>
      </div>
      <Mirrors data={data} invoke={invoke} />
      <button onClick={() => void invoke({ kind: "settings.save" })}>
        현재 설정 저장
      </button>
    </>
  );
}
function Mirrors({ data, invoke }: NativePanelProps) {
  const settings = [
    data.settings.last_used.display,
    ...data.settings.last_used.additional_displays,
  ];
  const update = (index: number, value: MirrorSettings) =>
    void invoke({ kind: "mirror.settings", index, settings: value });
  return (
    <fieldset className="mirror-controls">
      <legend>
        디스플레이 미러링 <small>영상은 별도 네이티브 창에 표시</small>
      </legend>
      {settings.map((settings, index) => {
        const job = data.jobs.mirrors[index];
        return (
          <div className="mirror-row" key={index}>
            <div className="section-heading">
              <strong>미러링 {index + 1}</strong>
              <NativeStatus job={job} />
            </div>
            <fieldset disabled={active(job)}>
              <label>
                원본 디스플레이
                <select
                  aria-label={`미러링 ${index + 1} 디스플레이`}
                  value={settings.display}
                  onChange={(event) =>
                    update(index, {
                      ...settings,
                      display: event.target.value,
                      display_label:
                        data.displays.find(
                          (display) => display.id === event.target.value,
                        )?.name ?? event.target.value,
                      enabled: !!event.target.value,
                    })
                  }
                >
                  <option value="">디스플레이 선택</option>
                  {settings.display &&
                    !data.displays.some(
                      (display) => display.id === settings.display,
                    ) && (
                      <option value={settings.display}>
                        {settings.display_label} · 연결 안 됨
                      </option>
                    )}
                  {data.displays.map((display) => (
                    <option key={display.id} value={display.id}>
                      {display.name} · {display.width}×{display.height}
                    </option>
                  ))}
                </select>
              </label>
              <div className="form-row">
                <label className="check-label">
                  <input
                    type="checkbox"
                    checked={settings.enabled}
                    disabled={!settings.display}
                    onChange={(event) =>
                      update(index, {
                        ...settings,
                        enabled: event.target.checked,
                      })
                    }
                  />
                  미러링 사용
                </label>
                <label className="check-label">
                  <input
                    type="checkbox"
                    checked={settings.fullscreen}
                    onChange={(event) =>
                      update(index, {
                        ...settings,
                        fullscreen: event.target.checked,
                      })
                    }
                  />
                  전체 화면
                </label>
                <label>
                  FPS
                  <select
                    aria-label={`미러링 ${index + 1} FPS`}
                    value={settings.fps}
                    onChange={(event) =>
                      update(index, {
                        ...settings,
                        fps: Number(event.target.value),
                      })
                    }
                  >
                    {[...new Set([settings.fps, 15, 30, 60, 120, 240])]
                      .sort((a, b) => a - b)
                      .map((fps) => (
                        <option key={fps}>{fps}</option>
                      ))}
                  </select>
                </label>
              </div>
            </fieldset>
            <div className="action-row">
              <button
                className="primary"
                disabled={
                  active(job) ||
                  !settings.enabled ||
                  !data.displays.some(
                    (display) => display.id === settings.display,
                  )
                }
                onClick={() => void invoke({ kind: "mirror.start", index })}
              >
                미러링 창 열기
              </button>
              <button
                disabled={!active(job)}
                onClick={() => void invoke({ kind: "mirror.stop", index })}
              >
                닫기
              </button>
              {index > 0 && (
                <button
                  disabled={data.jobs.mirrors.some(active)}
                  onClick={() => void invoke({ kind: "mirror.remove", index })}
                >
                  행 삭제
                </button>
              )}
            </div>
            <NativeJobLog job={job} />
          </div>
        );
      })}
      <button onClick={() => void invoke({ kind: "mirror.add" })}>
        미러링 추가
      </button>
    </fieldset>
  );
}
export function NativeCablePanel({ data, invoke }: NativePanelProps) {
  const cableRadioName = useId();
  const settings = data.cableSettings;
  const job = data.jobs.cable;
  const outputs = data.endpoints.filter(
    (endpoint) => endpoint.flow === "playback",
  );
  const captures = data.endpoints.filter(
    (endpoint) => endpoint.flow === "capture",
  );
  const cables = [...data.cables];
  if (
    settings.cable &&
    !cables.some((cable) => cable.playback_id === settings.cable!.playback_id)
  )
    cables.push(settings.cable);
  const volume = settings.muted
    ? 0
    : readablePercent(100 * 10 ** (settings.gain_db / 20));
  const [manualName, setManualName] = useState("수동 케이블");
  const [playback, setPlayback] = useState("");
  const [capture, setCapture] = useState("");
  const invalid =
    !settings.cable ||
    !outputs.some((output) => output.id === settings.output_id) ||
    settings.output_id === settings.cable.playback_id;
  return (
    <>
      <Presets
        names={settings.presets.map((preset) => preset.name)}
        save={(name) => void invoke({ kind: "cable.preset.save", name })}
        load={(name) => void invoke({ kind: "cable.preset.run", name })}
        remove={(name) => void invoke({ kind: "cable.preset.delete", name })}
      />
      <div className="section-heading">
        <div>
          <h1>가상 케이블 모니터링</h1>
          <p>케이블 자체를 바꾸지 않고 선택한 출력으로 듣습니다.</p>
        </div>
        <NativeStatus job={job} />
      </div>
      <fieldset className="choices">
        <legend>
          설치된 케이블 <small>하나 선택</small>
        </legend>
        {cables.map((cable) => (
          <label
            className={`choice ${settings.cable?.playback_id === cable.playback_id ? "selected" : ""}`}
            key={cable.playback_id}
          >
            <input
              type="radio"
              name={cableRadioName}
              checked={settings.cable?.playback_id === cable.playback_id}
              onChange={() =>
                void invoke({ kind: "cable.select", id: cable.playback_id })
              }
            />
            <span className="choice-copy">
              <strong>{cable.name}</strong>
              <small>
                {cable.provider === "manual" ? "수동 연결" : "설치된 드라이버"}
              </small>
            </span>
            {settings.cable?.playback_id === cable.playback_id && (
              <span className="selected-badge">선택됨</span>
            )}
          </label>
        ))}
        {!cables.length && (
          <p>
            인식한 케이블이 없습니다. 아래에서 재생·녹음 장치를 수동 연결할 수
            있습니다.
          </p>
        )}
      </fieldset>
      <details className="manual-cable">
        <summary>재생·녹음 장치 수동 연결</summary>
        <label>
          케이블 이름
          <input
            value={manualName}
            onChange={(event) => setManualName(event.target.value)}
          />
        </label>
        <label>
          케이블의 재생 장치
          <select
            value={playback}
            onChange={(event) => setPlayback(event.target.value)}
          >
            <option value="">선택하세요</option>
            {outputs.map((endpoint) => (
              <option key={endpoint.id} value={endpoint.id}>
                {endpoint.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          케이블의 녹음 장치
          <select
            value={capture}
            onChange={(event) => setCapture(event.target.value)}
          >
            <option value="">선택하세요</option>
            {captures.map((endpoint) => (
              <option key={endpoint.id} value={endpoint.id}>
                {endpoint.name}
              </option>
            ))}
          </select>
        </label>
        <button
          disabled={!playback || !capture || !manualName.trim()}
          onClick={() =>
            void invoke({
              kind: "cable.manual",
              name: manualName,
              playback,
              capture,
            })
          }
        >
          이 장치 쌍 사용
        </button>
      </details>
      <Choices
        name="모니터링 출력"
        outputs={outputs}
        selected={settings.output_id}
        choose={(id) => void invoke({ kind: "cable.output", id })}
      />
      <div className="volume-control">
        <VolumeControl
          value={volume}
          commit={(value) => {
            void invoke({ kind: "cable.volume", value });
          }}
        />
        <label className="check-label">
          <input
            type="checkbox"
            checked={settings.muted}
            onChange={(event) =>
              void invoke({ kind: "cable.mute", muted: event.target.checked })
            }
          />
          음소거
        </label>
      </div>
      {invalid && (
        <p className="inline-error">
          케이블과 다른 모니터링 출력을 선택하세요.
        </p>
      )}
      <div className="action-row">
        <button
          className="primary"
          disabled={invalid || active(job)}
          onClick={() => void invoke({ kind: "cable.start" })}
        >
          모니터링 시작
        </button>
        <button
          disabled={!active(job)}
          onClick={() => void invoke({ kind: "cable.stop" })}
        >
          모니터링 중지
        </button>
        <button
          disabled={!!data.cableStorageError}
          onClick={() => void invoke({ kind: "cable.save" })}
        >
          설정 저장
        </button>
      </div>
      {data.cableStorageError && (
        <p className="inline-error" role="alert">
          {data.cableStorageError}
        </p>
      )}
      <NativeJobLog job={job} />
    </>
  );
}

export function NativeWindows({ data, invoke }: NativePanelProps) {
  const wm = data.wm;
  const config = wm.configuration;
  const [viewId, setView] = usePanelState("wm.view", "");
  const [root, setRoot] = usePanelState("wm.root", "");
  const [slot, setSlot] = usePanelState("wm.slot", "");
  useEffect(() => {
    if (wm.bundleTarget) {
      setView(wm.bundleTarget.view);
      setSlot(wm.bundleTarget.slot);
      if (wm.bundleTarget.request) {
        setLastRequest(wm.bundleTarget.request);
        setPreview(undefined);
      }
    }
  }, [wm.bundleTarget?.view, wm.bundleTarget?.slot, wm.bundleTarget?.request]);
  const [selected, setSelected] = usePanelState<string[]>("wm.selected", []);
  const [mode, setMode] = usePanelState("wm.mode", "open");
  const [preview, setPreview] = usePanelState<NativePlan | undefined>(
    "wm.preview",
    undefined,
  );
  const [message, setMessage] = usePanelState("wm.message", "");
  const [dirty] = usePanelState("wm.dirty", false);
  const [lastRequest, setLastRequest] = usePanelState("wm.lastRequest", "");
  useEffect(() => {
    const candidates = new Set(wm.inventory.map((candidate) => candidate.id));
    setSelected((previous) => {
      const next = previous.filter((id) => candidates.has(id));
      return next.length === previous.length ? previous : next;
    });
  }, [wm.inventory]);
  useEffect(() => {
    if (config && !config.views[viewId])
      setView(Object.keys(config.views)[0] ?? "");
    if (config && !config.slots[slot])
      setSlot(Object.keys(config.slots)[0] ?? "");
  }, [config, viewId, slot]);
  useEffect(() => {
    const roots = config?.views[viewId]?.roots ?? {};
    if (!(root in roots)) setRoot(Object.keys(roots)[0] ?? "");
  }, [config, viewId, root]);
  useEffect(
    () => setPreview(undefined),
    [config?.revision, wm.connected, viewId, root, slot, mode],
  );
  const session = (command: NativeCommand) =>
    invoke({ kind: "windows.session", command });
  const makePreview = async () => {
    if (!config || !viewId || !root || !slot) return;
    const result = await session({
      kind: "preview",
      request: {
        id: crypto.randomUUID(),
        expected_revision: config.revision,
        scope: [slot],
        targets: [{ view: viewId, roots: { [root]: slot } }],
        mode,
        retain: [],
        selected_tabs: {},
        focus: null,
      },
    });
    if (result.ok) setPreview(result.result as NativePlan);
  };
  return (
    <>
      <div className="section-heading">
        <div>
          <h1>창 배치</h1>
          <p>기존 창과 저장된 배치를 연결하고, 변경을 확인한 뒤 적용합니다.</p>
        </div>
        <span className="tag">{wm.connected ? "연결됨" : "연결 안 됨"}</span>
      </div>
      <div className="action-row">
        {!wm.connected ? (
          <button
            className="primary"
            onClick={() => void invoke({ kind: "windows.connect" })}
          >
            창 관리자 연결
          </button>
        ) : (
          <>
            <button onClick={() => void invoke({ kind: "refresh" })}>
              목록 새로 검색
            </button>
            <button onClick={() => void invoke({ kind: "windows.disconnect" })}>
              연결 종료
            </button>
            <button onClick={() => void session({ kind: "recover" })}>
              숨긴 창 복구
            </button>
          </>
        )}
      </div>
      {wm.error && (
        <p className="inline-error" role="alert">
          {wm.error}
        </p>
      )}
      {wm.connected && config && (
        <>
          <div className="form-row">
            <label>
              배치
              <select
                value={viewId}
                onChange={(event) => setView(event.target.value)}
              >
                {Object.values(config.views).map((view) => (
                  <option key={view.id} value={view.id}>
                    {view.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              루트
              <select
                value={root}
                onChange={(event) => setRoot(event.target.value)}
              >
                {Object.keys(config.views[viewId]?.roots ?? {}).map((root) => (
                  <option key={root}>{root}</option>
                ))}
              </select>
            </label>
            <label>
              화면 영역
              <select
                value={slot}
                onChange={(event) => setSlot(event.target.value)}
              >
                <option value="">영역 선택</option>
                {Object.values(config.slots).map((slot) => (
                  <option key={slot.id} value={slot.id}>
                    {slot.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <details className="native-inventory">
            <summary>현재 열린 창 등록 · {wm.inventory.length}개</summary>
            <fieldset className="window-list">
              <legend>
                추가할 창 <small>여러 개 선택</small>
              </legend>
              {wm.inventory.map((candidate) => (
                <label
                  className={`window-row ${selected.includes(candidate.id) ? "selected" : ""}`}
                  key={candidate.id}
                >
                  <input
                    type="checkbox"
                    checked={selected.includes(candidate.id)}
                    onChange={(event) =>
                      setSelected((ids) =>
                        event.target.checked
                          ? [...ids, candidate.id]
                          : ids.filter((id) => id !== candidate.id),
                      )
                    }
                  />
                  <span>
                    <strong>{candidate.title || "제목 없는 창"}</strong>
                    <small>
                      {candidate.class} · PID {candidate.process}
                    </small>
                  </span>
                </label>
              ))}
            </fieldset>
            <button
              disabled={!selected.length || !viewId || !root}
              onClick={async () => {
                const result = await invoke({
                  kind: "windows.add",
                  candidates: selected,
                  view: viewId,
                  root,
                });
                if (result.ok) setSelected([]);
              }}
            >
              선택한 창을 배치에 등록
            </button>
          </details>
          <fieldset className="window-list">
            <legend>
              등록된 창 <small>총 {Object.keys(config.windows).length}개</small>
            </legend>
            {Object.values(config.windows).map((window) => (
              <div className="window-row" key={window.id}>
                <span>
                  <strong>{window.alias}</strong>
                  <small>
                    {wm.snapshot?.windows[window.id]?.available
                      ? "연결됨"
                      : "연결 안 됨"}
                  </small>
                </span>
                <details>
                  <summary>다른 창에 연결</summary>
                  {wm.inventory.map((candidate) => (
                    <button
                      key={candidate.id}
                      onClick={() =>
                        void session({
                          kind: "bind",
                          expected_revision: config.revision,
                          window: window.id,
                          candidate: candidate.id,
                        })
                      }
                    >
                      {candidate.title || candidate.class}
                    </button>
                  ))}
                </details>
              </div>
            ))}
          </fieldset>
          <LayoutEditor
            data={data}
            invoke={invoke}
            viewId={viewId}
            setView={setView}
            setSlot={setSlot}
          />
          <div className="form-row">
            <label>
              전환 방식
              <select
                value={mode}
                onChange={(event) => setMode(event.target.value)}
              >
                {[
                  ["open", "배치 열기"],
                  ["keep_size", "크기 유지"],
                  ["keep_here", "현재 위치 유지"],
                  ["bring", "가져오기"],
                  ["restore", "기본 규칙 복원"],
                  ["reflow", "재배치"],
                ].map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <div className="action-row">
            <button
              disabled={dirty || !viewId || !root || !slot}
              onClick={() => void makePreview()}
            >
              배치 미리보기
            </button>
            <button
              className="primary"
              disabled={
                dirty || !preview || !!Object.keys(preview.blocked).length
              }
              onClick={async () => {
                if (!preview) return;
                const plan = preview;
                setPreview(undefined);
                const result = await session({
                  kind: "apply",
                  plan: plan.plan,
                });
                if (result.ok) {
                  setLastRequest(plan.request);
                  setMessage(
                    `적용 결과: ${(result.result as { status: string }).status}`,
                  );
                }
              }}
            >
              실제 창에 적용
            </button>
            <button
              disabled={!lastRequest || dirty}
              onClick={async () => {
                const result = await session({ kind: "undo", id: lastRequest });
                if (result.ok) {
                  setPreview(result.result as NativePlan);
                  setMessage(
                    "되돌리기 미리보기를 확인하고 실제 창에 적용하세요.",
                  );
                  setLastRequest("");
                }
              }}
            >
              마지막 적용 되돌리기 미리보기
            </button>
            <button
              onClick={() =>
                void session({ kind: "pause", paused: !wm.snapshot?.paused })
              }
            >
              {wm.snapshot?.paused
                ? "자동 재배치 재개"
                : "자동 재배치 일시중지"}
            </button>
          </div>
          {dirty && (
            <p role="status">
              배치 편집을 저장하거나 취소한 뒤 미리보기·적용을 진행하세요.
            </p>
          )}
          {preview && (
            <div className="preview-card" aria-label="배치 미리보기 결과">
              <strong>
                이동 {preview.impact.moved} · 크기 변경 {preview.impact.resized}{" "}
                · 숨김 {preview.impact.hidden}
              </strong>
              {Object.values({ ...preview.desired, ...preview.restored }).map(
                (desired) => (
                  <div key={desired.window}>
                    <span>
                      {config.windows[desired.window]?.alias ?? desired.window}
                    </span>
                    <b>
                      {desired.frame.x}, {desired.frame.y} ·{" "}
                      {desired.frame.width}×{desired.frame.height}
                    </b>
                  </div>
                ),
              )}
              {preview.diagnostics.map((diagnostic, index) => (
                <p key={index}>{diagnostic}</p>
              ))}
              {Object.entries(preview.blocked).map(([id, error]) => (
                <p className="inline-error" key={id}>
                  {config.windows[id]?.alias ?? id}: {error.code} ·{" "}
                  {error.message}
                </p>
              ))}
            </div>
          )}
          {message && <p role="status">{message}</p>}
        </>
      )}
      {!wm.connected && (
        <p className="muted">
          기존 Window Manager가 설정을 사용 중이면 먼저 종료하세요. 연결만으로
          창을 이동하거나 배치를 적용하지 않습니다.
        </p>
      )}
    </>
  );
}

function LayoutEditor({
  data,
  invoke,
  viewId,
  setView,
  setSlot,
}: NativePanelProps & {
  viewId: string;
  setView(value: string): void;
  setSlot(value: string): void;
}) {
  const config = data.wm.configuration!;
  const [draft, setDraft] = usePanelState<NativeConfiguration>(
    "wm.draft",
    config,
  );
  const [dirty, setDirty] = usePanelState("wm.dirty", false);
  const [name, setName] = useState("");
  const [monitor, setMonitor] = useState("");
  const [slotName, setSlotName] = useState("");
  useEffect(() => {
    if (!dirty) setDraft(config);
  }, [config, dirty]);
  const patchNode = (id: string, patch: Partial<NativeNode>) => {
    const next = structuredClone(draft);
    const walk = (node: NativeNode) => {
      if (node.id === id) Object.assign(node, patch);
      node.children?.forEach(walk);
    };
    Object.values(next.views[viewId]?.roots ?? {}).forEach(walk);
    setDraft(next);
    setDirty(true);
  };
  const changeChild = (
    parentId: string,
    index: number,
    direction: number | null,
  ) => {
    const next = structuredClone(draft);
    const walk = (node: NativeNode) => {
      if (node.id === parentId && node.children) {
        const ratios = node.ratios as number[] | undefined;
        if (direction === null) {
          node.children.splice(index, 1);
          if (ratios?.length) ratios.splice(index, 1);
        } else {
          const destination = index + direction;
          if (destination >= 0 && destination < node.children.length) {
            [node.children[index], node.children[destination]] = [
              node.children[destination],
              node.children[index],
            ];
            if (ratios?.length)
              [ratios[index], ratios[destination]] = [
                ratios[destination],
                ratios[index],
              ];
          }
        }
      } else node.children?.forEach(walk);
    };
    Object.values(next.views[viewId]?.roots ?? {}).forEach(walk);
    setDraft(next);
    setDirty(true);
  };
  const tree = (node: NativeNode, parent?: NativeNode, index = 0): ReactNode =>
    node.type === "group" ? (
      <div className="node-editor" key={node.id}>
        <div className="form-row">
          <input
            aria-label="그룹 이름"
            value={node.name ?? ""}
            onChange={(event) =>
              patchNode(node.id, { name: event.target.value })
            }
          />
          <select
            aria-label="그룹 배치 방식"
            value={node.strategy}
            onChange={(event) =>
              patchNode(node.id, { strategy: event.target.value })
            }
          >
            {[
              ["horizontal", "가로"],
              ["vertical", "세로"],
              ["grid", "격자"],
              ["flow", "흐름"],
              ["free", "자유 위치"],
              ["semantic_tabs", "선택 창만 표시"],
            ].map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
          <label>
            간격
            <input
              value={node.gap ?? "8"}
              onChange={(event) =>
                patchNode(node.id, { gap: event.target.value })
              }
            />
          </label>
          {node.strategy === "grid" && (
            <label>
              열 수 규칙
              <input
                aria-label="격자 열 수 규칙"
                value={node.columns ?? ""}
                onChange={(event) =>
                  patchNode(node.id, { columns: event.target.value })
                }
              />
            </label>
          )}
        </div>
        {node.children?.map((child, index) => tree(child, node, index))}
      </div>
    ) : (
      <div className="placement-node" key={node.id}>
        {draft.windows[node.window ?? ""]?.alias ?? node.role}
        {parent && (
          <span className="action-row">
            <button
              aria-label="창 순서 위로"
              disabled={index === 0}
              onClick={() => changeChild(parent.id, index, -1)}
            >
              ↑
            </button>
            <button
              aria-label="창 순서 아래로"
              disabled={index === (parent.children?.length ?? 0) - 1}
              onClick={() => changeChild(parent.id, index, 1)}
            >
              ↓
            </button>
            <button onClick={() => changeChild(parent.id, index, null)}>
              배치에서 빼기
            </button>
          </span>
        )}
      </div>
    );
  const saveDraft = async () => {
    const result = await invoke({
      kind: "windows.session",
      command: {
        kind: "configure",
        expected_revision: config.revision,
        configuration: draft,
      },
    });
    if (result.ok) setDirty(false);
  };
  return (
    <details className="layout-editor">
      <summary>배치·그룹·화면 영역 편집</summary>
      {draft.revision !== config.revision && dirty && (
        <p role="alert" className="inline-error">
          다른 창에서 배치가 변경됐습니다. 편집을 취소하고 다시 읽으세요.
        </p>
      )}
      <label>
        배치 이름
        <input
          value={draft.views[viewId]?.name ?? ""}
          onChange={(event) => {
            const next = structuredClone(draft);
            if (next.views[viewId]) {
              next.views[viewId].name = event.target.value;
              setDraft(next);
              setDirty(true);
            }
          }}
        />
      </label>
      {Object.values(draft.views[viewId]?.roots ?? {}).map((node) =>
        tree(node),
      )}
      <details>
        <summary>창별 동작 허용</summary>
        {Object.values(draft.windows).map((window) => (
          <fieldset key={window.id}>
            <legend>{window.alias}</legend>
            {(
              [
                ["allow_move", "이동 허용"],
                ["allow_resize", "크기 변경 허용"],
                ["allow_show_state", "최소화·최대화 허용"],
                ["allow_dpi_transfer", "다른 DPI 화면으로 이동 허용"],
              ] as const
            ).map(([key, label]) => (
              <label className="check-label" key={key}>
                <input
                  type="checkbox"
                  checked={window.capabilities[key]}
                  onChange={(event) => {
                    const next = structuredClone(draft);
                    next.windows[window.id].capabilities[key] =
                      event.target.checked;
                    setDraft(next);
                    setDirty(true);
                  }}
                />
                {label}
              </label>
            ))}
            <label className="check-label">
              <input
                type="checkbox"
                checked={window.allow_hide}
                onChange={(event) => {
                  const next = structuredClone(draft);
                  next.windows[window.id].allow_hide = event.target.checked;
                  setDraft(next);
                  setDirty(true);
                }}
              />
              선택되지 않은 창 숨김 허용
            </label>
          </fieldset>
        ))}
      </details>
      <details>
        <summary>저장된 화면 영역 조정</summary>
        {Object.values(draft.slots).map((slot) => (
          <fieldset key={slot.id}>
            <legend>{slot.name}</legend>
            <div className="form-row">
              {(["왼쪽", "위쪽", "너비", "높이"] as const).map(
                (label, index) => (
                  <label key={label}>
                    {label} (0~1)
                    <input
                      type="number"
                      min="0"
                      max="1"
                      step="0.01"
                      value={slot.region[index]}
                      onChange={(event) => {
                        const next = structuredClone(draft);
                        next.slots[slot.id].region[index] = Number(
                          event.target.value,
                        );
                        setDraft(next);
                        setDirty(true);
                      }}
                    />
                  </label>
                ),
              )}
            </div>
          </fieldset>
        ))}
      </details>
      <div className="action-row">
        <button
          disabled={!dirty || draft.revision !== config.revision}
          onClick={() => void saveDraft()}
        >
          배치 편집 저장
        </button>
        <button
          disabled={!dirty}
          onClick={() => {
            setDirty(false);
            setDraft(config);
          }}
        >
          편집 취소
        </button>
      </div>
      <div className="form-row">
        <input
          aria-label="새 배치 이름"
          placeholder="새 배치 이름"
          value={name}
          onChange={(event) => setName(event.target.value)}
        />
        <button
          disabled={dirty || !name.trim()}
          onClick={async () => {
            const result = await invoke({ kind: "windows.view.add", name });
            if (result.ok) {
              setView((result.result as { view: string }).view);
              setName("");
            }
          }}
        >
          새 배치
        </button>
      </div>
      <fieldset>
        <legend>화면 영역 추가</legend>
        <input
          aria-label="화면 영역 이름"
          placeholder="영역 이름"
          value={slotName}
          onChange={(event) => setSlotName(event.target.value)}
        />
        <select
          aria-label="영역 디스플레이"
          value={monitor}
          onChange={(event) => setMonitor(event.target.value)}
        >
          <option value="">화면 선택</option>
          {data.wm.monitors?.monitors.map((item) => (
            <option key={item.device} value={item.device}>
              {item.name} · {item.work_area.width}×{item.work_area.height} · DPI{" "}
              {item.dpi}
            </option>
          ))}
        </select>
        <button
          disabled={dirty || !slotName.trim() || !monitor}
          onClick={async () => {
            const result = await invoke({
              kind: "windows.slot.add",
              name: slotName,
              device: monitor,
            });
            if (result.ok) {
              setSlot((result.result as { slot: string }).slot);
              setSlotName("");
            }
          }}
        >
          전체 화면 영역 추가
        </button>
      </fieldset>
    </details>
  );
}
