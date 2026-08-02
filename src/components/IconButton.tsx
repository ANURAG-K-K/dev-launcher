import type { CSSProperties, ReactNode } from "react";

export const ICON_BTN: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  padding: 6,
  cursor: "pointer",
  background: "transparent",
  border: "none",
  color: "var(--color-text)",
  borderRadius: 4,
};

export function IconButton({
  title,
  onClick,
  disabled,
  color,
  children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  color?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      disabled={disabled}
      style={{ ...ICON_BTN, color: color ?? ICON_BTN.color, opacity: disabled ? 0.4 : 1 }}
      onMouseEnter={(e) => {
        if (!disabled) e.currentTarget.style.background = "color-mix(in srgb, var(--color-text) 8%, transparent)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        {children}
      </svg>
    </button>
  );
}
