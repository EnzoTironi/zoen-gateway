import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { TriangleAlertIcon } from "lucide-react";

export function LoadingBlock({ label = "Carregando…" }: { label?: string }) {
  return (
    <div className="flex flex-col gap-3" role="status" aria-live="polite">
      <span className="sr-only">{label}</span>
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-24 w-full" />
      <Skeleton className="h-24 w-full" />
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
}: {
  title: string;
  description: string;
  children?: React.ReactNode;
}) {
  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      {children}
    </Empty>
  );
}
