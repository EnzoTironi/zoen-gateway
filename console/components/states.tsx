import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Skeleton } from "@/components/ui/skeleton";
import { TriangleAlertIcon } from "lucide-react";

export function LoadingBlock({ label = "Carregando…" }: { label?: string }) {
  return (
    <div
      className="divide-border/50 overflow-hidden rounded-lg border border-border/50"
      role="status"
      aria-live="polite"
    >
      <span className="sr-only">{label}</span>
      {Array.from({ length: 4 }).map((_, index) => (
        <div key={index} className="flex items-center gap-3 px-4 py-3">
          <Skeleton className="size-8 shrink-0 rounded-md" />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <Skeleton
              className="h-4"
              style={{ width: `${40 + ((index * 11) % 30)}%` }}
            />
            <Skeleton
              className="h-3"
              style={{ width: `${25 + ((index * 7) % 20)}%` }}
            />
          </div>
        </div>
      ))}
    </div>
  );
}

export function ErrorBlock({
  title = "Não foi possível carregar",
  message,
}: {
  title?: string;
  message: string;
}) {
  return (
    <Alert variant="destructive">
      <TriangleAlertIcon />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

export function EmptyBlock({
  title,
  description,
  children,
  icon,
}: {
  title: string;
  description: string;
  children?: React.ReactNode;
  icon?: React.ReactNode;
}) {
  return (
    <div className="border-border mb-8 flex flex-col items-center justify-center rounded-2xl border border-dashed py-16">
      {icon ? (
        <div className="bg-muted text-muted-foreground mb-4 flex size-12 items-center justify-center rounded-2xl">
          {icon}
        </div>
      ) : null}
      <p className="text-foreground/70 mb-1 text-[14px] font-medium">{title}</p>
      <p className="text-muted-foreground/60 mb-5 max-w-md text-center text-[13px]">
        {description}
      </p>
      {children}
    </div>
  );
}
