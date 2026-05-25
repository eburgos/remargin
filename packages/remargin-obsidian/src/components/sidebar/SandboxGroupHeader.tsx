import { ChevronDown } from "lucide-react";
import { useCallback } from "react";
import { ObsidianIcon } from "@/components/ui/ObsidianIcon";

/**
 * Lucide icon names we use for the Staged / Unstaged bulk actions.
 * Kept as a string union so callers get autocomplete; ObsidianIcon
 * accepts any Lucide name so extend the union as needed.
 */
export type SandboxGroupBulkIcon =
  | "square-check"
  | "arrow-up-to-line"
  | "arrow-down-to-line"
  | "chevrons-up"
  | "chevrons-down"
  // Legacy names — retained so older callers / tests still compile.
  | "check-check"
  | "minus"
  | "plus";

export interface SandboxGroupHeaderProps {
  /** Group label shown to the right of the chevron — "Staged" / "Unstaged". */
  label: string;
  /** Number of files currently in this group; shown as a small inline count. */
  count: number;
  /** Whether the group is currently expanded. */
  open: boolean;
  /** Toggle the group open/closed. */
  onToggleOpen: () => void;
  /**
   * Left bulk-action icon. Semantics by group:
   *   - Staged:   "square-check"        → select all staged rows
   *   - Unstaged: "arrow-up-to-line"    → stage the current selection
   */
  leftBulkIcon: SandboxGroupBulkIcon;
  leftBulkTitle: string;
  onLeftBulk: () => void;
  /**
   * Right bulk-action icon. Semantics by group:
   *   - Staged:   "arrow-down-to-line"  → unstage selected (or all)
   *   - Unstaged: "chevrons-up"         → stage everything unstaged
   */
  rightBulkIcon: SandboxGroupBulkIcon;
  rightBulkTitle: string;
  onRightBulk: () => void;
  /** Disable bulk actions when the group is empty. */
  disabled?: boolean;
}

/**
 * L3 header row for a Sandbox sub-group (Staged / Unstaged). Head-only —
 * the surrounding `<section class="rmg-l3">` wrapper and the L3 body are
 * owned by the parent (`PromptGroupSection`).
 *
 * Renders as a tracked-uppercase eyebrow label with a chevron + inline
 * count + two bulk-action icon buttons. Presentational only; bulk
 * semantics live in the parent.
 */
export function SandboxGroupHeader({
  label,
  count,
  open,
  onToggleOpen,
  leftBulkIcon,
  leftBulkTitle,
  onLeftBulk,
  rightBulkIcon,
  rightBulkTitle,
  onRightBulk,
  disabled,
}: SandboxGroupHeaderProps) {
  const stopAndRun = useCallback(
    (fn: () => void) => (e: React.MouseEvent) => {
      e.stopPropagation();
      fn();
    },
    []
  );

  return (
    <div
      className="rmg-l3__head"
      data-open={open ? "true" : "false"}
      role="button"
      tabIndex={0}
      onClick={onToggleOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onToggleOpen();
        }
      }}
    >
      <ChevronDown className="rmg-l3__chev" />
      <span className="rmg-l3__label">{label}</span>
      <span className="rmg-l3__count">{count}</span>
      <span /> {/* flex spacer */}
      <span className="rmg-l3__actions">
        <button
          type="button"
          className="rmg-icon-btn rmg-icon-btn--sm"
          title={leftBulkTitle}
          aria-label={leftBulkTitle}
          onClick={stopAndRun(onLeftBulk)}
          disabled={disabled}
        >
          <ObsidianIcon icon={leftBulkIcon} size={11} />
        </button>
        <button
          type="button"
          className="rmg-icon-btn rmg-icon-btn--sm"
          title={rightBulkTitle}
          aria-label={rightBulkTitle}
          onClick={stopAndRun(onRightBulk)}
          disabled={disabled}
        >
          <ObsidianIcon icon={rightBulkIcon} size={11} />
        </button>
      </span>
    </div>
  );
}
