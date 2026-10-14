const { contextBridge, ipcRenderer } = require("electron");
contextBridge.exposeInMainWorld("desktopShell", {
  version: 1,
  close: () => ipcRenderer.send("workspace:close-ui"),
  dragFinished: (point) => ipcRenderer.invoke("workspace:drag-finished", point),
});
window.addEventListener("DOMContentLoaded", () => {
  document.documentElement.dataset.desktopShell = "true";
});
