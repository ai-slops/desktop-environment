export type NativeCommand = { kind: string; [key: string]: unknown };
export type NativeEndpoint = {
  id: string;
  name: string;
  flow: "playback" | "capture";
  isDefault: boolean;
};
export type NativeCable = {
  name: string;
  provider: string;
  playback_id: string;
  capture_id: string;
};
export type NativeJob = { phase: string; error: string | null; logs: string[] };
export type MirrorSettings = {
  enabled: boolean;
  display: string;
  display_label: string;
  fullscreen: boolean;
  fps: number;
  timeout_ms: number;
};
export type NativeSettings = {
  audio: {
    enabled: boolean;
    source: string;
    source_label: string;
    target: string;
    target_label: string;
  };
  display: MirrorSettings;
  additional_displays: MirrorSettings[];
};
export type CableSettings = {
  version: number;
  cable: NativeCable | null;
  output_id: string;
  gain_db: number;
  muted: boolean;
  presets: {
    name: string;
    cable: NativeCable | null;
    output_id: string;
    gain_db: number;
    muted: boolean;
  }[];
};
export type Rect = { x: number; y: number; width: number; height: number };
export type NativeNode = {
  type: "group" | "placement";
  id: string;
  name?: string;
  strategy?: string;
  children?: NativeNode[];
  window?: string;
  role?: string;
  gap?: string;
  columns?: string;
  [key: string]: unknown;
};
export type NativeWindow = {
  id: string;
  alias: string;
  tags: { name: string; source: string }[];
  allow_hide: boolean;
  capabilities: {
    allow_move: boolean;
    allow_resize: boolean;
    allow_show_state: boolean;
    allow_dpi_transfer: boolean;
  };
  protection: {
    geometry_lock: boolean;
    maintain_visible: boolean;
    keep_monitor: boolean;
    prohibit_focus: boolean;
  };
  [key: string]: unknown;
};
export type NativeConfiguration = {
  version: number;
  revision: number;
  workspaces: Record<
    string,
    { id: string; name: string; remembered_view: string | null }
  >;
  windows: Record<string, NativeWindow>;
  views: Record<
    string,
    {
      id: string;
      workspace: string;
      name: string;
      roots: Record<string, NativeNode>;
    }
  >;
  slots: Record<
    string,
    {
      id: string;
      name: string;
      display: string;
      region: number[];
      fallback_displays: string[];
    }
  >;
  compositions: Record<string, unknown>;
  collections: Record<string, unknown>;
  shortcuts: { number: number; target: unknown }[];
};
export type NativePlan = {
  plan: string;
  request: string;
  revision: number;
  topology_revision: number;
  desired: Record<string, { window: string; slot: string; frame: Rect }>;
  restored?: Record<string, { window: string; slot: string; frame: Rect }>;
  impact: { moved: number; resized: number; hidden: number; shown: number };
  diagnostics: string[];
  blocked: Record<string, { code: string; message: string }>;
};
export type NativeSnapshot = {
  revision: number;
  configRoot: string;
  errors: string[];
  storageError: string | null;
  cableStorageError: string | null;
  settings: {
    last_used: NativeSettings;
    presets: { name: string; settings: NativeSettings }[];
  };
  cableSettings: CableSettings;
  bundlePresets?: {
    name: string;
    components: string[];
    window?: { view: string; slot: string } | null;
  }[];
  bundleStorageError?: string | null;
  endpoints: NativeEndpoint[];
  cables: NativeCable[];
  displays: { id: string; name: string; width: number; height: number }[];
  jobs: { audio: NativeJob; cable: NativeJob; mirrors: NativeJob[] };
  traces: {
    id: string;
    timestamp: number;
    operation: string;
    status: "ok" | "error";
    message: string;
  }[];
  wm: {
    bundleTarget?: { view: string; slot: string; request?: string };
    connected: boolean;
    error: string | null;
    configuration: NativeConfiguration | null;
    inventory: {
      id: string;
      title: string;
      class: string;
      process: number;
      frame: Rect;
      reference?: string | null;
    }[];
    snapshot: {
      revision: number;
      paused: boolean;
      windows: Record<
        string,
        { available: boolean; frame: Rect | null; slot: string | null }
      >;
      displays: Record<string, { id: string; area: Rect }>;
      presentations: Record<string, unknown>;
    } | null;
    monitors: {
      epoch: string;
      monitors: {
        device: string;
        hardware: string | null;
        name: string;
        work_area: Rect;
        dpi: number;
      }[];
    } | null;
    lastResult: unknown;
  };
};
