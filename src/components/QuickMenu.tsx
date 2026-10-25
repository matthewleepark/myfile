import { useEffect, useRef } from "react";

import type { ActionId, ActionInfo } from "../api";
import { ActionIcon } from "./ActionList";

interface Props {
  fileName: string;
  actions: ActionInfo[];
  /** Where the file was dropped, in CSS pixels relative to the window. */
  x: number;
  y: number;
  onPick: (id: ActionId) => void;
  onClose: () => void;
}

const WIDTH = 300;

/**
 * The menu shown when a file is dropped with Shift held: the file's actions
 * right where it landed. Arrow keys move, Enter picks, Escape closes.
 */
export function QuickMenu({ fileName, actions, x, y, onPick, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    ref.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [onClose]);

  function onKeyDown(e: React.KeyboardEvent) {
    const buttons = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button") ?? []);
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "Escape") onClose();
    else if (e.key === "ArrowDown") buttons[(i + 1) % buttons.length]?.focus();
    else if (e.key === "ArrowUp") buttons[(i - 1 + buttons.length) % buttons.length]?.focus();
    else return;
    e.preventDefault();
  }

  const left = Math.max(12, Math.min(x, window.innerWidth - WIDTH - 12));
  const top = Math.max(12, Math.min(y, window.innerHeight - (64 + actions.length * 58) - 12));

  return (
    <div
      ref={ref}
      className="quick-menu"
      style={{ left, top, width: WIDTH }}
      role="menu"
      aria-label={`Actions for ${fileName}`}
      onKeyDown={onKeyDown}
    >
      <p className="quick-menu-title" title={fileName}>
        {fileName}
      </p>
      {actions.length === 0 ? (
        <p className="muted small quick-menu-empty">No actions for this type of file yet.</p>
      ) : (
        actions.map((a) => (
          <button key={a.id} type="button" role="menuitem" className="quick-item" onClick={() => onPick(a.id)}>
            <span className="action-icon small">
              <ActionIcon id={a.id} />
            </span>
            <span className="quick-item-label">
              {a.label}
              {a.usesAi && <span className="tag">AI</span>}
            </span>
          </button>
        ))
      )}
    </div>
  );
}
