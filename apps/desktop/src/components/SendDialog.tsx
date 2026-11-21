import { useState } from "react";

export interface SendTarget {
  value: string;
  label: string;
}

interface Props {
  title: string;
  what: string;
  targets: SendTarget[];
  targetLabel: string;
  dirDefault: string;
  emptyText: string;
  onCancel: () => void;
  onSend: (target: string, dir: string) => void;
}

/** Pick where a file goes on another host or device; the choice is shown in full before sending. */
export function SendDialog({ title, what, targets, targetLabel, dirDefault, emptyText, onCancel, onSend }: Props) {
  const [target, setTarget] = useState(targets[0]?.value ?? "");
  const [dir, setDir] = useState(dirDefault);
  const chosen = targets.find((t) => t.value === target);
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="send-title">
        <h2 id="send-title">{title}</h2>
        <p className="muted small">{what}</p>
        {targets.length === 0 ? (
          <p className="banner bad small">{emptyText}</p>
        ) : (
          <>
            <label>
              {targetLabel}
              <select value={target} onChange={(e) => setTarget(e.target.value)}>
                {targets.map((t) => (
                  <option key={t.value} value={t.value}>
                    {t.label}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Folder
              <input className="mono" value={dir} onChange={(e) => setDir(e.target.value)} spellCheck={false} />
            </label>
            {chosen && (
              <p className="target small">
                → {chosen.label} : {dir}
              </p>
            )}
          </>
        )}
        <div className="modal-actions">
          <button onClick={onCancel}>Cancel</button>
          <button className="primary" disabled={!chosen || !dir.trim()} onClick={() => onSend(target, dir.trim())}>
            Send
          </button>
        </div>
      </div>
    </div>
  );
}
