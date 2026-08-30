import { ChevronDown, type LucideIcon } from "lucide-react";
import { CollapsibleTrigger } from "@/components/ui/collapsible";

interface SectionHeaderProps {
  icon: LucideIcon;
  title: string;
  badge?: number | string;
  badgeVariant?: "default" | "warning";
  open: boolean;
  actions?: React.ReactNode;
}

/**
 * Top-level section header (Sandbox, Inbox, …). Styled by the `.rmg-l1-head`
 * rules in sandbox-hierarchy.css — plain, unlayered CSS with defensive
 * resets, because Obsidian's unlayered `button` styles beat Tailwind v4's
 * `@layer utilities` regardless of specificity. The chevron is a single
 * icon that the CSS rotates on `data-open`.
 */
export function SectionHeader({
  icon: Icon,
  title,
  badge,
  badgeVariant = "default",
  open,
  actions,
}: SectionHeaderProps) {
  const badgeClass =
    badgeVariant === "warning"
      ? "rmg-l1-head__badge rmg-l1-head__badge--warning"
      : "rmg-l1-head__badge";

  // The trigger (a native <button>) wraps only the non-interactive
  // header content; {actions} render as a sibling so ViewToggle's
  // buttons never nest inside the trigger button.
  return (
    <div className="rmg-l1-head" data-open={open ? "true" : "false"}>
      <CollapsibleTrigger className="rmg-l1-head__trigger">
        <ChevronDown className="rmg-l1-head__chev" />
        <Icon className="rmg-l1-head__icon" />
        <span className="rmg-l1-head__title">{title}</span>
        {badge != null && <span className={badgeClass}>{badge}</span>}
      </CollapsibleTrigger>
      {actions && <span className="rmg-l1-head__actions">{actions}</span>}
    </div>
  );
}
