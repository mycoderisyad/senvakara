import { ReactNode } from "react";

interface Props {
  title: string;
  children: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/** In-app confirmation for destructive or overwriting actions. */
export function ConfirmDialog({ title, children, confirmLabel, danger, onConfirm, onCancel }: Props) {
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="modal" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title">
        <h2 id="confirm-title">{title}</h2>
        <div>{children}</div>
        <div className="modal-actions">
          <button onClick={onCancel} autoFocus>
            Cancel
          </button>
          <button className={danger ? "danger" : "primary"} onClick={onConfirm}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
