import type { DockviewApi, IDockviewPanel } from "dockview-react";
declare global {
  interface Window {
    desktopShell?: {
      version: number;
      close(): void;
      dragFinished(point: {
        x: number;
        y: number;
      }): Promise<{ x: number; y: number } | null>;
    };
  }
}

export function installTearOff(api: DockviewApi) {
  const listeners: Array<() => void> = [];
  const subscription = api.onWillDragPanel((event) => {
    const panel = event.panel;
    const target = event.nativeEvent.target as Node | null;
    const document = target?.ownerDocument ?? window.document;
    const owner = document.defaultView;
    if (!owner?.desktopShell) return;
    let escaped = false;
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") escaped = true;
    };
    const end = (event: DragEvent) => {
      document.removeEventListener("keydown", key, true);
      document.removeEventListener("dragend", end, true);
      if (escaped || event.dataTransfer?.dropEffect !== "none") return;
      void owner
        .desktopShell!.dragFinished({ x: event.screenX, y: event.screenY })
        .then((point) => {
          if (!point || !api.getPanel(panel.id)) return;
          void tearOff(api, panel, point.x, point.y);
        });
    };
    document.addEventListener("keydown", key, true);
    document.addEventListener("dragend", end, true);
    listeners.push(() => {
      document.removeEventListener("keydown", key, true);
      document.removeEventListener("dragend", end, true);
    });
  });
  return {
    dispose() {
      subscription.dispose();
      listeners.forEach((remove) => remove());
    },
  };
}
function tearOff(
  api: DockviewApi,
  panel: IDockviewPanel,
  x: number,
  y: number,
) {
  return api.addPopoutGroup(panel, {
    popoutUrl: "/popout.html",
    position: {
      left: Math.round(x - 30),
      top: Math.round(y - 20),
      width: 700,
      height: 600,
    },
  });
}
