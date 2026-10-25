import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

export interface DropPoint {
  /** CSS pixels from the top-left of the window's content. */
  x: number;
  y: number;
}

/**
 * Listens for files dragged onto the window from File Explorer / Finder.
 *
 * Tauri's native drag-drop event is used instead of HTML5 drag-and-drop
 * because only the native event gives us real filesystem paths.
 */
export function useFileDrop(onDrop: (paths: string[], at: DropPoint) => void) {
  const [dragging, setDragging] = useState(false);
  const onDropRef = useRef(onDrop);
  onDropRef.current = onDrop;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter") setDragging(true);
        else if (p.type === "leave") setDragging(false);
        else if (p.type === "drop") {
          setDragging(false);
          // The position arrives in physical pixels; the page uses CSS pixels.
          const scale = window.devicePixelRatio || 1;
          const at = { x: p.position.x / scale, y: p.position.y / scale };
          if (p.paths.length > 0) onDropRef.current(p.paths, at);
        }
      })
      .then((fn) => {
        // React StrictMode mounts twice in development; don't leak a listener.
        if (disposed) fn();
        else unlisten = fn;
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return dragging;
}
