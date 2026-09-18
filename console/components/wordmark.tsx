import { cn } from "@/lib/utils";
import Link from "next/link";

/**
 * Wordmark do produto: `executor` em Geist Mono + tag `beta` quieta.
 * Identidade por contenção, não por pílula colorida.
 */
export function Wordmark({
  className,
  href = "/",
  onNavigate,
}: {
  className?: string;
  href?: string;
  onNavigate?: () => void;
}) {
  const mark = (
    <span className={cn("inline-flex items-baseline gap-1.5", className)}>
      <span className="text-foreground font-mono text-sm font-medium tracking-tight">
        executor
      </span>
      <span className="text-muted-foreground font-mono text-[10px] font-medium tracking-[0.12em] uppercase">
        beta
      </span>
    </span>
  );
  if (!href) {
    return mark;
  }
  return (
    <Link href={href} onClick={onNavigate} className="flex items-center">
      {mark}
    </Link>
  );
}
