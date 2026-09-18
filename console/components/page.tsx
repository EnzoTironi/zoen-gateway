import { cn } from "@/lib/utils";

/**
 * Coluna da consola — a mesma escala do Executor original
 * (`max-w-4xl`, `px-6 py-10`, títulos `font-display` 2rem) para o
 * conteúdo não saltar entre as rotas.
 */
export function PageContainer({
  className,
  children,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto" data-slot="page-container">
      <div
        className={cn("mx-auto max-w-4xl px-6 py-10 lg:px-8 lg:py-14", className)}
        {...props}
      >
        {children}
      </div>
    </div>
  );
}

/**
 * Cabeçalho de página: título 2rem + descrição + ações alinhadas.
 */
export function PageHeader({
  title,
  description,
  actions,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<"div">, "title"> & {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <div
      data-slot="page-header"
      className={cn("mb-10 flex items-start justify-between gap-4", className)}
      {...props}
    >
      <div className="min-w-0">
        <h1 className="font-display text-foreground text-[2rem] leading-none tracking-tight">
          {title}
        </h1>
        {description ? (
          <p className="text-muted-foreground mt-2 max-w-2xl text-sm leading-relaxed">
            {description}
          </p>
        ) : null}
        {children}
      </div>
      {actions ? (
        <div className="flex shrink-0 items-center gap-2">{actions}</div>
      ) : null}
    </div>
  );
}
